//! Interactive API documentation: `/openapi.json`, Swagger UI
//! at `/docs` (with `/docs/oauth2-redirect`), and ReDoc at `/redoc`.
//!
//! The pages load Swagger UI and ReDoc from jsDelivr, pinned to the
//! versions in `assets/`. With the `docs-offline` feature, those assets
//! are compiled in and served from `/docs/…` and `/redoc/…`, so the pages
//! work without network access.

use axum::body::Body;
use axum::response::Response;
use http::HeaderValue;
use http::header::{CACHE_CONTROL, CONTENT_TYPE};

/// The Swagger UI release the pages use.
pub const SWAGGER_UI_VERSION: &str = "5.33.0";
/// The ReDoc release the pages use.
pub const REDOC_VERSION: &str = "2.5.4";

/// Whether the assets are compiled in (the `docs-offline` feature).
pub const OFFLINE: bool = cfg!(feature = "docs-offline");

/// A compiled-in asset: its path, content type and data.
#[cfg(feature = "docs-offline")]
pub(crate) const ASSETS: [(&str, &str, &[u8]); 4] = [
    (
        "/docs/swagger-ui-bundle.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../assets/swagger-ui-5.33.0/swagger-ui-bundle.js"),
    ),
    (
        "/docs/swagger-ui.css",
        "text/css; charset=utf-8",
        include_bytes!("../assets/swagger-ui-5.33.0/swagger-ui.css"),
    ),
    (
        "/docs/favicon-32x32.png",
        "image/png",
        include_bytes!("../assets/swagger-ui-5.33.0/favicon-32x32.png"),
    ),
    (
        "/redoc/redoc.standalone.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../assets/redoc-2.5.4/redoc.standalone.js"),
    ),
];

/// No assets without the `docs-offline` feature.
#[cfg(not(feature = "docs-offline"))]
pub(crate) const ASSETS: [(&str, &str, &[u8]); 0] = [];

/// Where a page loads an asset from: the server, or jsDelivr.
fn asset_url(offline_path: &str, package: &str, file: &str) -> String {
    if OFFLINE {
        offline_path.to_owned()
    } else {
        format!("https://cdn.jsdelivr.net/npm/{package}/{file}")
    }
}

fn swagger(file: &str) -> String {
    asset_url(
        &format!("/docs/{file}"),
        &format!("swagger-ui-dist@{SWAGGER_UI_VERSION}"),
        file,
    )
}

fn html(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// Escapes text for HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Swagger UI.
pub(crate) fn swagger_ui(title: &str) -> Response {
    let title = escape(title);
    let (css, js, icon) = (
        swagger("swagger-ui.css"),
        swagger("swagger-ui-bundle.js"),
        swagger("favicon-32x32.png"),
    );
    html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link type="text/css" rel="stylesheet" href="{css}">
<link rel="shortcut icon" href="{icon}">
<title>{title} - Swagger UI</title>
</head>
<body>
<div id="swagger-ui">
</div>
<script src="{js}"></script>
<script>
const ui = SwaggerUIBundle({{
    url: '/openapi.json',
    dom_id: '#swagger-ui',
    layout: 'BaseLayout',
    deepLinking: true,
    showExtensions: true,
    showCommonExtensions: true,
    oauth2RedirectUrl: window.location.origin + '/docs/oauth2-redirect',
    presets: [
        SwaggerUIBundle.presets.apis,
        SwaggerUIBundle.SwaggerUIStandalonePreset
    ],
}})
</script>
</body>
</html>
"#
    ))
}

/// Swagger UI's OAuth2 redirect page, with its script inline.
pub(crate) fn oauth2_redirect() -> Response {
    let script = include_str!("../assets/swagger-ui-5.33.0/oauth2-redirect.js");
    html(format!(
        "<!doctype html>\n<html lang=\"en-US\">\n<head>\n<title>Swagger UI: OAuth2 Redirect</title>\n</head>\n<body>\n<script>\n{script}\n</script>\n</body>\n</html>\n"
    ))
}

/// ReDoc.
pub(crate) fn redoc(title: &str) -> Response {
    let title = escape(title);
    let js = asset_url(
        "/redoc/redoc.standalone.js",
        &format!("redoc@{REDOC_VERSION}"),
        "bundles/redoc.standalone.js",
    );
    let icon = swagger("favicon-32x32.png");
    html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<title>{title} - ReDoc</title>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="shortcut icon" href="{icon}">
<style>
  body {{
    margin: 0;
    padding: 0;
  }}
</style>
</head>
<body>
<noscript>
    ReDoc requires Javascript to function. Please enable it to browse the documentation.
</noscript>
<redoc spec-url="/openapi.json"></redoc>
<script src="{js}"> </script>
</body>
</html>
"#
    ))
}

/// A compiled-in asset.
pub(crate) fn asset(index: usize) -> Response {
    let (_, content_type, data) = ASSETS[index];
    let mut response = Response::new(Body::from(data));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_escaped() {
        assert_eq!(escape("<A & B>"), "&lt;A &amp; B&gt;");
    }

    #[test]
    fn assets_come_from_the_pinned_cdn_or_the_server() {
        let url = swagger("swagger-ui-bundle.js");
        if OFFLINE {
            assert_eq!(url, "/docs/swagger-ui-bundle.js");
        } else {
            assert_eq!(
                url,
                "https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.33.0/swagger-ui-bundle.js"
            );
        }
    }
}
