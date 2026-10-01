//! Links from a Thing to other resources.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::one_or_many::OneOrMany;

/// A TD `Link` (TD 1.1 §5.3.4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// Target URI.
    pub href: String,
    /// Media type of the target, for example `multipart/x-mixed-replace`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Relation type, for example `alternate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    /// URI that overrides the link's context (by default the Thing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Icon sizes, for `rel: "icon"` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizes: Option<String>,
    /// Language of the target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hreflang: Option<OneOrMany<String>>,
    /// Other terms.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Link {
    /// A link with only an `href`.
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            media_type: None,
            rel: None,
            anchor: None,
            sizes: None,
            hreflang: None,
            extra: Map::new(),
        }
    }

    /// Sets the relation type.
    pub fn with_rel(mut self, rel: impl Into<String>) -> Self {
        self.rel = Some(rel.into());
        self
    }

    /// Sets the media type of the target.
    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }
}
