//! Integration tests for JSON Schema to TD DataSchema conversion.

use serde_json::json;
use teta_wot_td::convert::{ConversionOptions, from_json_schema, from_json_schema_with};
use teta_wot_td::{ConversionError, DataType};

#[test]
fn null_const_selects_only_the_null_type() {
    let schema = from_json_schema(&json!({
        "type": ["string", "null"],
        "const": null
    }))
    .unwrap();

    assert_eq!(schema.data_type, Some(DataType::Null));
    assert!(schema.one_of.is_none());
    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({ "type": "null" })
    );
}

#[test]
fn null_const_conflicting_with_a_non_null_type_is_rejected() {
    let error = from_json_schema(&json!({
        "type": "string",
        "const": null
    }))
    .unwrap_err();

    assert!(matches!(error, ConversionError::Conflict { .. }));
}

#[test]
fn number_subsumes_integer_in_a_type_union() {
    let schema = from_json_schema(&json!({
        "type": ["integer", "number"],
        "minimum": 0
    }))
    .unwrap();

    assert_eq!(schema.data_type, Some(DataType::Number));
    assert!(schema.one_of.is_none());
    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({ "type": "number", "minimum": 0 })
    );
}

#[test]
fn local_refs_inline_and_sibling_terms_override_them() {
    let schema = from_json_schema(&json!({
        "$defs": {
            "Reading": { "type": "number", "description": "Base description" }
        },
        "$ref": "#/$defs/Reading",
        "description": "Field description"
    }))
    .unwrap();

    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({ "type": "number", "description": "Field description" })
    );
}

#[test]
fn tuple_items_are_preserved_and_false_items_is_removed() {
    let schema = from_json_schema(&json!({
        "type": "array",
        "prefixItems": [{ "type": "string" }, { "type": "integer" }],
        "items": false,
        "maxItems": 2
    }))
    .unwrap();

    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({
            "type": "array",
            "items": [{ "type": "string" }, { "type": "integer" }],
            "maxItems": 2
        })
    );
}

#[test]
fn recursive_refs_stop_at_the_configured_limit() {
    let error = from_json_schema_with(
        &json!({
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": { "next": { "$ref": "#/$defs/Node" } }
                }
            },
            "$ref": "#/$defs/Node"
        }),
        &ConversionOptions { recursion_limit: 4 },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConversionError::RecursionLimit { limit: 4, .. }
    ));
}

#[test]
fn nested_single_all_of_counts_toward_the_recursion_limit() {
    let error = from_json_schema_with(
        &json!({
            "allOf": [{
                "allOf": [{
                    "allOf": [{ "type": "string" }]
                }]
            }]
        }),
        &ConversionOptions { recursion_limit: 2 },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConversionError::RecursionLimit { limit: 2, .. }
    ));
}
