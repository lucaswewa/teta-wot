//! Discovery: `/.well-known/wot` and a read-only
//! Thing Description Directory, as W3C WoT Discovery describes them.
//!
//! - `GET /.well-known/wot` is the directory's own TD (a `ThingDirectory`),
//!   at the server's root whatever the API prefix.
//! - `GET {prefix}/directory/things` lists the served TDs, sorted by `id`,
//!   as `application/ld+json`: an array, or with `format=collection` a
//!   `ThingCollection`. `offset` and `limit` page it, with `next` and
//!   `canonical` links.
//! - `GET {prefix}/directory/things/{id}` is one TD, as
//!   `application/td+json`.
//!
//! Every route also answers `HEAD`; other methods get 405. Errors are
//! problem details whatever the wire profile, as the Discovery API wants.
//! The directory can't be written to: its Things are the server's.

use std::collections::HashMap;

use axum::extract::Query;
use axum::response::Response;
use http::header::{CONTENT_TYPE, LINK};
use http::{HeaderValue, StatusCode, Uri};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::problem::{http_problem, problem_response, problem_type};
use crate::render::{self, Urls};
use crate::{HttpOptions, td_id};

/// The TD 1.1 context and the Discovery context.
const DISCOVERY_CONTEXT: &str = "https://www.w3.org/2022/wot/discovery";

/// The directory's path under the API prefix.
pub(crate) fn directory_path(urls: &Urls) -> String {
    format!("{}/directory/", urls.prefix)
}

/// The directory's TD: the Discovery specification's Thing Model for a
/// directory, with only its read operations (`things` and
/// `retrieveThing`).
pub(crate) fn directory_td(options: &HttpOptions, urls: &Urls) -> Value {
    let invalid_query = json!({
        "description": "Invalid query arguments",
        "contentType": "application/problem+json",
        "htv:statusCodeValue": 400
    });
    json!({
        "@context": ["https://www.w3.org/2022/wot/td/v1.1", DISCOVERY_CONTEXT],
        "@type": "ThingDirectory",
        "id": td_id(&options.server_id, "directory"),
        "title": "Thing Description Directory",
        "description": format!(
            "The Things served by {}: a read-only Thing Description Directory.",
            options.api_title
        ),
        "base": urls.absolute(&directory_path(urls)),
        "securityDefinitions": {"nosec_sc": {"scheme": "nosec"}},
        "security": "nosec_sc",
        "properties": {
            "things": {
                "description": "Retrieve all Thing Descriptions",
                "readOnly": true,
                "uriVariables": {
                    "offset": {
                        "title": "Number of TDs to skip before the page",
                        "type": "number",
                        "default": 0
                    },
                    "limit": {"title": "Number of TDs in a page", "type": "number"},
                    "format": {
                        "title": "Payload format",
                        "type": "string",
                        "enum": ["array", "collection"],
                        "default": "array"
                    }
                },
                "forms": [{
                    "href": "things{?offset,limit,format}",
                    "op": "readproperty",
                    "htv:methodName": "GET",
                    "response": {
                        "description": "Success response",
                        "htv:statusCodeValue": 200,
                        "contentType": "application/ld+json",
                        "htv:headers": [{"htv:fieldName": "Link"}]
                    },
                    "additionalResponses": [invalid_query]
                }]
            }
        },
        "actions": {
            "retrieveThing": {
                "description": "Retrieve a Thing Description",
                "uriVariables": {
                    "id": {
                        "@type": "ThingID",
                        "title": "Thing Description ID",
                        "type": "string",
                        "format": "iri-reference"
                    }
                },
                "output": {
                    "description": "The schema is implied by the content type",
                    "type": "object"
                },
                "safe": true,
                "idempotent": true,
                "synchronous": true,
                "forms": [{
                    "href": "things/{id}",
                    "op": "invokeaction",
                    "htv:methodName": "GET",
                    "response": {
                        "description": "Success response",
                        "htv:statusCodeValue": 200,
                        "contentType": "application/td+json"
                    },
                    "additionalResponses": [{
                        "description": "TD with the given id not found",
                        "contentType": "application/problem+json",
                        "htv:statusCodeValue": 404
                    }]
                }]
            }
        }
    })
}

/// A response with a JSON body of the given media type.
pub(crate) fn json_as(media_type: &'static str, value: &Value) -> Response {
    let mut response = render::json(StatusCode::OK, value);
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(media_type));
    response
}

/// The TDs, sorted by `id` in code point order, as the listing wants.
fn sorted(mut tds: Vec<Value>) -> Vec<Value> {
    tds.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    tds
}

/// `GET {prefix}/directory/things`.
#[allow(clippy::result_large_err)] // the errors are the responses to send
pub(crate) fn listing(tds: Vec<Value>, uri: &Uri, urls: &Urls) -> Response {
    let query = match Query::<HashMap<String, String>>::try_from_uri(uri) {
        Ok(Query(query)) => query,
        Err(error) => return bad_query("query", &error.body_text()),
    };
    let number = |name: &str| -> Result<Option<usize>, Response> {
        match query.get(name) {
            None => Ok(None),
            Some(raw) => raw
                .parse::<usize>()
                .map(Some)
                .map_err(|_| bad_query(name, "must be a whole number, 0 or more")),
        }
    };
    let offset = match number("offset") {
        Ok(offset) => offset,
        Err(response) => return response,
    };
    let limit = match number("limit") {
        Ok(Some(0)) => return bad_query("limit", "must be a positive whole number"),
        Ok(limit) => limit,
        Err(response) => return response,
    };
    let collection = match query.get("format").map(String::as_str) {
        None | Some("array") => false,
        Some("collection") => true,
        Some(_) => return bad_query("format", "must be `array` or `collection`"),
    };

    let tds = sorted(tds);
    let total = tds.len();
    let start = offset.unwrap_or(0).min(total);
    let end = limit.map_or(total, |limit| start.saturating_add(limit).min(total));
    let page: Vec<Value> = tds[start..end].to_vec();

    let path = format!("{}things", directory_path(urls));
    let page_url = |offset: usize| {
        let mut url = format!("{path}?offset={offset}");
        if let Some(limit) = limit {
            url.push_str(&format!("&limit={limit}"));
        }
        if collection {
            url.push_str("&format=collection");
        }
        urls.absolute(&url)
    };
    let next = (end < total).then(|| page_url(end));
    let body = if collection {
        let mut body = json!({
            "@context": DISCOVERY_CONTEXT,
            "@type": "ThingCollection",
            "total": total,
            "members": page,
            "@id": page_url(start),
        });
        if let Some(next) = &next {
            body["next"] = json!(next);
        }
        body
    } else {
        Value::Array(page)
    };
    let mut response = json_as("application/ld+json", &body);
    let paged = offset.is_some() || limit.is_some();
    if let Some(next) = &next {
        append_link(&mut response, &format!("<{next}>; rel=\"next\""));
    }
    if paged {
        let etag = etag(&tds);
        append_link(
            &mut response,
            &format!(
                "<{}>; rel=\"canonical\"; etag=\"{etag}\"",
                urls.absolute(&path)
            ),
        );
    }
    response
}

/// `GET {prefix}/directory/things/{id}`.
pub(crate) fn retrieve(tds: Vec<Value>, raw_id: &str) -> Response {
    let id = percent_decode(raw_id);
    match tds.into_iter().find(|td| td["id"].as_str() == Some(&id)) {
        Some(td) => json_as("application/td+json", &td),
        None => http_problem(
            StatusCode::NOT_FOUND,
            &format!("No Thing Description has the ID {id}"),
        ),
    }
}

/// The collection's state, for the `canonical` link: it changes when the
/// TDs' IDs do.
fn etag(tds: &[Value]) -> String {
    let ids: Vec<&str> = tds.iter().filter_map(|td| td["id"].as_str()).collect();
    Uuid::new_v5(&Uuid::NAMESPACE_URL, ids.join("\n").as_bytes())
        .simple()
        .to_string()
}

fn append_link(response: &mut Response, link: &str) {
    if let Ok(value) = HeaderValue::from_str(link) {
        response.headers_mut().append(LINK, value);
    }
}

/// 400 for a query argument the listing can't use.
fn bad_query(name: &str, reason: &str) -> Response {
    problem_response(
        StatusCode::BAD_REQUEST,
        json!({
            "type": problem_type("validation-error"),
            "title": "Validation error",
            "status": 400,
            "detail": format!("Invalid query argument `{name}`: {reason}"),
            "invalid-params": [{"in": "query", "name": name, "reason": reason}],
        }),
    )
}

/// Decodes `%XX` escapes (the ID in the path may be escaped).
pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = raw
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tds() -> Vec<Value> {
        ["urn:c", "urn:a", "urn:b"]
            .iter()
            .map(|id| json!({"id": id}))
            .collect()
    }

    fn urls() -> Urls {
        Urls {
            origin: "http://host".into(),
            prefix: "/api".into(),
        }
    }

    async fn body(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn links(response: &Response) -> Vec<&str> {
        response
            .headers()
            .get_all(LINK)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn the_listing_is_sorted_and_paged() {
        let all = listing(tds(), &"/api/directory/things".parse().unwrap(), &urls());
        assert_eq!(all.headers()[CONTENT_TYPE], "application/ld+json");
        assert!(links(&all).is_empty(), "not paged");
        assert_eq!(
            body(all).await,
            json!([{"id": "urn:a"}, {"id": "urn:b"}, {"id": "urn:c"}])
        );

        let first = listing(
            tds(),
            &"/api/directory/things?limit=2".parse().unwrap(),
            &urls(),
        );
        let links_first: Vec<String> = links(&first).iter().map(|l| l.to_string()).collect();
        assert_eq!(
            links_first[0],
            "<http://host/api/directory/things?offset=2&limit=2>; rel=\"next\""
        );
        assert!(
            links_first[1]
                .starts_with("<http://host/api/directory/things>; rel=\"canonical\"; etag=\"")
        );
        assert_eq!(body(first).await, json!([{"id": "urn:a"}, {"id": "urn:b"}]));

        let last = listing(
            tds(),
            &"/api/directory/things?offset=2&limit=2".parse().unwrap(),
            &urls(),
        );
        assert_eq!(links(&last).len(), 1, "no next link on the last page");
        assert_eq!(body(last).await, json!([{"id": "urn:c"}]));
    }

    #[tokio::test]
    async fn the_collection_format() {
        let response = listing(
            tds(),
            &"/api/directory/things?limit=1&format=collection"
                .parse()
                .unwrap(),
            &urls(),
        );
        let body = body(response).await;
        assert_eq!(body["@type"], "ThingCollection");
        assert_eq!(body["total"], 3);
        assert_eq!(body["members"], json!([{"id": "urn:a"}]));
        assert_eq!(
            body["next"],
            "http://host/api/directory/things?offset=1&limit=1&format=collection"
        );
    }

    #[tokio::test]
    async fn bad_arguments_are_400() {
        for query in ["limit=0", "limit=x", "offset=-1", "format=xml"] {
            let response = listing(
                tds(),
                &format!("/api/directory/things?{query}").parse().unwrap(),
                &urls(),
            );
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
            assert_eq!(response.headers()[CONTENT_TYPE], "application/problem+json");
        }
    }

    #[tokio::test]
    async fn retrieval_decodes_the_id() {
        let found = retrieve(tds(), "urn%3Ab");
        assert_eq!(found.headers()[CONTENT_TYPE], "application/td+json");
        assert_eq!(body(found).await, json!({"id": "urn:b"}));
        let missing = retrieve(tds(), "urn:x");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()[CONTENT_TYPE], "application/problem+json");
    }

    #[test]
    fn the_directory_td_is_valid() {
        let td = directory_td(&HttpOptions::default(), &urls());
        teta_wot_td::validation::validate_json(&td).unwrap();
        assert_eq!(td["base"], "http://host/api/directory/");
    }
}
