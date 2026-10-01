//! Integration tests for Thing Description links.

use serde_json::json;
use teta_wot_td::{Link, OneOrMany};

#[test]
fn new_link_serializes_with_only_href() {
    let link = Link::new("https://example.test/thing");

    assert_eq!(
        serde_json::to_value(link).unwrap(),
        json!({
            "href": "https://example.test/thing"
        })
    );
}

#[test]
fn builder_uses_type_for_media_type() {
    let link = Link::new("/alternate")
        .with_rel("alternate")
        .with_media_type("application/td+json");

    assert_eq!(
        serde_json::to_value(link).unwrap(),
        json!({
            "href": "/alternate",
            "rel": "alternate",
            "type": "application/td+json"
        })
    );
}

#[test]
fn link_round_trips_optional_fields_and_extension_terms() {
    let document = json!({
        "href": "https://example.test/icon.png",
        "type": "image/png",
        "rel": "icon",
        "anchor": "https://example.test/thing",
        "sizes": "48x48",
        "hreflang": ["en", "fr"],
        "x-link-term": { "priority": 1 }
    });

    let link: Link = serde_json::from_value(document.clone()).unwrap();
    assert_eq!(
        link.hreflang.as_ref().unwrap(),
        &OneOrMany::Many(vec!["en".to_owned(), "fr".to_owned()])
    );
    assert_eq!(link.extra["x-link-term"], json!({ "priority": 1 }));
    assert_eq!(serde_json::to_value(link).unwrap(), document);

    let single_language = json!({ "href": "/help", "hreflang": "en" });
    let link: Link = serde_json::from_value(single_language.clone()).unwrap();
    assert_eq!(
        link.hreflang.as_ref().unwrap(),
        &OneOrMany::One("en".to_owned())
    );
    assert_eq!(serde_json::to_value(link).unwrap(), single_language);
}

#[test]
fn link_requires_href() {
    assert!(serde_json::from_value::<Link>(json!({ "rel": "alternate" })).is_err());
}
