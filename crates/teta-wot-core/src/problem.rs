//! RFC 9457 problem details.

use serde::{Deserialize, Serialize};

/// Where teta-wot documents its exceptions. Problem `type` URIs for
/// teta-wot exceptions point here.
pub const TETA_WOT_EXCEPTIONS_URL: &str = "https://www.tetaprecision.com/api/exceptions/index.html";

/// The `type` URI teta-wot uses for one of its exceptions, such as
/// `GlobalLockBusyError`.
pub fn teta_wot_exception_type(exception: &str) -> String {
    format!("{TETA_WOT_EXCEPTIONS_URL}#teta_wot.exceptions.{exception}")
}

/// A problem details object: `{detail, type, status, title, instance}`.
///
/// Every field is written, `null` when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemDetails {
    /// What went wrong, for humans.
    pub detail: Option<String>,
    /// A URI identifying the kind of problem.
    #[serde(rename = "type")]
    pub problem_type: Option<String>,
    /// The HTTP status code that goes with it.
    pub status: Option<u16>,
    /// A short name for the kind of problem.
    pub title: Option<String>,
    /// A URI for this occurrence.
    pub instance: Option<String>,
}

impl ProblemDetails {
    /// A problem named after a teta-wot exception, with its `type` URI.
    pub fn teta_wot_things(exception: &str, detail: impl Into<String>, status: u16) -> Self {
        Self {
            detail: Some(detail.into()),
            problem_type: Some(teta_wot_exception_type(exception)),
            status: Some(status),
            title: Some(exception.to_owned()),
            instance: None,
        }
    }

    /// A problem with no `type` URI.
    pub fn untyped(title: impl Into<String>, detail: impl Into<String>, status: u16) -> Self {
        Self {
            detail: Some(detail.into()),
            problem_type: None,
            status: Some(status),
            title: Some(title.into()),
            instance: None,
        }
    }
}
