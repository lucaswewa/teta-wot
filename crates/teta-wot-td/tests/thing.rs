//! Integration tests for Thing Description building and validation.

use serde_json::json;
use teta_wot_td::{
    ActionAffordance, Context, ContextEntry, DataSchema, DataType, EventAffordance, Form, Link,
    NO_SECURITY_NAME, Operation, OperationScope, PropertyAffordance, SecurityScheme, TD_CONTEXT_V1,
    TD_CONTEXT_V1_1, TdError, ThingDescription,
};

#[test]
fn minimal_builder_adds_no_security_and_serializes_required_fields() {
    let td = ThingDescription::builder("Example Thing").build().unwrap();
    assert_eq!(NO_SECURITY_NAME, "no_security");

    assert_eq!(
        serde_json::to_value(&td).unwrap(),
        json!({
            "title": "Example Thing",
            "properties": {},
            "actions": {},
            "securityDefinitions": {
                "no_security": { "scheme": "nosec", "description": "No security" }
            },
            "security": "no_security",
            "@context": TD_CONTEXT_V1_1
        })
    );
    assert!(td.validate().is_ok());
}

#[test]
fn populated_builder_round_trips_through_json() {
    let property = PropertyAffordance::builder(DataSchema::of(DataType::Number))
        .form(Form::new("/temperature").with_op(Operation::ReadProperty));
    let action =
        ActionAffordance::builder().form(Form::new("/reset").with_op(Operation::InvokeAction));
    let event =
        EventAffordance::builder().form(Form::new("/alarm").with_op(Operation::SubscribeEvent));

    let td = ThingDescription::builder("Sensor")
        .id("urn:example:sensor")
        .description("A lab sensor")
        .base("https://example.test/")
        .version("2.0")
        .semantic_type("ex:Sensor")
        .semantic_type("ex:Device")
        .context_entry(ContextEntry::from("https://example.test/context"))
        .context_prefix("ex", "https://example.test/vocab#")
        .property("temperature", property)
        .action("reset", action)
        .event("alarm", event)
        .link(Link::new("/manual").with_rel("help"))
        .form(Form::new("/properties").with_op(Operation::ReadAllProperties))
        .term("ex:vendor", "Example Labs")
        .build()
        .unwrap();

    let document = serde_json::to_value(&td).unwrap();
    assert_eq!(document["id"], "urn:example:sensor");
    assert_eq!(document["version"]["instance"], "2.0");
    assert_eq!(document["@type"], json!(["ex:Sensor", "ex:Device"]));
    assert_eq!(
        document["@context"],
        json!([
            TD_CONTEXT_V1_1,
            "https://example.test/context",
            { "ex": "https://example.test/vocab#" }
        ])
    );
    assert_eq!(document["properties"]["temperature"]["type"], "number");
    assert_eq!(
        document["actions"]["reset"]["forms"][0]["op"],
        "invokeaction"
    );
    assert_eq!(
        document["events"]["alarm"]["forms"][0]["op"],
        "subscribeevent"
    );
    assert_eq!(document["links"][0]["rel"], "help");
    assert_eq!(document["forms"][0]["op"], "readallproperties");
    assert_eq!(document["ex:vendor"], "Example Labs");

    let decoded: ThingDescription = serde_json::from_value(document).unwrap();
    assert_eq!(decoded, td);
    assert!(decoded.validate().is_ok());
}

#[test]
fn security_definitions_and_references_are_checked() {
    assert_eq!(
        ThingDescription::builder("Thing")
            .security_definition("basic_sc", SecurityScheme::basic())
            .build()
            .unwrap_err(),
        TdError::MissingSecurity
    );

    assert_eq!(
        ThingDescription::builder("Thing")
            .security_definition("basic_sc", SecurityScheme::basic())
            .security("unknown")
            .build()
            .unwrap_err(),
        TdError::UndefinedSecurity("unknown".into())
    );

    assert_eq!(
        ThingDescription::builder("Thing")
            .security_definition("combo_sc", SecurityScheme::combo_one_of(["missing_sc"]))
            .security("combo_sc")
            .build()
            .unwrap_err(),
        TdError::UndefinedSecurity("missing_sc".into())
    );

    let mut form = Form::new("/all").with_op(Operation::ReadAllProperties);
    form.security = Some("missing_sc".into());
    assert_eq!(
        ThingDescription::builder("Thing")
            .form(form)
            .build()
            .unwrap_err(),
        TdError::UndefinedSecurity("missing_sc".into())
    );

    let mut td = ThingDescription::builder("Thing").build().unwrap();
    td.security_definitions.clear();
    assert_eq!(td.validate(), Err(TdError::NoSecurityDefinitions));
}

#[test]
fn explicitly_selected_security_scheme_builds() {
    let td = ThingDescription::builder("Thing")
        .security_definition("basic_sc", SecurityScheme::basic())
        .security("basic_sc")
        .build()
        .unwrap();

    assert!(td.validate().is_ok());
    assert_eq!(serde_json::to_value(td).unwrap()["security"], "basic_sc");
}

#[test]
fn duplicate_names_are_rejected_within_each_affordance_kind() {
    let property = || {
        PropertyAffordance::builder(DataSchema::of(DataType::Number))
            .form(Form::new("/p").with_op(Operation::ReadProperty))
    };
    assert_eq!(
        ThingDescription::builder("Thing")
            .property("p", property())
            .property("p", property())
            .build()
            .unwrap_err(),
        TdError::DuplicateAffordance {
            scope: OperationScope::Property,
            name: "p".into(),
        }
    );

    let action =
        || ActionAffordance::builder().form(Form::new("/a").with_op(Operation::InvokeAction));
    assert_eq!(
        ThingDescription::builder("Thing")
            .action("a", action())
            .action("a", action())
            .build()
            .unwrap_err(),
        TdError::DuplicateAffordance {
            scope: OperationScope::Action,
            name: "a".into(),
        }
    );

    let event =
        || EventAffordance::builder().form(Form::new("/e").with_op(Operation::SubscribeEvent));
    assert_eq!(
        ThingDescription::builder("Thing")
            .event("e", event())
            .event("e", event())
            .build()
            .unwrap_err(),
        TdError::DuplicateAffordance {
            scope: OperationScope::Event,
            name: "e".into(),
        }
    );
}

#[test]
fn affordances_need_nonempty_names_and_forms() {
    assert_eq!(
        ThingDescription::builder("Thing")
            .property(
                "",
                PropertyAffordance::builder(DataSchema::of(DataType::Number)).form(Form::new("/p")),
            )
            .build()
            .unwrap_err(),
        TdError::EmptyName {
            scope: OperationScope::Property,
        }
    );
    assert_eq!(
        ThingDescription::builder("Thing")
            .property("p", PropertyAffordance::builder(DataSchema::any()))
            .build()
            .unwrap_err(),
        TdError::NoForms {
            scope: OperationScope::Property,
            name: "p".into(),
        }
    );
    assert_eq!(
        ThingDescription::builder("Thing")
            .action("a", ActionAffordance::builder())
            .build()
            .unwrap_err(),
        TdError::NoForms {
            scope: OperationScope::Action,
            name: "a".into(),
        }
    );
    assert_eq!(
        ThingDescription::builder("Thing")
            .event("e", EventAffordance::builder())
            .build()
            .unwrap_err(),
        TdError::NoForms {
            scope: OperationScope::Event,
            name: "e".into(),
        }
    );
}

#[test]
fn operation_scope_is_checked_for_affordance_and_top_level_forms() {
    let cases = [
        (
            ThingDescription::builder("Thing")
                .property(
                    "p",
                    PropertyAffordance::builder(DataSchema::any())
                        .form(Form::new("/p").with_op(Operation::InvokeAction)),
                )
                .build(),
            OperationScope::Property,
            "p",
            Operation::InvokeAction,
        ),
        (
            ThingDescription::builder("Thing")
                .action(
                    "a",
                    ActionAffordance::builder()
                        .form(Form::new("/a").with_op(Operation::ReadProperty)),
                )
                .build(),
            OperationScope::Action,
            "a",
            Operation::ReadProperty,
        ),
        (
            ThingDescription::builder("Thing")
                .event(
                    "e",
                    EventAffordance::builder()
                        .form(Form::new("/e").with_op(Operation::InvokeAction)),
                )
                .build(),
            OperationScope::Event,
            "e",
            Operation::InvokeAction,
        ),
        (
            ThingDescription::builder("Thing")
                .form(Form::new("/all").with_op(Operation::SubscribeEvent))
                .build(),
            OperationScope::Thing,
            "",
            Operation::SubscribeEvent,
        ),
    ];

    for (result, scope, name, op) in cases {
        assert_eq!(
            result.unwrap_err(),
            TdError::MisplacedOperation {
                scope,
                name: name.into(),
                op,
            }
        );
    }
}

#[test]
fn direct_document_validation_checks_context() {
    let mut td = ThingDescription::builder("Thing").build().unwrap();
    td.context = Context::Uri(TD_CONTEXT_V1.into());

    assert!(matches!(td.validate(), Err(TdError::InvalidContext(_))));
}
