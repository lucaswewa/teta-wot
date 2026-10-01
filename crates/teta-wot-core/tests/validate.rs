//! Integration tests for JSON coercion, schema validation, and error reporting.

use serde_json::{Value, json};
use teta_wot_core::validate::{SchemaValidator, coercion};
use teta_wot_core::{LocItem, ValidationError};
use teta_wot_td::DataSchema;

fn validator(schema: Value) -> SchemaValidator {
    let schema: DataSchema = serde_json::from_value(schema).unwrap();
    SchemaValidator::new(schema).unwrap()
}

fn check(schema: Value, input: Value) -> Result<Value, ValidationError> {
    validator(schema).validate(&input, &[LocItem::from("body")])
}

fn assert_issue(schema: Value, input: Value, kind: &str, context: Value) {
    let error = check(schema, input.clone()).unwrap_err();
    assert_eq!(error.issues.len(), 1);
    let issue = &error.issues[0];
    assert_eq!(issue.kind, kind);
    assert_eq!(issue.loc, [LocItem::from("body")]);
    assert_eq!(issue.input, input);
    assert_eq!(serde_json::to_value(&issue.ctx).unwrap(), context);
    assert!(!issue.msg.is_empty());
}

#[test]
fn integer_coercion_accepts_whole_numbers_booleans_and_digit_strings() {
    for (input, expected) in [
        (json!(42), json!(42)),
        (json!(u64::MAX), json!(u64::MAX)),
        (json!(i64::MIN), json!(i64::MIN)),
        (json!(2.0), json!(2)),
        (json!(-2.0), json!(-2)),
        (json!(true), json!(1)),
        (json!(false), json!(0)),
        (json!(" +1_234.00 "), json!(1234)),
        (json!("-002"), json!(-2)),
        (json!(u64::MAX.to_string()), json!(u64::MAX)),
    ] {
        assert_eq!(coercion::integer(&input).unwrap(), expected, "{input}");
    }
}

#[test]
fn integer_coercion_distinguishes_parsing_type_fraction_and_size_errors() {
    for input in [
        "",
        "+",
        "_1",
        "1_",
        "1__2",
        "2.",
        "2.01",
        "1e3",
        "0x10",
        "１２",
        "18446744073709551616",
    ] {
        assert_eq!(
            coercion::integer(&json!(input)).unwrap_err().0,
            "int_parsing",
            "{input}"
        );
    }
    for input in [Value::Null, json!([]), json!({})] {
        assert_eq!(
            coercion::integer(&input).unwrap_err(),
            ("int_type", "Input should be a valid integer")
        );
    }
    assert_eq!(
        coercion::integer(&json!(1.5)).unwrap_err().0,
        "int_from_float"
    );
    assert_eq!(
        coercion::integer(&json!(1e20)).unwrap_err().0,
        "int_parsing_size"
    );
}

#[test]
fn number_coercion_returns_floats_and_accepts_exponents_and_separators() {
    for (input, expected) in [
        (json!(2), 2.0),
        (json!(2.5), 2.5),
        (json!(true), 1.0),
        (json!(false), 0.0),
        (json!(" -1_234.5 "), -1234.5),
        (json!("1e3"), 1000.0),
        (json!("2."), 2.0),
        (json!(".25"), 0.25),
    ] {
        let value = coercion::number(&input).unwrap();
        assert_eq!(value, json!(expected), "{input}");
        assert!(value.as_number().unwrap().is_f64());
    }
    for input in [
        "", "nope", "inf", "NaN", "1e999", "_1", "1_", "1__2", "1_.0", "1._0",
    ] {
        assert_eq!(
            coercion::number(&json!(input)).unwrap_err().0,
            "float_parsing",
            "{input}"
        );
    }
    for input in [Value::Null, json!([]), json!({})] {
        assert_eq!(coercion::number(&input).unwrap_err().0, "float_type");
    }
}

#[test]
fn boolean_coercion_accepts_documented_spellings_and_zero_or_one() {
    for spelling in ["0", "off", "f", "false", "n", "no"] {
        assert_eq!(
            coercion::boolean(&json!(spelling.to_uppercase())).unwrap(),
            json!(false)
        );
    }
    for spelling in ["1", "on", "t", "true", "y", "yes"] {
        assert_eq!(
            coercion::boolean(&json!(spelling.to_uppercase())).unwrap(),
            json!(true)
        );
    }
    for (input, expected) in [
        (json!(false), false),
        (json!(true), true),
        (json!(0), false),
        (json!(1), true),
        (json!(0.0), false),
        (json!(1.0), true),
    ] {
        assert_eq!(coercion::boolean(&input).unwrap(), json!(expected));
    }
    for input in [json!(2), json!(u64::MAX), json!("maybe"), json!(" true ")] {
        assert_eq!(coercion::boolean(&input).unwrap_err().0, "bool_parsing");
    }
    for input in [json!(2.0), json!(0.5), Value::Null, json!([]), json!({})] {
        assert_eq!(coercion::boolean(&input).unwrap_err().0, "bool_type");
    }
}

#[test]
fn string_coercion_preserves_strings_and_rejects_other_json_types() {
    for input in [json!(""), json!(" café 🦀 ")] {
        assert_eq!(coercion::string(&input).unwrap(), input);
    }
    for input in [json!(1), json!(true), Value::Null, json!([]), json!({})] {
        assert_eq!(
            coercion::string(&input).unwrap_err(),
            ("string_type", "Input should be a valid string")
        );
    }
}

#[test]
fn primitive_schemas_apply_coercions_and_leave_input_unchanged() {
    for (schema, input, expected) in [
        (json!({"type":"integer"}), json!("2"), json!(2)),
        (json!({"type":"number"}), json!("2"), json!(2.0)),
        (json!({"type":"boolean"}), json!("YES"), json!(true)),
        (json!({"type":"string"}), json!("text"), json!("text")),
        (json!({"type":"null"}), Value::Null, Value::Null),
        (
            json!({}),
            json!({"free": [1, null]}),
            json!({"free": [1, null]}),
        ),
    ] {
        let validator = validator(schema);
        let original = input.clone();
        assert_eq!(validator.validate(&input, &[]).unwrap(), expected);
        assert_eq!(input, original);
    }
    assert_issue(
        json!({"type":"null"}),
        json!(0),
        "none_required",
        Value::Null,
    );
}

#[test]
fn numeric_bounds_report_original_input_and_typed_context() {
    for (keyword, bound, input, kind, key) in [
        ("minimum", 2, "1", "greater_than_equal", "ge"),
        ("exclusiveMinimum", 2, "2", "greater_than", "gt"),
        ("maximum", 2, "3", "less_than_equal", "le"),
        ("exclusiveMaximum", 2, "2", "less_than", "lt"),
        ("multipleOf", 2, "3", "multiple_of", "multiple_of"),
    ] {
        for data_type in ["integer", "number"] {
            let mut schema = json!({"type": data_type});
            schema[keyword] = json!(bound);
            let context_bound = if data_type == "number" {
                json!(bound as f64)
            } else {
                json!(bound)
            };
            let context = Value::Object(serde_json::Map::from_iter([(key.into(), context_bound)]));
            assert_issue(schema, json!(input), kind, context);
        }
    }
    let schema = json!({"type":"integer", "minimum":2, "maximum":4});
    for input in [2, 4] {
        assert_eq!(check(schema.clone(), json!(input)).unwrap(), json!(input));
    }
    assert_eq!(
        check(json!({"type":"number", "multipleOf":0.1}), json!(0.3)).unwrap(),
        json!(0.3)
    );
}

#[test]
fn numeric_constraint_failures_follow_documented_priority() {
    let schema = json!({"type":"integer", "multipleOf":2, "maximum":0, "exclusiveMaximum":0,
        "minimum":10, "exclusiveMinimum":10});
    for (removed, kind) in [
        (None, "multiple_of"),
        (Some("multipleOf"), "less_than_equal"),
        (Some("maximum"), "less_than"),
        (Some("exclusiveMaximum"), "greater_than_equal"),
        (Some("minimum"), "greater_than"),
    ]
    .into_iter()
    .scan(schema, |schema, (removed, kind)| {
        if let Some(key) = removed {
            schema.as_object_mut().unwrap().remove(key);
        }
        Some((schema.clone(), kind))
    }) {
        let error = check(removed, json!(3)).unwrap_err();
        assert_eq!(error.issues.len(), 1);
        assert_eq!(error.issues[0].kind, kind);
    }
}

#[test]
fn string_bounds_count_unicode_characters_and_check_length_before_pattern() {
    let schema = json!({"type":"string", "minLength":2, "maxLength":3, "pattern":"^[é🦀]+$"});
    assert_eq!(check(schema.clone(), json!("é🦀")).unwrap(), json!("é🦀"));
    assert_issue(
        schema.clone(),
        json!("x"),
        "string_too_short",
        json!({"min_length":2}),
    );
    assert_issue(
        schema.clone(),
        json!("xxxx"),
        "string_too_long",
        json!({"max_length":3}),
    );
    assert_issue(
        schema,
        json!("xx"),
        "string_pattern_mismatch",
        json!({"pattern":"^[é🦀]+$"}),
    );
    assert_eq!(
        check(
            json!({"type":"string", "pattern":"cat"}),
            json!("a cat naps")
        )
        .unwrap(),
        json!("a cat naps")
    );
}

#[test]
fn invalid_patterns_are_rejected_in_every_supported_child_schema() {
    for schema in [
        json!({"type":"string", "pattern":"["}),
        json!({"oneOf":[{"type":"string", "pattern":"["}]}),
        json!({"type":"array", "items":{"type":"string", "pattern":"["}}),
        json!({"type":"array", "items":[{"type":"string", "pattern":"["}]}),
        json!({"type":"object", "properties":{"name":{"type":"string", "pattern":"["}}}),
    ] {
        assert!(SchemaValidator::new(serde_json::from_value(schema).unwrap()).is_err());
    }
    let schema = json!({"type":"object", "properties":{
        "first":{"type":"string", "pattern":"^ok$"},
        "second":{"type":"array", "items":{"type":"string", "pattern":"^ok$"}}
    }});
    let validator = validator(schema);
    let clone = validator.clone();
    assert_eq!(
        serde_json::to_value(clone.schema()).unwrap(),
        serde_json::to_value(validator.schema()).unwrap()
    );
    let error = clone
        .validate(&json!({"first":"bad", "second":["bad"]}), &[])
        .unwrap_err();
    assert_eq!(
        error
            .issues
            .iter()
            .map(|i| i.loc.clone())
            .collect::<Vec<_>>(),
        [
            vec![LocItem::from("first")],
            vec![LocItem::from("second"), LocItem::from(0)]
        ]
    );
}

#[test]
fn arrays_coerce_items_and_report_all_indexed_errors() {
    let schema = json!({"type":"array", "items":{"type":"integer"}});
    assert_eq!(
        check(schema.clone(), json!(["2", true, 3.0])).unwrap(),
        json!([2, 1, 3])
    );
    let error = check(schema.clone(), json!(["bad", 1, null])).unwrap_err();
    assert_eq!(
        error
            .issues
            .iter()
            .map(|i| (i.kind.as_str(), i.loc.clone()))
            .collect::<Vec<_>>(),
        [
            ("int_parsing", vec![LocItem::from("body"), LocItem::from(0)]),
            ("int_type", vec![LocItem::from("body"), LocItem::from(2)]),
        ]
    );
    assert_issue(schema, json!({}), "list_type", Value::Null);
    assert_eq!(
        check(json!({"type":"array"}), json!([null, {"x":1}])).unwrap(),
        json!([null, {"x":1}])
    );
}

#[test]
fn array_length_errors_have_context_and_correct_priority() {
    let schema = json!({"type":"array", "items":{"type":"integer"}, "minItems":2, "maxItems":3});
    assert_issue(
        schema.clone(),
        json!(["bad", 1, 2, 3]),
        "too_long",
        json!({"field_type":"List", "max_length":3, "actual_length":4}),
    );
    assert_issue(
        schema.clone(),
        json!([1]),
        "too_short",
        json!({"field_type":"List", "min_length":2, "actual_length":1}),
    );
    let error = check(schema.clone(), json!(["bad"])).unwrap_err();
    assert_eq!(error.issues.len(), 1);
    assert_eq!(error.issues[0].kind, "int_parsing");
    assert_eq!(check(schema.clone(), json!([1, 2])).unwrap(), json!([1, 2]));
    assert_eq!(check(schema, json!([1, 2, 3])).unwrap(), json!([1, 2, 3]));
}

#[test]
fn tuples_validate_positions_and_report_missing_and_excess_items() {
    let schema = json!({"type":"array", "items":[{"type":"integer"},{"type":"boolean"}]});
    assert_eq!(
        check(schema.clone(), json!(["2", "yes"])).unwrap(),
        json!([2, true])
    );
    let error = check(schema.clone(), json!(["bad"])).unwrap_err();
    assert_eq!(error.issues.len(), 2);
    assert_eq!(error.issues[0].kind, "int_parsing");
    assert_eq!(error.issues[1].kind, "missing");
    assert_eq!(
        error.issues[1].loc,
        [LocItem::from("body"), LocItem::from(1)]
    );
    assert_eq!(error.issues[1].input, json!(["bad"]));
    assert_issue(
        schema.clone(),
        json!([1, true, 3]),
        "too_long",
        json!({"field_type":"Tuple", "max_length":2, "actual_length":3}),
    );
    assert_issue(schema, json!(false), "tuple_type", Value::Null);
}

#[test]
fn models_coerce_known_fields_ignore_unknown_fields_and_do_not_fill_defaults() {
    let schema = json!({"type":"object", "properties":{
        "count":{"type":"integer"}, "label":{"type":"string", "default":"untitled"}
    }, "required":["count"]});
    assert_eq!(
        check(schema.clone(), json!({"count":"2", "extra":99})).unwrap(),
        json!({"count":2})
    );
    let error = check(schema.clone(), json!({})).unwrap_err();
    assert_eq!(error.issues[0].kind, "missing");
    assert_eq!(
        error.issues[0].loc,
        [LocItem::from("body"), LocItem::from("count")]
    );
    assert_eq!(error.issues[0].input, json!({}));
    assert_issue(schema, json!([]), "model_attributes_type", Value::Null);
    assert_issue(
        json!({"type":"object"}),
        json!([]),
        "dict_type",
        Value::Null,
    );
}

#[test]
fn additional_properties_can_be_kept_forbidden_or_validated_as_map_values() {
    for additional in [None, Some(json!(true))] {
        let mut schema = json!({"type":"object"});
        if let Some(additional) = additional {
            schema["additionalProperties"] = additional;
        }
        assert_eq!(check(schema, json!({"x":[1]})).unwrap(), json!({"x":[1]}));
    }
    assert_eq!(
        check(
            json!({"type":"object", "properties":{}, "additionalProperties":true}),
            json!({"x":1})
        )
        .unwrap(),
        json!({"x":1})
    );
    let schema = json!({"type":"object", "properties":{"count":{"type":"integer"}},
        "required":["count"], "additionalProperties":false});
    let error = check(schema, json!({"count":"bad", "extra":99})).unwrap_err();
    assert_eq!(error.issues.len(), 2);
    assert_eq!(error.issues[1].kind, "extra_forbidden");
    assert_eq!(
        error.issues[1].loc,
        [LocItem::from("body"), LocItem::from("extra")]
    );
    assert_eq!(error.issues[1].input, json!(99));
    let schema = json!({"type":"object", "additionalProperties":{"type":"integer"}});
    assert_eq!(
        check(schema.clone(), json!({"a":"2", "b":true})).unwrap(),
        json!({"a":2, "b":1})
    );
    let error = check(schema, json!({"a":"bad", "b":null})).unwrap_err();
    assert_eq!(error.issues.len(), 2);
    assert_eq!(
        error.issues[0].loc,
        [LocItem::from("body"), LocItem::from("a")]
    );
    assert_eq!(
        error.issues[1].loc,
        [LocItem::from("body"), LocItem::from("b")]
    );
}

#[test]
fn nested_models_and_arrays_preserve_error_order_and_full_location() {
    let schema = json!({"type":"object", "properties":{
        "rows":{"type":"array", "items":{"type":"object", "properties":{
            "count":{"type":"integer"}, "label":{"type":"string"}
        }, "required":["count","label"]}}, "enabled":{"type":"boolean"}
    }, "required":["rows","enabled"]});
    let error = check(
        schema,
        json!({"rows":[{"count":"bad"}, {"count":1,"label":false}], "enabled":"maybe"}),
    )
    .unwrap_err();
    let body = LocItem::from("body");
    assert_eq!(
        error
            .issues
            .iter()
            .map(|i| (i.kind.as_str(), i.loc.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "int_parsing",
                vec![body.clone(), "rows".into(), 0.into(), "count".into()]
            ),
            (
                "missing",
                vec![body.clone(), "rows".into(), 0.into(), "label".into()]
            ),
            (
                "string_type",
                vec![body.clone(), "rows".into(), 1.into(), "label".into()]
            ),
            ("bool_parsing", vec![body, "enabled".into()]),
        ]
    );
}

#[test]
fn nullable_union_accepts_null_and_reports_non_null_errors_without_branch_tags() {
    for branches in [
        json!([{"type":"integer"},{"type":"null"}]),
        json!([{"type":"null"},{"type":"integer"}]),
    ] {
        let schema = json!({"oneOf":branches});
        assert_eq!(check(schema.clone(), Value::Null).unwrap(), Value::Null);
        assert_eq!(check(schema.clone(), json!("2")).unwrap(), json!(2));
        assert_issue(schema, json!("bad"), "int_parsing", Value::Null);
    }
}

#[test]
fn unions_prefer_matching_json_types_before_coercion_and_discard_failed_attempts() {
    let schema = json!({"oneOf":[{"type":"integer"},{"type":"string"}]});
    assert_eq!(check(schema.clone(), json!("2")).unwrap(), json!("2"));
    assert_eq!(check(schema.clone(), json!(2)).unwrap(), json!(2));
    assert_eq!(check(schema, json!(true)).unwrap(), json!(1));
    let schema = json!({"oneOf":[{"type":"integer","minimum":10},{"type":"number"}]});
    assert_eq!(check(schema, json!(2)).unwrap(), json!(2.0));
}

#[test]
fn failed_unions_report_each_branch_with_its_tag_in_order() {
    let schema = json!({"oneOf":[{"type":"integer"},{"type":"number"},{"type":"string"},
        {"type":"boolean"},{"type":"null"},{"type":"array"},
        {"type":"object","title":"Model","properties":{"field":{"type":"integer"}},"required":["field"]},
        {"type":"object","properties":{"field":{"type":"integer"}},"required":["field"]},
        {"enum":["never"]}]});
    let error = check(schema, json!({})).unwrap_err();
    let expected = [
        ("int", "int_type"),
        ("float", "float_type"),
        ("str", "string_type"),
        ("bool", "bool_type"),
        ("none", "none_required"),
        ("list", "list_type"),
        ("Model", "missing"),
        ("dict", "missing"),
        ("branch-8", "literal_error"),
    ];
    assert_eq!(error.issues.len(), expected.len());
    for (issue, (tag, kind)) in error.issues.iter().zip(expected) {
        let mut loc = vec![LocItem::from("body"), LocItem::from(tag)];
        if kind == "missing" {
            loc.push(LocItem::from("field"));
        }
        assert_eq!(issue.kind, kind);
        assert_eq!(issue.loc, loc);
        assert_eq!(issue.input, json!({}));
    }
}

#[test]
fn enum_constant_and_literal_unions_use_python_numeric_equality() {
    for schema in [
        json!({"enum":[1,2]}),
        json!({"const":1}),
        json!({"oneOf":[{"const":1},{"const":2}]}),
    ] {
        assert_eq!(check(schema.clone(), json!(true)).unwrap(), json!(1));
        assert_eq!(check(schema.clone(), json!(1.0)).unwrap(), json!(1));
        assert_eq!(
            check(schema, json!("1")).unwrap_err().issues[0].kind,
            "literal_error"
        );
    }
    assert_issue(
        json!({"enum":["a","b","c"]}),
        json!("d"),
        "literal_error",
        json!({"expected":"'a', 'b' or 'c'"}),
    );
    assert_issue(
        json!({"enum":[true,false,null]}),
        json!("no"),
        "literal_error",
        json!({"expected":"True, False or None"}),
    );
    assert_issue(
        json!({"const":"fixed"}),
        json!("other"),
        "literal_error",
        json!({"expected":"'fixed'"}),
    );
}

#[test]
fn validation_errors_serialize_and_format_locations_and_context() {
    let error = ValidationError::missing(vec!["body".into(), "rows".into(), 2.into()], Value::Null);
    assert_eq!(
        serde_json::to_value(&error.issues).unwrap(),
        json!([{
            "type":"missing", "loc":["body","rows",2], "msg":"Field required", "input":null
        }])
    );
    assert_eq!(
        error.to_string(),
        "1 validation error\n  body.rows.2: Field required [type=missing]"
    );
    assert_eq!(LocItem::from("field").to_string(), "field");
    assert_eq!(LocItem::from(2).to_string(), "2");
    let root = ValidationError::missing(vec![], json!({}));
    assert_eq!(
        root.to_string(),
        "1 validation error\n  Field required [type=missing]"
    );
    let combined = ValidationError {
        issues: vec![error.issues[0].clone(), root.issues[0].clone()],
    };
    assert!(combined.to_string().starts_with("2 validation errors\n"));
    let error = check(json!({"type":"integer", "minimum":2}), json!(1)).unwrap_err();
    assert_eq!(
        serde_json::to_value(&error.issues[0]).unwrap()["ctx"],
        json!({"ge":2})
    );
}

#[test]
fn serde_conversion_errors_preserve_input_and_location() {
    let input = json!("not an integer");
    let source = serde_json::from_value::<i64>(input.clone()).unwrap_err();
    let error = ValidationError::from_serde(&source, vec!["body".into()], input.clone());
    assert_eq!(error.issues.len(), 1);
    assert_eq!(error.issues[0].kind, "value_error");
    assert_eq!(error.issues[0].loc, [LocItem::from("body")]);
    assert_eq!(error.issues[0].input, input);
    assert_eq!(error.issues[0].msg, format!("Value error, {source}"));
    assert_eq!(error.issues[0].ctx, None);
    assert_eq!(ValidationError::single(error.issues[0].clone()), error);
}
