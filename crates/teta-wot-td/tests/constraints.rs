//! Integration tests for applying value constraints to TD data schemas.

use serde_json::json;
use teta_wot_td::{Bound, ConstraintError, Constraints, DataSchema, DataType};

#[test]
fn numeric_constraints_write_the_expected_schema_terms() {
    let mut schema = DataSchema::of(DataType::Integer);
    Constraints::new()
        .gt(-2)
        .ge(-1)
        .lt(10)
        .le(9)
        .multiple_of(2)
        .apply(&mut schema)
        .unwrap();

    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({
            "type": "integer",
            "exclusiveMinimum": -2,
            "minimum": -1,
            "exclusiveMaximum": 10,
            "maximum": 9,
            "multipleOf": 2
        })
    );

    let mut number = DataSchema::of(DataType::Number);
    Constraints::new()
        .multiple_of(0.5)
        .allow_inf_nan(false)
        .apply(&mut number)
        .unwrap();
    assert_eq!(
        serde_json::to_value(number).unwrap(),
        json!({ "type": "number", "multipleOf": 0.5 })
    );
}

#[test]
fn lengths_follow_the_schema_type() {
    let cases = [
        (
            DataType::String,
            json!({ "type": "string", "minLength": 2, "maxLength": 5 }),
        ),
        (
            DataType::Array,
            json!({ "type": "array", "minItems": 2, "maxItems": 5 }),
        ),
        (
            DataType::Object,
            json!({ "type": "object", "minProperties": 2, "maxProperties": 5 }),
        ),
    ];

    for (data_type, expected) in cases {
        let mut schema = DataSchema::of(data_type);
        Constraints::new()
            .min_length(2)
            .max_length(5)
            .apply(&mut schema)
            .unwrap();
        assert_eq!(serde_json::to_value(schema).unwrap(), expected);
    }

    let mut string = DataSchema::of(DataType::String);
    Constraints::new()
        .pattern("^[A-Z]+$")
        .apply(&mut string)
        .unwrap();
    assert_eq!(
        serde_json::to_value(string).unwrap(),
        json!({ "type": "string", "pattern": "^[A-Z]+$" })
    );
}

#[test]
fn one_of_constraints_apply_only_to_suitable_branches() {
    let mut schema = DataSchema {
        one_of: Some(vec![
            DataSchema::of(DataType::Integer),
            DataSchema::of(DataType::String),
            DataSchema::of(DataType::Null),
        ]),
        ..DataSchema::any()
    };

    Constraints::new()
        .ge(1)
        .min_length(2)
        .pattern("^x")
        .apply(&mut schema)
        .unwrap();

    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({
            "oneOf": [
                { "type": "integer", "minimum": 1 },
                { "type": "string", "minLength": 2, "pattern": "^x" },
                { "type": "null" }
            ]
        })
    );
}

#[test]
fn empty_constraints_leave_even_an_untyped_schema_unchanged() {
    let constraints = Constraints::new();
    assert!(constraints.is_empty());

    let mut schema = DataSchema::any();
    constraints.apply(&mut schema).unwrap();
    assert_eq!(serde_json::to_value(schema).unwrap(), json!({}));
    assert!(!Constraints::new().ge(1).is_empty());
}

#[test]
fn large_unsigned_bounds_are_not_truncated() {
    assert_eq!(Bound::from(u64::MAX), Bound::UInt(u64::MAX));
    assert_eq!(Bound::from(7_u64), Bound::Int(7));

    let mut schema = DataSchema::of(DataType::Integer);
    Constraints::new().le(u64::MAX).apply(&mut schema).unwrap();
    assert_eq!(
        serde_json::to_value(schema).unwrap(),
        json!({ "type": "integer", "maximum": u64::MAX })
    );
}

#[test]
fn invalid_constraints_return_specific_errors() {
    let mut integer = DataSchema::of(DataType::Integer);
    assert_eq!(
        Constraints::new().pattern("x").apply(&mut integer),
        Err(ConstraintError::Unsuitable {
            constraint: "pattern",
            data_type: Some(DataType::Integer),
        })
    );

    let mut untyped = DataSchema::any();
    assert_eq!(
        Constraints::new().ge(1).apply(&mut untyped),
        Err(ConstraintError::Untyped)
    );

    let mut string_or_null = DataSchema {
        one_of: Some(vec![
            DataSchema::of(DataType::String),
            DataSchema::of(DataType::Null),
        ]),
        ..DataSchema::any()
    };
    assert_eq!(
        Constraints::new().gt(0).apply(&mut string_or_null),
        Err(ConstraintError::Unsuitable {
            constraint: "gt",
            data_type: None,
        })
    );

    for invalid in [Bound::Int(0), Bound::Int(-1), Bound::Float(0.0)] {
        let mut number = DataSchema::of(DataType::Number);
        assert_eq!(
            Constraints::new().multiple_of(invalid).apply(&mut number),
            Err(ConstraintError::NotPositive)
        );
    }

    let mut number = DataSchema::of(DataType::Number);
    assert_eq!(
        Constraints::new().gt(f64::INFINITY).apply(&mut number),
        Err(ConstraintError::NotFinite { constraint: "gt" })
    );
    assert_eq!(
        Constraints::new().multiple_of(f64::NAN).apply(&mut number),
        Err(ConstraintError::NotFinite {
            constraint: "multiple_of",
        })
    );
}

#[test]
fn an_invalid_bound_does_not_partially_modify_a_schema() {
    let mut schema = DataSchema::of(DataType::Number);
    let original = schema.clone();

    assert_eq!(
        Constraints::new()
            .ge(1)
            .le(f64::INFINITY)
            .apply(&mut schema),
        Err(ConstraintError::NotFinite { constraint: "le" })
    );
    assert_eq!(schema, original);

    let mut union = DataSchema {
        one_of: Some(vec![
            DataSchema::of(DataType::String),
            DataSchema::of(DataType::Number),
        ]),
        ..DataSchema::any()
    };
    let original = union.clone();
    assert_eq!(
        Constraints::new()
            .min_length(2)
            .le(f64::INFINITY)
            .apply(&mut union),
        Err(ConstraintError::NotFinite { constraint: "le" })
    );
    assert_eq!(union, original);
}
