//! Security schemes.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::OneOrMany;
use crate::data_schema::MultiLanguage;

/// The `scheme` of a [`SecurityScheme`] (TD 1.1 §5.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySchemeType {
    /// No security.
    Nosec,
    /// The consumer negotiates the scheme with the protocol.
    Auto,
    /// A combination of other schemes (`oneOf` or `allOf`).
    Combo,
    /// HTTP Basic authentication.
    Basic,
    /// HTTP Digest authentication.
    Digest,
    /// An API key.
    Apikey,
    /// A bearer token.
    Bearer,
    /// A pre-shared key.
    Psk,
    /// OAuth 2.0.
    Oauth2,
}

/// Where a credential is sent (`in`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialLocation {
    /// An HTTP header.
    Header,
    /// A query parameter.
    Query,
    /// The request body.
    Body,
    /// A cookie.
    Cookie,
    /// A URI template variable.
    Uri,
    /// Chosen by the protocol.
    Auto,
}

/// A TD security scheme (TD 1.1 §5.3.3).
///
/// All schemes share this one flat struct, with the scheme-specific terms
/// optional. Use the constructors to build a well-formed scheme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScheme {
    /// Semantic annotations.
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub semantic_type: Option<OneOrMany<String>>,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Descriptions in several languages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<MultiLanguage>,
    /// URI of a proxy server this scheme applies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// The scheme.
    pub scheme: SecuritySchemeType,
    /// Where the credential is sent (basic, digest, apikey, bearer).
    #[serde(rename = "in", skip_serializing_if = "Option::is_none")]
    pub location: Option<CredentialLocation>,
    /// Name of the header, query parameter or cookie that carries the credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Quality of protection (digest).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qop: Option<String>,
    /// URI of the authorization server (bearer, oauth2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,
    /// Signing algorithm (bearer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alg: Option<String>,
    /// Token format, such as `jwt` (bearer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Identity hint (psk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// URI of the token server (oauth2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// URI of the refresh server (oauth2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
    /// Authorisation scopes (oauth2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<OneOrMany<String>>,
    /// Authorisation flow, such as `code` or `client` (oauth2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// Names of schemes, exactly one of which must be satisfied (combo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_of: Option<Vec<String>>,
    /// Names of schemes that must all be satisfied (combo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_of: Option<Vec<String>>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SecurityScheme {
    fn new(scheme: SecuritySchemeType) -> Self {
        Self {
            semantic_type: None,
            description: None,
            descriptions: None,
            proxy: None,
            scheme,
            location: None,
            name: None,
            qop: None,
            authorization: None,
            alg: None,
            format: None,
            identity: None,
            token: None,
            refresh: None,
            scopes: None,
            flow: None,
            one_of: None,
            all_of: None,
            extra: Map::new(),
        }
    }

    /// No security.
    pub fn nosec() -> Self {
        Self::new(SecuritySchemeType::Nosec)
    }

    /// Security negotiated by the protocol.
    pub fn auto() -> Self {
        Self::new(SecuritySchemeType::Auto)
    }

    /// HTTP Basic authentication in the `Authorization` header.
    pub fn basic() -> Self {
        Self::new(SecuritySchemeType::Basic)
    }

    /// HTTP Digest authentication.
    pub fn digest() -> Self {
        Self::new(SecuritySchemeType::Digest)
    }

    /// An API key sent in `location` under `name`.
    pub fn apikey(location: CredentialLocation, name: impl Into<String>) -> Self {
        Self {
            location: Some(location),
            name: Some(name.into()),
            ..Self::new(SecuritySchemeType::Apikey)
        }
    }

    /// A bearer token in the `Authorization` header.
    pub fn bearer() -> Self {
        Self::new(SecuritySchemeType::Bearer)
    }

    /// A pre-shared key.
    pub fn psk() -> Self {
        Self::new(SecuritySchemeType::Psk)
    }

    /// OAuth 2.0 with the given flow, such as `code` or `client`.
    pub fn oauth2(flow: impl Into<String>) -> Self {
        Self {
            flow: Some(flow.into()),
            ..Self::new(SecuritySchemeType::Oauth2)
        }
    }

    /// Exactly one of the named schemes must be satisfied.
    pub fn combo_one_of<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            one_of: Some(names.into_iter().map(Into::into).collect()),
            ..Self::new(SecuritySchemeType::Combo)
        }
    }

    /// All of the named schemes must be satisfied.
    pub fn combo_all_of<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            all_of: Some(names.into_iter().map(Into::into).collect()),
            ..Self::new(SecuritySchemeType::Combo)
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The names of other schemes this one refers to (combo schemes only).
    pub fn referenced_schemes(&self) -> impl Iterator<Item = &str> {
        self.one_of
            .iter()
            .chain(self.all_of.iter())
            .flatten()
            .map(String::as_str)
    }
}
