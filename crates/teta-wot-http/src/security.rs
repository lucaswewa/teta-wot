//! Optional credentials: HTTP Basic or a bearer token,
//! required by every interaction and described in the TDs. Descriptions
//! (TDs, the directory, the OpenAPI document and the docs) stay public.

use axum::response::Response;
use http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use http::{HeaderMap, HeaderValue, StatusCode};
use teta_wot_td::{CredentialLocation, SecurityScheme};

use crate::problem::HttpError;
use crate::{Security, WireProfile};

/// The realm named in `WWW-Authenticate`.
const REALM: &str = "wot-rs";

/// The TD's security definition for `security`: its name and scheme.
pub(crate) fn scheme(security: &Security) -> (String, SecurityScheme) {
    match security {
        Security::Basic { .. } => {
            let mut scheme =
                SecurityScheme::basic().with_description("HTTP Basic authentication (RFC 7617)");
            // The WoT Profile wants both, with these values.
            scheme.location = Some(CredentialLocation::Header);
            scheme.name = Some("Authorization".to_owned());
            ("basic_sc".to_owned(), scheme)
        }
        Security::Bearer { .. } => {
            let mut scheme = SecurityScheme::bearer().with_description("A bearer token (RFC 6750)");
            scheme.location = Some(CredentialLocation::Header);
            scheme.name = Some("Authorization".to_owned());
            // Not a JWT, TD 1.1's default: the server compares the token.
            scheme.format = Some("opaque".to_owned());
            ("bearer_sc".to_owned(), scheme)
        }
    }
}

/// Whether the request carries the credentials.
pub(crate) fn authorized(security: &Security, headers: &HeaderMap) -> bool {
    let (scheme, expected) = match security {
        Security::Basic { username, password } => {
            ("basic", base64(format!("{username}:{password}").as_bytes()))
        }
        Security::Bearer { token } => ("bearer", token.clone()),
    };
    headers.get_all(AUTHORIZATION).iter().any(|value| {
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some((given_scheme, credentials)) = value.trim().split_once(' ') else {
            return false;
        };
        given_scheme.eq_ignore_ascii_case(scheme)
            && constant_time_eq(credentials.trim().as_bytes(), expected.as_bytes())
    })
}

/// 401, with the scheme in `WWW-Authenticate`. The body is FastAPI's for
/// its security helpers in the `tetathing` profile.
pub(crate) fn unauthorized(security: &Security, profile: WireProfile) -> Response {
    let mut response = HttpError::Status(StatusCode::UNAUTHORIZED, "Not authenticated".to_owned())
        .response(profile);
    let challenge = match security {
        Security::Basic { .. } => format!("Basic realm=\"{REALM}\", charset=\"UTF-8\""),
        Security::Bearer { .. } => format!("Bearer realm=\"{REALM}\""),
    };
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    }
    response
}

/// Compares without stopping at the first difference, so that the time
/// taken doesn't tell how much of a guess was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Standard base64, with padding (RFC 4648).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648() {
        for (input, output) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), output);
        }
    }

    #[test]
    fn credentials_are_checked() {
        let headers = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, value.parse().unwrap());
            headers
        };
        let basic = Security::basic("Aladdin", "open sesame");
        assert!(authorized(
            &basic,
            &headers("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==")
        ));
        assert!(authorized(
            &basic,
            &headers("basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==")
        ));
        assert!(!authorized(
            &basic,
            &headers("Basic QWxhZGRpbjpvcGVuIHNlc2FtZR==")
        ));
        assert!(!authorized(&basic, &HeaderMap::new()));
        let bearer = Security::bearer("s3cret");
        assert!(authorized(&bearer, &headers("Bearer s3cret")));
        assert!(!authorized(&bearer, &headers("Bearer s3cre")));
        assert!(!authorized(&bearer, &headers("Basic s3cret")));
    }

    #[test]
    fn a_401_challenges() {
        let response = unauthorized(&Security::bearer("x"), WireProfile::Wot);
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[WWW_AUTHENTICATE],
            "Bearer realm=\"wot-rs\""
        );
    }
}
