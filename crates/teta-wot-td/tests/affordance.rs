//! Integration tests for property, action, and event affordances.

use serde_json::json;
use teta_wot_td::{
    ActionAffordance, DataSchema, DataType, EventAffordance, Form, Operation, PropertyAffordance,
};

#[test]
fn property_builder_flattens_schema_terms_and_keeps_extension_terms_last() {
    let mut schema = DataSchema::of(DataType::Integer)
        .with_title("Old title")
        .with_description("Old description");
    schema.extra.insert("ex:source".into(), json!("sensor"));

    let property = PropertyAffordance::builder(schema)
        .title("Temperature")
        .description("Current reading")
        .read_only(true)
        .write_only(false)
        .observable(true)
        .default_value(json!(21))
        .unit("degree Celsius")
        .semantic_type("saref:Temperature")
        .semantic_type("ex:Reading")
        .form(Form::new("/temperature").with_op(Operation::ReadProperty))
        .uri_variable("id", DataSchema::of(DataType::String))
        .build();

    let expected = json!({
        "type": "integer",
        "title": "Temperature",
        "description": "Current reading",
        "readOnly": true,
        "writeOnly": false,
        "observable": true,
        "default": 21,
        "unit": "degree Celsius",
        "@type": ["saref:Temperature", "ex:Reading"],
        "forms": [{ "href": "/temperature", "op": "readproperty" }],
        "uriVariables": { "id": { "type": "string" } },
        "ex:source": "sensor"
    });
    assert_eq!(serde_json::to_value(&property).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<PropertyAffordance>(expected).unwrap(),
        property
    );

    let serialized = serde_json::to_string(&property).unwrap();
    let position = |key: &str| serialized.find(key).unwrap();
    assert!(position("\"readOnly\"") < position("\"forms\""));
    assert!(position("\"forms\"") < position("\"uriVariables\""));
    assert!(position("\"uriVariables\"") < position("\"observable\""));
    assert!(position("\"observable\"") < position("\"ex:source\""));
}

#[test]
fn property_builder_writes_access_flags_and_omits_null_default() {
    let property: PropertyAffordance = PropertyAffordance::builder(DataSchema::of(DataType::Null))
        .default_value(json!(null))
        .form(Form::new("/empty"))
        .into();

    assert_eq!(
        serde_json::to_value(property).unwrap(),
        json!({
            "type": "null",
            "readOnly": false,
            "writeOnly": false,
            "forms": [{ "href": "/empty" }]
        })
    );
}

#[test]
fn action_builder_round_trips_input_output_and_flags() {
    let action: ActionAffordance = ActionAffordance::builder()
        .title("Reset")
        .description("Reset the device")
        .input(DataSchema::of(DataType::Integer))
        .output(DataSchema::of(DataType::Boolean))
        .safe(false)
        .idempotent(true)
        .synchronous(false)
        .semantic_type("ex:ResetAction")
        .form(Form::new("/reset").with_op(Operation::InvokeAction))
        .uri_variable("id", DataSchema::of(DataType::String))
        .into();

    let expected = json!({
        "title": "Reset",
        "description": "Reset the device",
        "input": { "type": "integer" },
        "output": { "type": "boolean" },
        "safe": false,
        "idempotent": true,
        "synchronous": false,
        "@type": "ex:ResetAction",
        "forms": [{ "href": "/reset", "op": "invokeaction" }],
        "uriVariables": { "id": { "type": "string" } }
    });
    assert_eq!(serde_json::to_value(&action).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<ActionAffordance>(expected).unwrap(),
        action
    );
}

#[test]
fn event_builder_and_deserializer_preserve_event_data_terms() {
    let event = EventAffordance::builder()
        .title("Alarm")
        .description("Raised when a threshold is crossed")
        .subscription(DataSchema::of(DataType::String))
        .data(DataSchema::of(DataType::Number))
        .cancellation(DataSchema::of(DataType::Boolean))
        .semantic_type("ex:AlarmEvent")
        .semantic_type("ex:Notification")
        .form(Form::new("/alarm").with_op(Operation::SubscribeEvent))
        .build();

    let expected = json!({
        "title": "Alarm",
        "description": "Raised when a threshold is crossed",
        "subscription": { "type": "string" },
        "data": { "type": "number" },
        "cancellation": { "type": "boolean" },
        "@type": ["ex:AlarmEvent", "ex:Notification"],
        "forms": [{ "href": "/alarm", "op": "subscribeevent" }]
    });
    assert_eq!(serde_json::to_value(&event).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<EventAffordance>(expected).unwrap(),
        event
    );

    let full = json!({
        "forms": [{ "href": "/alarm" }],
        "dataResponse": { "type": "object" },
        "uriVariables": { "id": { "type": "integer" } },
        "titles": { "en": "Alarm", "fr": "Alarme" },
        "ex:severity": "high"
    });
    let decoded: EventAffordance = serde_json::from_value(full.clone()).unwrap();
    assert_eq!(decoded.extra["ex:severity"], json!("high"));
    assert_eq!(serde_json::to_value(decoded).unwrap(), full);
}

#[test]
fn deserialization_requires_a_forms_field() {
    assert!(serde_json::from_value::<PropertyAffordance>(json!({ "type": "string" })).is_err());
    assert!(serde_json::from_value::<ActionAffordance>(json!({ "title": "Reset" })).is_err());
    assert!(serde_json::from_value::<EventAffordance>(json!({ "title": "Alarm" })).is_err());
}
