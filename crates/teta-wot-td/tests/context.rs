//! Integration tests for Thing Description contexts.

use serde_json::json;
use teta_wot_td::{Context, ContextEntry, TD_CONTEXT_V1, TD_CONTEXT_V1_1, TdError};

#[test]
fn default_context_is_the_td_1_1_uri() {
    let context = Context::default();

    assert_eq!(
        serde_json::to_value(&context).unwrap(),
        json!(TD_CONTEXT_V1_1)
    );
    assert!(context.validate().is_ok());
}

#[test]
fn adding_entries_keeps_the_base_uri_and_combines_prefixes() {
    let mut context = Context::default();
    context.push(ContextEntry::from("https://example.test/context"));
    context.add_prefix("saref", "https://saref.etsi.org/core/");
    context.add_prefix("ex", "https://example.test/vocab#");

    let expected = json!([
        TD_CONTEXT_V1_1,
        "https://example.test/context",
        {
            "saref": "https://saref.etsi.org/core/",
            "ex": "https://example.test/vocab#"
        }
    ]);
    assert_eq!(serde_json::to_value(&context).unwrap(), expected);
    assert!(context.validate().is_ok());

    let decoded: Context = serde_json::from_value(expected).unwrap();
    assert_eq!(decoded, context);
}

#[test]
fn td_1_0_uri_is_valid_only_immediately_before_td_1_1() {
    let valid = json!([
        TD_CONTEXT_V1,
        TD_CONTEXT_V1_1,
        { "@language": "en", "ex": "https://example.test/vocab#" }
    ]);
    let context: Context = serde_json::from_value(valid.clone()).unwrap();
    assert!(context.validate().is_ok());
    assert_eq!(serde_json::to_value(context).unwrap(), valid);

    let invalid = [
        json!(TD_CONTEXT_V1),
        json!([TD_CONTEXT_V1]),
        json!([
            TD_CONTEXT_V1,
            "https://example.test/context",
            TD_CONTEXT_V1_1
        ]),
        json!([TD_CONTEXT_V1_1, TD_CONTEXT_V1]),
        json!([TD_CONTEXT_V1, TD_CONTEXT_V1_1, TD_CONTEXT_V1]),
    ];
    for document in invalid {
        let context: Context = serde_json::from_value(document.clone()).unwrap();
        assert!(
            matches!(context.validate(), Err(TdError::InvalidContext(_))),
            "unexpectedly valid: {document}"
        );
    }
}

#[test]
fn empty_or_unrelated_contexts_are_rejected() {
    let invalid = [
        json!([]),
        json!("https://example.test/context"),
        json!(["https://example.test/context", TD_CONTEXT_V1_1]),
        json!([{ "ex": "https://example.test/vocab#" }, TD_CONTEXT_V1_1]),
    ];
    for document in invalid {
        let context: Context = serde_json::from_value(document.clone()).unwrap();
        assert!(
            matches!(context.validate(), Err(TdError::InvalidContext(_))),
            "unexpectedly valid: {document}"
        );
    }
}
