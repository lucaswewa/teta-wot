//! Error responses in each wire profile.
//!
//! In the `tetathing` profile, errors are `{"detail": …}`, the
//! 422 validation list, or TetaThing's problem details, all as
//! `application/json`. In the `wot` profile, and on the discovery routes,
//! every error is an RFC 9457 problem, as `application/problem+json`:
//!
//! - errors that are only HTTP's (such as an unknown path) have the type
//!   `about:blank` and the status's reason phrase as their title;
//! - the framework's own errors have a `type` under [`PROBLEM_TYPES_URL`],
//!   documented in `docs/problems.md`;
//! - validation errors are 400, with one `invalid-params` entry per issue.

use axum::response::Response;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use serde_json::{Map, Value, json};
use teta_wot_core::problem::{TETA_WOT_EXCEPTIONS_URL, teta_wot_exception_type};
use teta_wot_core::{LocItem, ProblemDetails, ValidationIssue};

use crate::WireProfile;
use crate::render::{self, detail, unprocessable};

/// Where the `wot` profile's problem types are documented; each type is
/// this URL, `#`, and the type's name, such as `validation-error`.
pub const PROBLEM_TYPES_URL: &str = "https://github.com/lucaswewa/wot/blob/main/docs/problems.md";

/// The media type of problem details (RFC 9457).
pub(crate) const PROBLEM_JSON: &str = "application/problem+json";

/// The `type` URI of one of the framework's problem types.
pub fn problem_type(name: &str) -> String {
    format!("{PROBLEM_TYPES_URL}#{name}")
}

/// What went wrong, before it is written in a profile's format.
#[derive(Debug)]
pub(crate) enum HttpError<'a> {
    /// An error with only an HTTP meaning.
    Status(StatusCode, String),
    /// Invalid input.
    Invalid(&'a [ValidationIssue]),
    /// A problem reported by the runtime.
    Problem(ProblemDetails),
    /// One of the framework's problem types, with status and
    /// `detail` in the `tetathing` profile.
    Typed {
        /// The status in the `tetathing` profile.
        tetathing: StatusCode,
        /// The status in the `wot` profile.
        wot: StatusCode,
        /// The type's name, under [`PROBLEM_TYPES_URL`].
        name: &'static str,
        title: &'static str,
        detail: String,
    },
}

impl HttpError<'_> {
    pub(crate) fn not_found() -> Self {
        HttpError::Status(StatusCode::NOT_FOUND, "Not Found".to_owned())
    }

    /// The response, in the profile's format.
    pub(crate) fn response(self, profile: WireProfile) -> Response {
        match profile {
            WireProfile::TetaThing => self.tetathing(),
            WireProfile::Wot => self.wot(),
        }
    }

    fn tetathing(self) -> Response {
        match self {
            HttpError::Status(status, message) => detail(status, &message),
            HttpError::Invalid(issues) => unprocessable(issues),
            HttpError::Problem(problem) => render::problem(&problem),
            HttpError::Typed {
                tetathing, detail, ..
            } => render::detail(tetathing, &detail),
        }
    }

    fn wot(self) -> Response {
        match self {
            HttpError::Status(status, message) => http_problem(status, &message),
            HttpError::Invalid(issues) => validation_problem(issues),
            HttpError::Problem(problem) => {
                let (status, body) = wot_problem(&problem);
                problem_response(status, body)
            }
            HttpError::Typed {
                wot,
                name,
                title,
                detail,
                ..
            } => problem_response(
                wot,
                json!({
                    "type": problem_type(name),
                    "title": title,
                    "status": wot.as_u16(),
                    "detail": detail,
                }),
            ),
        }
    }
}

/// A problem response: `application/problem+json`.
pub(crate) fn problem_response(status: StatusCode, body: Value) -> Response {
    let mut response = render::json(status, &body);
    response
        .headers_mut()
        .insert(CONTENT_TYPE, http::HeaderValue::from_static(PROBLEM_JSON));
    response
}

/// An error with only an HTTP meaning: type `about:blank`, titled with the
/// status's reason phrase.
pub(crate) fn http_problem(status: StatusCode, message: &str) -> Response {
    problem_response(
        status,
        json!({
            "type": "about:blank",
            "title": status.canonical_reason().unwrap_or("Error"),
            "status": status.as_u16(),
            "detail": message,
        }),
    )
}

/// 400, with one `invalid-params` entry per issue: `name` is a JSON
/// Pointer into the body (or the path or query parameter's name), and
/// `reason` is pydantic's message.
fn validation_problem(issues: &[ValidationIssue]) -> Response {
    let params: Vec<Value> = issues.iter().map(invalid_param).collect();
    let count = issues.len();
    problem_response(
        StatusCode::BAD_REQUEST,
        json!({
            "type": problem_type("validation-error"),
            "title": "Validation error",
            "status": 400,
            "detail": format!("{count} validation error{}", if count == 1 { "" } else { "s" }),
            "invalid-params": params,
        }),
    )
}

fn invalid_param(issue: &ValidationIssue) -> Value {
    let (location, rest) = issue
        .loc
        .split_first()
        .map_or(("body", &[][..]), |(first, rest)| match first {
            LocItem::Key(key) if matches!(key.as_str(), "body" | "path" | "query") => {
                (key.as_str(), rest)
            }
            _ => ("body", &issue.loc[..]),
        });
    let name = if location == "body" {
        rest.iter()
            .map(|item| match item {
                LocItem::Key(key) => format!("/{}", key.replace('~', "~0").replace('/', "~1")),
                LocItem::Index(index) => format!("/{index}"),
            })
            .collect::<String>()
    } else {
        rest.iter()
            .map(|item| match item {
                LocItem::Key(key) => key.clone(),
                LocItem::Index(index) => index.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".")
    };
    json!({"in": location, "name": name, "reason": issue.msg, "code": issue.kind})
}

/// A runtime problem in the `wot` profile: the status and body. TetaThing's
/// exception types become the framework's, and `null` members are left out.
pub(crate) fn wot_problem(problem: &ProblemDetails) -> (StatusCode, Value) {
    let tetathing = |exception: &str| {
        problem.problem_type.as_deref() == Some(&teta_wot_exception_type(exception))
    };
    let (name, title, status) = if tetathing("GlobalLockBusyError") {
        (
            "global-lock-busy",
            "Global lock busy",
            StatusCode::SERVICE_UNAVAILABLE,
        )
    } else if tetathing("InvocationCancelledError") {
        ("cancelled", "Cancelled", StatusCode::SERVICE_UNAVAILABLE)
    } else if tetathing("InvocationError")
        || problem
            .problem_type
            .as_deref()
            .is_none_or(|t| t.starts_with(TETA_WOT_EXCEPTIONS_URL))
    {
        ("failed", "Failed", StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        // A type the Thing chose: kept, with its own title and status.
        let status = problem
            .status
            .and_then(|s| StatusCode::from_u16(s).ok())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut body = Map::new();
        body.insert("type".into(), json!(problem.problem_type));
        if let Some(title) = &problem.title {
            body.insert("title".into(), json!(title));
        }
        body.insert("status".into(), json!(status.as_u16()));
        if let Some(detail) = &problem.detail {
            body.insert("detail".into(), json!(detail));
        }
        if let Some(instance) = &problem.instance {
            body.insert("instance".into(), json!(instance));
        }
        return (status, Value::Object(body));
    };
    let mut body = Map::new();
    body.insert("type".into(), json!(problem_type(name)));
    body.insert("title".into(), json!(title));
    body.insert("status".into(), json!(status.as_u16()));
    if let Some(detail) = &problem.detail {
        body.insert("detail".into(), json!(detail));
    }
    if let Some(error) = problem.title.as_ref().filter(|t| *t != title) {
        body.insert("error".into(), json!(error));
    }
    if let Some(instance) = &problem.instance {
        body.insert("instance".into(), json!(instance));
    }
    (status, Value::Object(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tetathing_problems_get_the_frameworks_types() {
        let busy = ProblemDetails::teta_wot_things("GlobalLockBusyError", "busy", 409);
        let (status, body) = wot_problem(&busy);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            json!({
                "type": problem_type("global-lock-busy"),
                "title": "Global lock busy",
                "status": 503,
                "detail": "busy",
                "error": "GlobalLockBusyError",
            })
        );
        let (status, body) = wot_problem(&ProblemDetails::untyped("DeviceError", "gone", 500));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["type"], problem_type("failed"));
        assert_eq!(body["error"], "DeviceError");
        assert!(body.get("instance").is_none(), "no null members");
    }

    #[test]
    fn invalid_params_point_into_the_body() {
        let issue = |loc: Vec<LocItem>| ValidationIssue {
            kind: "int_parsing".into(),
            loc,
            msg: "Input should be a valid integer".into(),
            input: json!("x"),
            ctx: None,
        };
        assert_eq!(
            invalid_param(&issue(vec!["body".into(), "a/b".into(), LocItem::Index(2)])),
            json!({"in": "body", "name": "/a~1b/2", "reason": "Input should be a valid integer", "code": "int_parsing"})
        );
        assert_eq!(invalid_param(&issue(vec!["body".into()]))["name"], "");
        assert_eq!(
            invalid_param(&issue(vec!["query".into(), "limit".into()])),
            json!({"in": "query", "name": "limit", "reason": "Input should be a valid integer", "code": "int_parsing"})
        );
    }
}
