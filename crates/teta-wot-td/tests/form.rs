//! Integration tests for Thing Description forms.

use serde_json::{Value, json};
use teta_wot_td::{Form, Operation, OperationScope};

#[test]
fn operations_have_the_expected_wire_names_and_scopes() {
    use Operation::*;
    use OperationScope::{Action, Event, Property, Thing};

    let cases = [
        (ReadProperty, "readproperty", Property),
        (WriteProperty, "writeproperty", Property),
        (ObserveProperty, "observeproperty", Property),
        (UnobserveProperty, "unobserveproperty", Property),
        (InvokeAction, "invokeaction", Action),
        (QueryAction, "queryaction", Action),
        (CancelAction, "cancelaction", Action),
        (SubscribeEvent, "subscribeevent", Event),
        (UnsubscribeEvent, "unsubscribeevent", Event),
        (ReadAllProperties, "readallproperties", Thing),
        (WriteAllProperties, "writeallproperties", Thing),
        (ReadMultipleProperties, "readmultipleproperties", Thing),
        (WriteMultipleProperties, "writemultipleproperties", Thing),
        (ObserveAllProperties, "observeallproperties", Thing),
        (UnobserveAllProperties, "unobserveallproperties", Thing),
        (QueryAllActions, "queryallactions", Thing),
        (SubscribeAllEvents, "subscribeallevents", Thing),
        (UnsubscribeAllEvents, "unsubscribeallevents", Thing),
    ];

    for (operation, wire_name, scope) in cases {
        assert_eq!(operation.scope(), scope, "{wire_name}");
        assert_eq!(serde_json::to_value(operation).unwrap(), json!(wire_name));
        assert_eq!(
            serde_json::from_value::<Operation>(json!(wire_name)).unwrap(),
            operation
        );
    }
}

#[test]
fn form_builder_omits_unset_fields_and_preserves_operation_shape() {
    let bare = Form::new("/temperature");
    assert_eq!(
        bare.operations().collect::<Vec<_>>(),
        Vec::<Operation>::new()
    );
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({ "href": "/temperature" })
    );

    let single = Form::new("/temperature").with_op(Operation::ReadProperty);

    assert_eq!(
        single.operations().collect::<Vec<_>>(),
        vec![Operation::ReadProperty]
    );
    assert_eq!(
        serde_json::to_value(&single).unwrap(),
        json!({ "href": "/temperature", "op": "readproperty" })
    );

    let multiple = Form::new("/temperature")
        .with_op([Operation::ReadProperty, Operation::ObserveProperty])
        .with_content_type("application/json")
        .with_subprotocol("sse")
        .with_term("htv:methodName", "GET");
    assert_eq!(
        multiple.operations().collect::<Vec<_>>(),
        vec![Operation::ReadProperty, Operation::ObserveProperty]
    );
    assert_eq!(
        serde_json::to_value(&multiple).unwrap(),
        json!({
            "href": "/temperature",
            "op": ["readproperty", "observeproperty"],
            "contentType": "application/json",
            "subprotocol": "sse",
            "htv:methodName": "GET"
        })
    );
}

#[test]
fn form_round_trips_responses_security_and_extension_terms() {
    let document = json!({
        "href": "https://example.test/actions/reset",
        "op": ["invokeaction", "queryaction"],
        "contentType": "application/json",
        "contentCoding": "gzip",
        "subprotocol": "longpoll",
        "security": ["basic_sc", "bearer_sc"],
        "scopes": "admin",
        "response": {
            "contentType": "text/plain",
            "x-response-term": 7
        },
        "additionalResponses": [{
            "success": false,
            "contentType": "application/problem+json",
            "schema": "problem",
            "x-error-term": true
        }],
        "htv:methodName": "POST"
    });

    let form: Form = serde_json::from_value(document.clone()).unwrap();
    assert_eq!(
        form.operations().collect::<Vec<_>>(),
        vec![Operation::InvokeAction, Operation::QueryAction]
    );
    assert_eq!(
        form.response.as_ref().unwrap().extra["x-response-term"],
        json!(7)
    );
    assert_eq!(
        form.additional_responses.as_ref().unwrap()[0].extra["x-error-term"],
        Value::Bool(true)
    );
    assert_eq!(form.extra["htv:methodName"], json!("POST"));
    assert_eq!(serde_json::to_value(form).unwrap(), document);
}

#[test]
fn form_rejects_missing_href_and_unknown_operation() {
    assert!(serde_json::from_value::<Form>(json!({ "op": "readproperty" })).is_err());
    assert!(
        serde_json::from_value::<Form>(json!({ "href": "/x", "op": "notanoperation" })).is_err()
    );
}
