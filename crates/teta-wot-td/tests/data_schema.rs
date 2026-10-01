//! Integration tests for Thing Description data schemas.

use serde_json::{Value, json};
use teta_wot_td::{ArrayItems, DataSchema, DataType};

#[test]
fn data_types_use_td_names_and_reject_type_arrays() {
    let cases = [
        (DataType::Boolean, "boolean"),
        (DataType::Integer, "integer"),
        (DataType::Number, "number"),
        (DataType::String, "string"),
        (DataType::Object, "object"),
        (DataType::Array, "array"),
        (DataType::Null, "null"),
    ];

    for (data_type, wire_name) in cases {
        assert_eq!(data_type.as_str(), wire_name);
        assert_eq!(serde_json::to_value(data_type).unwrap(), json!(wire_name));
        assert_eq!(
            serde_json::from_value::<DataType>(json!(wire_name)).unwrap(),
            data_type
        );
    }
    assert!(serde_json::from_value::<DataSchema>(json!({ "type": ["integer", "null"] })).is_err());
}

#[test]
fn builders_serialize_typed_terms_and_grow_semantic_annotations() {
    assert_eq!(serde_json::to_value(DataSchema::any()).unwrap(), json!({}));

    let single_annotation =
        DataSchema::of(DataType::Number).with_semantic_type("saref:Temperature");
    assert_eq!(
        serde_json::to_value(single_annotation).unwrap(),
        json!({ "type": "number", "@type": "saref:Temperature" })
    );

    let schema = DataSchema::of(DataType::Number)
        .with_title("Temperature")
        .with_description("Current reading")
        .with_unit("degree Celsius")
        .with_default(json!(21.5))
        .with_semantic_type("saref:Temperature")
        .with_semantic_type("ex:Reading");
    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({
            "type": "number",
            "title": "Temperature",
            "description": "Current reading",
            "unit": "degree Celsius",
            "default": 21.5,
            "@type": ["saref:Temperature", "ex:Reading"]
        })
    );
}

#[test]
fn null_default_and_const_are_omitted() {
    let mut schema = DataSchema::any().with_default(Value::Null);
    schema.constant = Some(Value::Null);
    assert_eq!(serde_json::to_value(schema).unwrap(), json!({}));

    let decoded: DataSchema = serde_json::from_value(json!({
        "default": null,
        "const": null
    }))
    .unwrap();
    assert_eq!(decoded.default, None);
    assert_eq!(decoded.constant, None);
}

#[test]
fn nested_object_schema_round_trips_with_extension_terms() {
    let document = json!({
        "type": "object",
        "@type": "ex:Measurement",
        "titles": { "en": "Measurement", "fr": "Mesure" },
        "descriptions": { "en": "A sensor reading" },
        "readOnly": true,
        "properties": {
            "reading": {
                "oneOf": [
                    { "type": "number", "minimum": -40, "maximum": 125 },
                    { "type": "null" }
                ]
            },
            "status": { "type": "string", "enum": ["ok", "warning"] }
        },
        "required": ["reading"],
        "additionalProperties": false,
        "ex:quality": "calibrated"
    });

    let schema: DataSchema = serde_json::from_value(document.clone()).unwrap();
    assert_eq!(schema.data_type, Some(DataType::Object));
    assert_eq!(schema.extra["additionalProperties"], json!(false));
    assert_eq!(schema.extra["ex:quality"], json!("calibrated"));
    assert_eq!(serde_json::to_value(schema).unwrap(), document);
}

#[test]
fn array_items_support_single_and_tuple_schemas() {
    let single = json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 0 },
        "minItems": 1,
        "maxItems": 3
    });
    let schema: DataSchema = serde_json::from_value(single.clone()).unwrap();
    assert!(matches!(schema.items.as_ref(), Some(ArrayItems::Single(_))));
    assert_eq!(serde_json::to_value(schema).unwrap(), single);

    let tuple = json!({
        "type": "array",
        "items": [
            { "type": "string", "minLength": 1 },
            { "type": "number", "multipleOf": 0.5 }
        ]
    });
    let schema: DataSchema = serde_json::from_value(tuple.clone()).unwrap();
    assert!(matches!(schema.items.as_ref(), Some(ArrayItems::Tuple(_))));
    assert_eq!(serde_json::to_value(schema).unwrap(), tuple);
}

#[test]
fn string_constraints_keep_their_json_names() {
    let document = json!({
        "type": "string",
        "minLength": 2,
        "maxLength": 10,
        "pattern": "^[A-Z]+$",
        "format": "uri",
        "contentEncoding": "base64",
        "contentMediaType": "text/plain",
        "const": "AB"
    });

    let schema: DataSchema = serde_json::from_value(document.clone()).unwrap();
    assert_eq!(schema.data_type, Some(DataType::String));
    assert_eq!(serde_json::to_value(schema).unwrap(), document);
}
