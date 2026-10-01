//! Integration tests for the vendored TD 1.1 JSON Schema validator.

#![cfg(feature = "validation")]

use serde_json::{Value, json};
use teta_wot_td::validation::{TD_JSON_SCHEMA, validate_json};
use teta_wot_td::{
    DataSchema, DataType, Form, Operation, PropertyAffordance, TD_CONTEXT_V1_1, ThingDescription,
};

fn minimal_td() -> Value {
    json!({
        "@context": TD_CONTEXT_V1_1,
        "title": "Example Thing",
        "security": "nosec_sc",
        "securityDefinitions": {
            "nosec_sc": { "scheme": "nosec" }
        }
    })
}

#[test]
fn vendored_schema_parses_and_accepts_a_minimal_td() {
    let schema: Value = serde_json::from_str(TD_JSON_SCHEMA).unwrap();
    assert_eq!(schema["version"], "1.1-12-March-2025");
    assert_eq!(schema["$id"], "https://www.w3.org/2022/wot/td-schema/v1.1");
    validate_json(&minimal_td()).unwrap();
}

#[test]
fn validation_reports_required_fields_and_nested_paths() {
    let missing = validate_json(&json!({})).unwrap_err();
    assert!(missing.violations.len() >= 4);
    assert!(
        missing
            .violations
            .iter()
            .all(|v| v.instance_path.is_empty())
    );
    assert!(
        missing
            .violations
            .iter()
            .all(|v| v.schema_path.ends_with("/required"))
    );

    let mut td = minimal_td();
    td["title"] = json!(42);
    td["created"] = json!("not-a-date");
    let invalid = validate_json(&td).unwrap_err();
    assert!(
        invalid
            .violations
            .iter()
            .any(|v| v.instance_path == "/title")
    );
    assert!(
        invalid
            .violations
            .iter()
            .any(|v| v.instance_path == "/created")
    );
    assert!(invalid.to_string().contains("the TD doesn't match"));
    assert!(invalid.to_string().contains("/title"));
}

#[test]
fn validation_rejects_an_invalid_property_affordance() {
    let mut td = minimal_td();
    td["properties"] = json!({
        "temperature": { "type": "number" }
    });

    let error = validate_json(&td).unwrap_err();
    assert!(
        error
            .violations
            .iter()
            .any(|v| v.instance_path.starts_with("/properties/temperature"))
    );
}

#[test]
fn thing_description_method_validates_serialized_td() {
    let property = PropertyAffordance::builder(DataSchema::of(DataType::Number))
        .form(Form::new("/temperature").with_op(Operation::ReadProperty));
    let mut td = ThingDescription::builder("Temperature Sensor")
        .property("temperature", property)
        .build()
        .unwrap();

    td.validate_schema().unwrap();

    td.created = Some("not-a-date".into());
    let error = td.validate_schema().unwrap_err();
    assert!(
        error
            .violations
            .iter()
            .any(|v| v.instance_path == "/created")
    );
}
