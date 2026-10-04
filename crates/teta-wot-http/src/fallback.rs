//! The fallback server: when the
//! server can't start, an HTTP error page is served instead, so that a
//! headless instrument still explains itself at its usual address.

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::middleware::from_fn;
use axum::response::Response;
use http::header::{CONTENT_TYPE, LOCATION};
use http::{HeaderValue, StatusCode};

use crate::cors::cors;

/// What the fallback page shows.
#[derive(Debug, Clone, Default)]
pub struct FallbackPage {
    /// Why the server couldn't start.
    pub error_message: String,
    /// The Things that were built, if any.
    pub things: Vec<String>,
    /// The configuration, as text.
    pub config: String,
    /// The error and its causes, one per line.
    pub traceback: String,
    /// Log output, if any was captured.
    pub logging: Option<String>,
}

impl FallbackPage {
    /// The page's HTML.
    pub fn html(&self) -> String {
        let mut html = String::from(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\" />\n\
             <title>teta-wot: the server failed to start</title>\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n\
             <style>\nbody { font-family: system-ui, sans-serif; margin: 2rem; line-height: 1.5; color: #1f2933; }\n\
             main { max-width: 900px; margin: 0 auto; }\n\
             .error { padding: 1rem; background: #fee2e2; border: 1px solid #fecaca; border-radius: 6px; color: #7f1d1d; }\n\
             pre { background: #f1f5f9; padding: 1rem; border-radius: 6px; white-space: pre-wrap; word-break: break-word; }\n\
             .muted { color: #6b7280; }\n</style>\n</head>\n<body>\n<main>\n\
             <h1>The server couldn't start</h1>\n\
             <p class=\"muted\">The server failed during startup. This is usually caused by an invalid \
             configuration, a missing Thing type, or an error while a Thing started.</p>\n",
        );
        if !self.error_message.is_empty() {
            html.push_str(&format!(
                "<div class=\"error\">{}</div>\n",
                escape(&self.error_message)
            ));
        }
        if !self.things.is_empty() {
            html.push_str("<h2>The following Things loaded successfully:</h2>\n<ul>\n");
            for thing in &self.things {
                html.push_str(&format!("<li>{}</li>\n", escape(thing)));
            }
            html.push_str("</ul>\n");
        }
        html.push_str(&format!(
            "<h2>Configuration</h2>\n<pre>{}</pre>\n<h2>Error</h2>\n<pre>{}</pre>\n<h2>Logging</h2>\n",
            escape(&self.config),
            escape(&self.traceback)
        ));
        match &self.logging {
            Some(logging) if !logging.is_empty() => {
                html.push_str(&format!("<pre>{}</pre>\n", escape(logging)));
            }
            _ => html.push_str("<p class=\"muted\">No logging information available.</p>\n"),
        }
        html.push_str(
            "</main>\n<footer class=\"muted\">teta-wot fallback server</footer>\n</body>\n</html>\n",
        );
        html
    }
}

/// The fallback server's router.
pub fn fallback_router(page: FallbackPage) -> Router {
    let html = page.html();
    Router::new()
        .fallback(move |request: Request| {
            let html = html.clone();
            async move { answer(&html, request.uri().path()) }
        })
        .layer(from_fn(cors))
}

fn answer(html: &str, path: &str) -> Response {
    let mut response = match path {
        "/" => {
            let mut response = Response::new(Body::from(html.to_owned()));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
        }
        "/fallback" => {
            let mut response = Response::new(Body::from("true"));
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response
        }
        _ => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
            response
                .headers_mut()
                .insert(LOCATION, HeaderValue::from_static("/"));
            response
        }
    };
    response.headers_mut().remove(http::header::CONTENT_LENGTH);
    response
}

/// Escapes text for HTML.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn text_is_escaped() {
        assert_eq!(
            escape("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert(&#x27;x&#x27;) &amp; &quot;y&quot;&lt;/script&gt;"
        );
    }
}
