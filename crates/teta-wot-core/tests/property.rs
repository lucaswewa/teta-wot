//! Integration tests for property cells and client-facing property operations.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use teta_wot_core::{
    BuildError, Cancelled, Constraints, DataProperty, DeviceError, FunctionalProperty,
    GlobalLockBusy, InvocationScope, LocItem, MessageKind, Prop, PropertyEntry, PropertyError,
    Runtime, Thing, ThingDefinition, ValidationError,
};

#[test]
fn prop_reads_updates_and_resets_to_its_original_default() {
    let prop = Prop::new(vec![1, 2]);
    let mut copy = prop.get();
    copy.push(99);
    assert_eq!(prop.read(Vec::len), 2);
    prop.update(|v| v.push(3)).unwrap();
    assert_eq!(prop.get(), [1, 2, 3]);
    assert_eq!(prop.default_value(), &[1, 2]);
    prop.reset().unwrap();
    assert_eq!(prop.get(), [1, 2]);
    assert_eq!(Prop::<i64>::default().get(), 0);
}

#[test]
fn invalid_writes_and_updates_leave_value_and_watchers_unchanged() {
    let constraints = Constraints::new().ge(0).le(10);
    let prop = Prop::new(5_i64).with_constraints(constraints.clone());
    let mut watcher = prop.subscribe();
    assert_eq!(prop.constraints(), &constraints);
    let schema = prop.data_schema().unwrap();
    assert_eq!(schema.minimum, Some(0.into()));
    assert_eq!(schema.maximum, Some(10.into()));
    for result in [prop.set(-1), prop.update(|v| *v = 11)] {
        let PropertyError::Invalid(error) = result.unwrap_err() else {
            panic!("expected validation error");
        };
        assert_eq!(error.issues.len(), 1);
        assert!(error.issues[0].loc.is_empty());
        assert_eq!(prop.get(), 5);
        assert!(!watcher.has_changed().unwrap());
    }
    prop.set(10).unwrap();
    assert!(watcher.has_changed().unwrap());
    assert_eq!(*watcher.borrow_and_update(), 10);
    prop.reset().unwrap();
    assert_eq!(*watcher.borrow_and_update(), 5);
}

#[test]
fn watchers_see_latest_value_and_same_value_writes_notify() {
    let prop = Prop::new(String::from("initial"));
    let mut first = prop.subscribe();
    let mut second = prop.subscribe();
    prop.set("intermediate".into()).unwrap();
    prop.set("latest".into()).unwrap();
    assert_eq!(&*first.borrow_and_update(), "latest");
    assert_eq!(&*second.borrow_and_update(), "latest");
    assert!(!first.has_changed().unwrap());
    prop.set("latest".into()).unwrap();
    assert!(first.has_changed().unwrap());
}

#[test]
fn string_constraints_reject_bad_values_without_mutation() {
    let prop = Prop::new("AB".to_owned()).with_constraints(
        Constraints::new()
            .min_length(2)
            .max_length(4)
            .pattern("^[A-Z]+$"),
    );
    for value in ["A", "ABCDE", "ab"] {
        assert!(matches!(
            prop.set(value.into()),
            Err(PropertyError::Invalid(_))
        ));
        assert_eq!(prop.get(), "AB");
    }
    prop.set("XYZ".into()).unwrap();
    assert_eq!(prop.get(), "XYZ");
}

#[test]
fn invalid_definition_and_invalid_default_reset_are_reported() {
    let bad = Prop::new(1_i64).with_constraints(Constraints::new().pattern("abc"));
    assert!(matches!(
        bad.data_schema(),
        Err(PropertyError::Definition(_))
    ));
    assert!(matches!(bad.set(2), Err(PropertyError::Definition(_))));
    assert_eq!(bad.get(), 1);

    let prop = Prop::new(-1_i64).with_constraints(Constraints::new().ge(0));
    prop.set(2).unwrap();
    assert!(matches!(prop.reset(), Err(PropertyError::Invalid(_))));
    assert_eq!(prop.get(), 2);
}

struct Properties {
    count: Prop<i64>,
    optional: Prop<Option<i64>>,
    caller_thread: std::thread::ThreadId,
}

fn getter() -> FunctionalProperty<Properties, i64> {
    FunctionalProperty::getter(|t: Arc<Properties>| async move { Ok(t.count.get()) })
}

fn writable() -> FunctionalProperty<Properties, i64> {
    getter().setter(|t, value| async move { t.count.set(value) })
}

impl Thing for Properties {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Properties")
            .property(
                "count",
                DataProperty::new(|t: &Self| &t.count)
                    .title("Counter")
                    .description("Current count")
                    .unit("items")
                    .semantic_type("CounterProperty")
                    .semantic_type("Quantity"),
            )
            .property(
                "readonly",
                DataProperty::new(|t: &Self| &t.count).read_only(),
            )
            .property(
                "unlocked",
                DataProperty::new(|t: &Self| &t.count).global_lock(false),
            )
            .property("optional", DataProperty::new(|t: &Self| &t.optional))
            .property(
                "getter",
                getter().doc("Computed count\n\nRead from the cell."),
            )
            .property(
                "writable",
                writable().constraints(Constraints::new().ge(0).le(10)),
            )
            .property("defaulted", writable().default_value(3))
            .property(
                "resetter",
                writable()
                    .default_value(3)
                    .resetter(|t| async move { t.count.set(7) }),
            )
            .property(
                "functional_readonly",
                writable().default_value(3).read_only(),
            )
            .property(
                "resetter_only",
                getter().resetter(|t| async move { t.count.set(7) }),
            )
            .property(
                "blocking",
                FunctionalProperty::blocking_getter(|t: &Self| {
                    assert_ne!(std::thread::current().id(), t.caller_thread);
                    tracing::info!("blocking property log");
                    teta_wot_core::check_cancelled()?;
                    Ok(t.count.get())
                }),
            )
            .property(
                "getter_error",
                FunctionalProperty::getter(|_: Arc<Self>| async {
                    Err::<i64, _>(PropertyError::failed(anyhow::anyhow!("getter failed")))
                }),
            )
            .property(
                "setter_error",
                getter()
                    .setter(|_, _| async {
                        Err(PropertyError::failed(anyhow::anyhow!("setter failed")))
                    })
                    .resetter(|_| async {
                        Err(PropertyError::failed(anyhow::anyhow!("resetter failed")))
                    }),
            )
    }
}

fn fixture(global_lock: bool) -> (Arc<Properties>, Runtime) {
    let thing = Arc::new(Properties {
        count: Prop::new(2).with_constraints(Constraints::new().ge(0).le(10)),
        optional: Prop::new(Some(1)),
        caller_thread: std::thread::current().id(),
    });
    let runtime = Runtime::builder()
        .global_lock(global_lock)
        .global_lock_timeout(Duration::from_millis(10))
        .thing_arc("counter", thing.clone())
        .build()
        .unwrap();
    (thing, runtime)
}

fn entry<'a>(runtime: &'a Runtime, name: &str) -> &'a PropertyEntry {
    runtime.thing("counter").unwrap().property(name).unwrap()
}

fn assert_invalid(result: Result<(), PropertyError>, value: Value, kind: &str) {
    let PropertyError::Invalid(error) = result.unwrap_err() else {
        panic!("expected validation error");
    };
    assert_eq!(error.issues.len(), 1);
    let issue = &error.issues[0];
    assert_eq!(issue.loc, [LocItem::from("body")]);
    assert_eq!(issue.input, value);
    assert_eq!(issue.kind, kind);
}

#[tokio::test]
async fn data_property_metadata_and_client_writes_match_the_cell() {
    let (thing, runtime) = fixture(false);
    let property = entry(&runtime, "count");
    assert_eq!(property.name(), "count");
    assert_eq!(property.title(), "Counter");
    assert_eq!(property.description(), Some("Current count"));
    assert_eq!(property.unit(), Some("items"));
    assert_eq!(property.semantic_types(), ["CounterProperty", "Quantity"]);
    assert!(!property.is_read_only());
    assert!(property.is_observable());
    assert!(property.is_resettable());
    assert_eq!(property.default_value(), Some(json!(2)));
    assert_eq!(property.data_schema().maximum, Some(10.into()));
    assert_eq!(property.read().await.unwrap(), json!(2));
    property.write(json!("6")).await.unwrap();
    assert_eq!(thing.count.get(), 6);
    assert_eq!(property.clone().read().await.unwrap(), json!(6));
    property.reset().await.unwrap();
    assert_eq!(thing.count.get(), 2);
}

#[tokio::test]
async fn client_validation_rejects_null_and_bad_values_before_mutating() {
    let (thing, runtime) = fixture(false);
    for name in ["count", "writable"] {
        let property = entry(&runtime, name);
        assert_invalid(property.write(Value::Null).await, Value::Null, "missing");
        assert_invalid(
            property.write(json!(11)).await,
            json!(11),
            "less_than_equal",
        );
        assert_invalid(
            property.write(json!("nope")).await,
            json!("nope"),
            "int_parsing",
        );
        assert_eq!(thing.count.get(), 2);
    }
    // Null is treated as an absent request body even when the Rust type allows it.
    assert_invalid(
        entry(&runtime, "optional").write(Value::Null).await,
        Value::Null,
        "missing",
    );
    assert_eq!(thing.optional.get(), Some(1));
    thing.optional.set(None).unwrap();
    assert_eq!(
        entry(&runtime, "optional").read().await.unwrap(),
        Value::Null
    );
}

#[tokio::test]
async fn readonly_properties_refuse_client_writes_and_resets_but_allow_rust_updates() {
    let (thing, runtime) = fixture(false);
    for name in ["readonly", "getter", "functional_readonly", "resetter_only"] {
        let property = entry(&runtime, name);
        assert!(property.is_read_only());
        assert!(!property.is_resettable());
        assert!(matches!(property.write(json!(4)).await,
            Err(PropertyError::ReadOnly(n)) if n == name));
        assert!(matches!(property.reset().await,
            Err(PropertyError::ReadOnly(n)) if n == name));
    }
    assert_eq!(thing.count.get(), 2);
    thing.count.set(5).unwrap();
    assert_eq!(entry(&runtime, "readonly").read().await.unwrap(), json!(5));
}

#[tokio::test]
async fn bound_data_properties_publish_rust_and_client_changes() {
    let (thing, runtime) = fixture(false);
    let mut subscription = runtime.broker().subscribe("counter", "count");
    let mut other = runtime.broker().subscribe("other", "count");
    let mut watcher = thing.count.subscribe();
    assert!(subscription.try_recv().is_none());
    for value in [4, 6, 2] {
        match value {
            4 => thing.count.set(value).unwrap(),
            6 => entry(&runtime, "count").write(json!(value)).await.unwrap(),
            _ => entry(&runtime, "count").reset().await.unwrap(),
        }
        let message = subscription.try_recv().expect("change notification");
        assert_eq!(message.thing, "counter");
        assert_eq!(message.affordance, "count");
        assert_eq!(message.kind, MessageKind::Property);
        assert_eq!(message.payload, json!(value));
        assert_eq!(*watcher.borrow_and_update(), value);
        assert!(other.try_recv().is_none());
    }
    assert!(matches!(
        thing.count.set(11),
        Err(PropertyError::Invalid(_))
    ));
    assert!(matches!(
        entry(&runtime, "count").write(json!(-1)).await,
        Err(PropertyError::Invalid(_))
    ));
    assert!(subscription.try_recv().is_none());
    assert!(!watcher.has_changed().unwrap());
}

#[tokio::test]
async fn functional_properties_use_setters_defaults_and_explicit_resetters() {
    let (thing, runtime) = fixture(false);
    let property = entry(&runtime, "getter");
    assert_eq!(property.title(), "Computed count");
    assert_eq!(property.description(), Some("Read from the cell."));
    assert_eq!(property.default_value(), None);
    assert!(!property.is_observable());
    let writable = entry(&runtime, "writable");
    assert!(!writable.is_read_only());
    assert!(!writable.is_resettable());
    writable.write(json!("8")).await.unwrap();
    assert_eq!(writable.read().await.unwrap(), json!(8));
    assert!(matches!(writable.reset().await,
        Err(PropertyError::NotResettable(name)) if name == "writable"));
    let defaulted = entry(&runtime, "defaulted");
    assert!(defaulted.is_resettable());
    assert_eq!(defaulted.default_value(), Some(json!(3)));
    defaulted.reset().await.unwrap();
    assert_eq!(thing.count.get(), 3);
    entry(&runtime, "resetter").reset().await.unwrap();
    assert_eq!(thing.count.get(), 7); // Explicit resetter takes precedence over default 3.
}

#[tokio::test]
async fn functional_getter_setter_and_resetter_errors_reach_the_caller() {
    let (thing, runtime) = fixture(false);
    for (result, expected) in [
        (
            entry(&runtime, "getter_error").read().await.map(|_| ()),
            "getter failed",
        ),
        (
            entry(&runtime, "setter_error").write(json!(4)).await,
            "setter failed",
        ),
        (
            entry(&runtime, "setter_error").reset().await,
            "resetter failed",
        ),
    ] {
        assert!(
            matches!(result, Err(PropertyError::Failed(error)) if error.to_string() == expected)
        );
    }
    assert_eq!(thing.count.get(), 2);
}

#[tokio::test]
async fn blocking_getters_preserve_invocation_cancellation_and_logging() {
    teta_wot_core::testing::init_tracing();
    let (_, runtime) = fixture(false);
    let scope = InvocationScope::fake();
    scope.cancel_token().cancel();
    let property = entry(&runtime, "blocking");
    assert!(matches!(
        scope.clone().run(property.read()).await,
        Err(PropertyError::Cancelled(_))
    ));
    assert!(!scope.cancel_token().is_cancelled());
    assert!(
        scope
            .logs()
            .iter()
            .any(|log| log.message == "blocking property log")
    );
    assert_eq!(property.read().await.unwrap(), json!(2));
}

#[tokio::test(start_paused = true)]
async fn global_lock_blocks_writes_and_resets_but_allows_reads_and_opt_out() {
    let (thing, runtime) = fixture(true);
    let scope = InvocationScope::fake();
    let lock = runtime.global_lock().unwrap();
    let guard = lock.try_acquire(scope.lock_owner()).unwrap();
    for name in ["count", "defaulted"] {
        let property = entry(&runtime, name);
        assert!(matches!(
            property.write(json!(4)).await,
            Err(PropertyError::GlobalLockBusy(_))
        ));
        assert!(matches!(
            property.reset().await,
            Err(PropertyError::GlobalLockBusy(_))
        ));
        assert_eq!(property.read().await.unwrap(), json!(2));
    }
    assert_eq!(thing.count.get(), 2);
    entry(&runtime, "unlocked").write(json!(4)).await.unwrap();
    entry(&runtime, "unlocked").reset().await.unwrap();
    scope
        .clone()
        .run(async {
            entry(&runtime, "count").write(json!(5)).await.unwrap();
            entry(&runtime, "count").reset().await.unwrap();
        })
        .await;
    assert_eq!(lock.owner(), Some(scope.lock_owner()));
    drop(guard);
    entry(&runtime, "count").write(json!(6)).await.unwrap();
    assert_eq!(lock.owner(), None);
    assert_invalid(
        entry(&runtime, "count").write(json!(11)).await,
        json!(11),
        "less_than_equal",
    );
    assert_eq!(lock.owner(), None);
}

struct BadData(Prop<i64>);
impl Thing for BadData {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Bad data").property("bad", DataProperty::new(|t: &Self| &t.0))
    }
}

struct BadFunctional;
impl Thing for BadFunctional {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Bad functional").property(
            "bad",
            FunctionalProperty::getter(|_: Arc<Self>| async { Ok(1_i64) })
                .constraints(Constraints::new().pattern("x")),
        )
    }
}

#[test]
fn invalid_property_constraints_fail_runtime_build_with_the_affordance_name() {
    for result in [
        Runtime::builder()
            .thing(
                "broken",
                BadData(Prop::new(0).with_constraints(Constraints::new().pattern("x"))),
            )
            .build(),
        Runtime::builder().thing("broken", BadFunctional).build(),
    ] {
        let BuildError::Definition(error) = result.unwrap_err() else {
            panic!("expected definition error");
        };
        assert_eq!(error.thing, "broken");
        assert_eq!(error.affordance, "bad");
        assert!(!error.message.is_empty());
    }
}

#[test]
fn property_errors_produce_expected_problem_details() {
    let invalid = ValidationError::missing(vec![LocItem::from("body")], Value::Null);
    for (error, title, status, typed) in [
        (
            PropertyError::Invalid(invalid),
            "ValidationError",
            500,
            false,
        ),
        (
            PropertyError::ReadOnly("count".into()),
            "ReadOnlyPropertyError",
            500,
            false,
        ),
        (
            PropertyError::NotResettable("count".into()),
            "FeatureNotAvailableError",
            500,
            false,
        ),
        (
            PropertyError::Device(DeviceError::NotOpen("motor".into())),
            "DeviceError",
            500,
            false,
        ),
        (
            PropertyError::Definition("bad".into()),
            "PropertyDefinitionError",
            500,
            false,
        ),
        (
            PropertyError::from(anyhow::anyhow!("failed")),
            "PropertyError",
            500,
            false,
        ),
        (
            PropertyError::from(GlobalLockBusy),
            "GlobalLockBusyError",
            409,
            true,
        ),
        (
            PropertyError::from(Cancelled),
            "InvocationCancelledError",
            500,
            true,
        ),
    ] {
        let problem = error.problem();
        assert_eq!(problem.title.as_deref(), Some(title));
        assert_eq!(problem.status, Some(status));
        assert_eq!(problem.detail, Some(error.to_string()));
        assert_eq!(
            problem.problem_type,
            typed.then(|| teta_wot_core::problem::teta_wot_exception_type(title))
        );
        assert_eq!(problem.instance, None);
    }
}
