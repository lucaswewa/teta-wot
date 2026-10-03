//! Integration tests for runtime registration, lifecycle, and action execution.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use teta_wot_core::{
    Action, ActionCtx, ActionError, BuildError, Constraints, DataProperty, Device, DeviceState,
    Driver, FunctionalProperty, InvocationScope, InvocationStatus, LocItem, MessageKind, NoInput,
    Prop, Runtime, TdOptions, Thing, ThingCtx, ThingDefinition,
};
use tokio::sync::Notify;

struct Empty;
impl Thing for Empty {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Empty")
    }
}

#[tokio::test]
async fn empty_runtime_has_no_things_or_lock_and_lifecycle_is_harmless() {
    let runtime = Runtime::builder().build().unwrap();
    assert_eq!(runtime.things().count(), 0);
    assert!(runtime.thing("missing").is_none());
    assert!(runtime.global_lock().is_none());
    assert!(runtime.invocations().list().is_empty());
    runtime.stop().await;
    runtime.start().await.unwrap();
    runtime.shutdown(Duration::ZERO).await;
}

#[test]
fn builder_checks_names_and_preserves_registration_order() {
    for name in ["", "with space", "slash/name", "a.b", "café"] {
        assert!(matches!(Runtime::builder().thing(name, Empty).build(),
            Err(BuildError::InvalidName(actual)) if actual == name));
    }
    let runtime = Runtime::builder()
        .thing("z_1-A", Empty)
        .thing("a", Empty)
        .build()
        .unwrap();
    assert_eq!(
        runtime.things().map(|t| t.name()).collect::<Vec<_>>(),
        ["z_1-A", "a"]
    );
    assert!(
        matches!(Runtime::builder().thing("same", Empty).thing("same", Empty).build(),
        Err(BuildError::DuplicateThing(name)) if name == "same")
    );
}

struct Duplicate<const MODE: u8>;
impl<const MODE: u8> Thing for Duplicate<MODE> {
    fn definition() -> ThingDefinition<Self> {
        let property = || FunctionalProperty::getter(|_: Arc<Self>| async { Ok(1_i64) });
        let action = || {
            Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async { Ok::<_, ActionError>(()) })
        };
        let definition = ThingDefinition::new("Duplicate");
        match MODE {
            0 => definition
                .property("same", property())
                .property("same", property()),
            1 => definition.action("same", action()).action("same", action()),
            _ => definition
                .property("same", property())
                .action("same", action()),
        }
    }
}

#[test]
fn duplicate_affordances_are_rejected_within_and_across_kinds() {
    for result in [
        Runtime::builder().thing("thing", Duplicate::<0>).build(),
        Runtime::builder().thing("thing", Duplicate::<1>).build(),
        Runtime::builder().thing("thing", Duplicate::<2>).build(),
    ] {
        assert!(
            matches!(result, Err(BuildError::DuplicateAffordance { thing, name })
            if thing == "thing" && name == "same")
        );
    }
}

type Events = Arc<Mutex<Vec<String>>>;
struct RecordingDriver {
    label: String,
    events: Events,
    fail_open: bool,
    fail_close: bool,
}
impl Driver for RecordingDriver {
    fn open(&mut self) -> anyhow::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("open:{}", self.label));
        anyhow::ensure!(!self.fail_open, "open failed");
        Ok(())
    }
    fn close(&mut self) -> anyhow::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("close:{}", self.label));
        anyhow::ensure!(!self.fail_close, "close failed");
        Ok(())
    }
}

struct LifecycleThing {
    first: Device<RecordingDriver>,
    second: Device<RecordingDriver>,
    events: Events,
    fail_start: bool,
}
impl LifecycleThing {
    fn new(
        label: &str,
        events: &Events,
        fail_start: bool,
        fail_open: bool,
        fail_close: bool,
    ) -> Self {
        let driver = |suffix| {
            let events = events.clone();
            let label = format!("{label}:{suffix}");
            Device::new(move || {
                Ok(RecordingDriver {
                    label: label.clone(),
                    events: events.clone(),
                    fail_open: fail_open && suffix == "second",
                    fail_close,
                })
            })
        };
        Self {
            first: driver("first"),
            second: driver("second"),
            events: events.clone(),
            fail_start,
        }
    }
}
impl Thing for LifecycleThing {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Lifecycle")
            .device("first", |t: &Self| &t.first)
            .device("second", |t: &Self| &t.second)
    }
    async fn start(self: Arc<Self>, ctx: ThingCtx) -> anyhow::Result<()> {
        assert_eq!(self.first.state(), DeviceState::Ready);
        assert_eq!(self.second.state(), DeviceState::Ready);
        assert_eq!(self.first.name(), format!("{}:first", ctx.name()));
        self.events
            .lock()
            .unwrap()
            .push(format!("start:{}", ctx.name()));
        anyhow::ensure!(!self.fail_start, "start failed");
        Ok(())
    }
    async fn stop(self: Arc<Self>, ctx: ThingCtx) {
        assert_eq!(self.first.state(), DeviceState::Ready);
        assert_eq!(self.second.state(), DeviceState::Ready);
        self.events
            .lock()
            .unwrap()
            .push(format!("stop:{}", ctx.name()));
    }
}

#[tokio::test]
async fn lifecycle_opens_devices_before_start_and_stops_in_reverse_order() {
    let events = Events::default();
    let first = Arc::new(LifecycleThing::new("a", &events, false, false, false));
    let second = Arc::new(LifecycleThing::new("b", &events, false, false, true));
    let runtime = Runtime::builder()
        .thing_arc("a", first.clone())
        .thing_arc("b", second.clone())
        .build()
        .unwrap();
    assert!(events.lock().unwrap().is_empty());
    runtime.start().await.unwrap();
    runtime.stop().await;
    runtime.stop().await;
    assert_eq!(
        *events.lock().unwrap(),
        [
            "open:a:first",
            "open:a:second",
            "start:a",
            "open:b:first",
            "open:b:second",
            "start:b",
            "stop:b",
            "close:b:second",
            "close:b:first",
            "stop:a",
            "close:a:second",
            "close:a:first",
        ]
    );
    assert_eq!(first.first.state(), DeviceState::Closed);
    assert_eq!(second.second.state(), DeviceState::Closed);
}

#[tokio::test]
async fn startup_failure_closes_partial_devices_and_rolls_back_started_things() {
    for fail_open in [false, true] {
        let events = Events::default();
        let runtime = Runtime::builder()
            .thing("a", LifecycleThing::new("a", &events, false, false, false))
            .thing(
                "b",
                LifecycleThing::new("b", &events, !fail_open, fail_open, false),
            )
            .thing("c", LifecycleThing::new("c", &events, false, false, false))
            .build()
            .unwrap();
        let error = runtime.start().await.unwrap_err();
        assert_eq!(error.thing, "b");
        assert!(error.error.to_string().contains(if fail_open {
            "open failed"
        } else {
            "start failed"
        }));
        runtime.stop().await;
        let mut expected = vec![
            "open:a:first",
            "open:a:second",
            "start:a",
            "open:b:first",
            "open:b:second",
        ];
        if !fail_open {
            expected.extend(["start:b", "close:b:second"]);
        }
        expected.extend(["close:b:first", "stop:a", "close:a:second", "close:a:first"]);
        assert_eq!(*events.lock().unwrap(), expected);
        for name in ["a", "b", "c"] {
            let thing = runtime
                .thing(name)
                .unwrap()
                .instance::<LifecycleThing>()
                .unwrap();
            assert_eq!(thing.first.state(), DeviceState::Closed);
            assert_eq!(thing.second.state(), DeviceState::Closed);
        }
    }
}

struct Actions {
    value: Prop<i64>,
    calls: AtomicUsize,
    stops: AtomicUsize,
    started: Notify,
    release: Notify,
}
impl Default for Actions {
    fn default() -> Self {
        Self {
            value: Prop::new(2),
            calls: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            started: Notify::new(),
            release: Notify::new(),
        }
    }
}
impl Thing for Actions {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Action fixture")
            .description("Runtime test fixture.")
            .property(
                "z_value",
                DataProperty::new(|t: &Self| &t.value)
                    .title("Value")
                    .description("Stored value")
                    .unit("items")
                    .semantic_type("Quantity"),
            )
            .property(
                "a_readonly",
                DataProperty::new(|t: &Self| &t.value).read_only(),
            )
            .action(
                "z_echo",
                Action::new(|t: Arc<Self>, ctx: ActionCtx, input: i64| async move {
                    assert_eq!(ctx.thing_name(), "actions");
                    assert_eq!(
                        InvocationScope::current().unwrap().id(),
                        ctx.invocation_id()
                    );
                    t.calls.fetch_add(1, Ordering::SeqCst);
                    tracing::info!("echo action log");
                    Ok::<_, ActionError>(input + 1)
                })
                .title("Echo")
                .description("Adds one")
                .retention(Duration::from_secs(5))
                .semantic_type("EchoAction"),
            )
            .action(
                "a_noop",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    Ok::<_, ActionError>(())
                }),
            )
            .action(
                "optional",
                Action::new(
                    |_: Arc<Self>, _: ActionCtx, input: Option<i64>| async move {
                        Ok::<_, ActionError>(input)
                    },
                ),
            )
            .action(
                "unlocked",
                Action::new(|t: Arc<Self>, _: ActionCtx, _: NoInput| async move {
                    t.calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ActionError>(())
                })
                .global_lock(false),
            )
            .action(
                "handled",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    Err::<(), _>(ActionError::handled("anticipated failure"))
                }),
            )
            .action(
                "failed",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    Err::<(), ActionError>(anyhow::anyhow!("unexpected failure").into())
                }),
            )
            .action(
                "panicked",
                Action::new(|_: Arc<Self>, _: ActionCtx, _: NoInput| async {
                    panic!("action panic");
                    #[allow(unreachable_code)]
                    Ok::<(), ActionError>(())
                }),
            )
            .action(
                "wait",
                Action::new(|t: Arc<Self>, ctx: ActionCtx, _: NoInput| async move {
                    t.started.notify_one();
                    Err::<(), ActionError>(ctx.cancelled().await.into())
                }),
            )
            .action(
                "stubborn",
                Action::new(|t: Arc<Self>, _: ActionCtx, _: NoInput| async move {
                    t.started.notify_one();
                    t.release.notified().await;
                    Ok::<_, ActionError>(())
                }),
            )
    }
    async fn stop(self: Arc<Self>, _: ThingCtx) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

fn fixture(locked: bool) -> (Arc<Actions>, Runtime) {
    let thing = Arc::new(Actions::default());
    let runtime = Runtime::builder()
        .global_lock(locked)
        .global_lock_timeout(Duration::from_millis(10))
        .thing_arc("actions", thing.clone())
        .build()
        .unwrap();
    (thing, runtime)
}

#[test]
fn thing_handles_expose_metadata_ordered_affordances_and_original_instance() {
    let (thing, runtime) = fixture(false);
    let handle = runtime.thing("actions").unwrap();
    assert_eq!(handle.name(), "actions");
    assert_eq!(handle.title(), "Action fixture");
    assert_eq!(handle.description(), Some("Runtime test fixture."));
    assert!(Arc::ptr_eq(&handle.instance::<Actions>().unwrap(), &thing));
    assert!(handle.instance::<Empty>().is_none());
    assert!(handle.property("missing").is_none());
    assert!(handle.action("missing").is_none());
    assert_eq!(
        handle.properties().map(|p| p.name()).collect::<Vec<_>>(),
        ["z_value", "a_readonly"]
    );
    assert_eq!(
        handle.actions().map(|a| a.name()).collect::<Vec<_>>(),
        [
            "z_echo", "a_noop", "optional", "unlocked", "handled", "failed", "panicked", "wait",
            "stubborn"
        ]
    );
    let action = handle.action("z_echo").unwrap();
    assert_eq!(action.title(), "Echo");
    assert_eq!(action.description(), Some("Adds one"));
    assert_eq!(action.retention(), Duration::from_secs(5));
    assert_eq!(
        serde_json::to_value(action.input_schema()).unwrap()["type"],
        "integer"
    );
    assert_eq!(
        serde_json::to_value(action.output_schema()).unwrap()["type"],
        "integer"
    );
}

#[test]
fn global_lock_configuration_is_applied() {
    let runtime = Runtime::builder().global_lock(true).build().unwrap();
    assert_eq!(
        runtime.global_lock().unwrap().timeout(),
        Duration::from_millis(50)
    );
    let (_, configured) = fixture(true);
    assert_eq!(
        configured.global_lock().unwrap().timeout(),
        Duration::from_millis(10)
    );
}

#[tokio::test]
async fn action_invocation_registers_validated_input_context_output_and_notifications() {
    teta_wot_core::testing::init_tracing();
    let (thing, runtime) = fixture(false);
    let mut subscription = runtime.broker().subscribe("actions", "z_echo");
    let action = runtime
        .thing("actions")
        .unwrap()
        .action("z_echo")
        .unwrap()
        .clone();
    let invocation = action.invoke(json!("4")).unwrap();
    assert_eq!(invocation.status(), InvocationStatus::Pending);
    assert_eq!(invocation.input(), &json!(4));
    assert!(Arc::ptr_eq(
        &runtime.invocations().get(invocation.id()).unwrap(),
        &invocation
    ));
    assert_eq!(invocation.wait().await, InvocationStatus::Completed);
    assert_eq!(invocation.output(), Some(json!(5)));
    assert_eq!(invocation.error(), None);
    assert_eq!(thing.calls.load(Ordering::SeqCst), 1);
    assert!(
        invocation
            .logs()
            .iter()
            .any(|log| log.message == "echo action log")
    );
    for status in ["pending", "running", "completed"] {
        let message = subscription.try_recv().unwrap();
        assert_eq!(message.thing, "actions");
        assert_eq!(message.affordance, "z_echo");
        assert_eq!(message.kind, MessageKind::Action);
        assert_eq!(message.payload, json!(status));
    }
    let record = invocation.record();
    assert!(record.time_started.is_some());
    assert!(record.time_completed >= record.time_started);
}

#[tokio::test]
async fn invalid_action_input_does_not_register_or_run_an_invocation() {
    let (thing, runtime) = fixture(false);
    let action = runtime.thing("actions").unwrap().action("z_echo").unwrap();
    let mut subscription = runtime.broker().subscribe("actions", "z_echo");
    for (input, kind) in [(Value::Null, "missing"), (json!("bad"), "int_parsing")] {
        let error = action.invoke(input.clone()).unwrap_err();
        assert_eq!(error.issues.len(), 1);
        assert_eq!(error.issues[0].kind, kind);
        assert_eq!(error.issues[0].loc, [LocItem::from("body")]);
        assert_eq!(error.issues[0].input, input);
    }
    assert!(runtime.invocations().list().is_empty());
    assert!(subscription.try_recv().is_none());
    assert_eq!(thing.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_input_and_nullable_actions_accept_null_bodies() {
    let (_, runtime) = fixture(false);
    let handle = runtime.thing("actions").unwrap();
    for (name, input) in [
        ("a_noop", Value::Null),
        ("a_noop", json!({})),
        ("optional", Value::Null),
    ] {
        let invocation = handle.action(name).unwrap().invoke(input).unwrap();
        assert_eq!(invocation.wait().await, InvocationStatus::Completed);
        assert_eq!(invocation.output(), Some(Value::Null));
    }
}

#[tokio::test]
async fn action_errors_and_panics_finish_as_errors_with_the_correct_logs() {
    teta_wot_core::testing::init_tracing();
    let (_, runtime) = fixture(false);
    for (name, title, detail, handled) in [
        ("handled", "InvocationError", "anticipated failure", true),
        ("failed", "Error", "unexpected failure", false),
        (
            "panicked",
            "Panicked",
            "The action panicked: action panic",
            false,
        ),
    ] {
        let invocation = runtime
            .thing("actions")
            .unwrap()
            .action(name)
            .unwrap()
            .invoke(Value::Null)
            .unwrap();
        assert_eq!(invocation.wait().await, InvocationStatus::Error);
        assert_eq!(invocation.output(), None);
        let problem = invocation.error().unwrap();
        assert_eq!(problem.title.as_deref(), Some(title));
        assert_eq!(problem.detail.as_deref(), Some(detail));
        let logs = invocation.logs();
        let log = logs.iter().find(|log| log.message == detail).unwrap();
        assert_eq!(log.levelname, "ERROR");
        assert_eq!(log.traceback.is_none(), handled);
        assert_eq!(log.exception_type.is_none(), handled);
    }
}

#[tokio::test(start_paused = true)]
async fn lock_contention_rejects_actions_before_running_and_opt_out_succeeds() {
    teta_wot_core::testing::init_tracing();
    for (level, name) in [
        (tracing::Level::ERROR, "ERROR"),
        (tracing::Level::WARN, "WARNING"),
        (tracing::Level::INFO, "INFO"),
    ] {
        let thing = Arc::new(Actions::default());
        let runtime = Runtime::builder()
            .global_lock(true)
            .global_lock_timeout(Duration::from_millis(10))
            .global_lock_log_level(level)
            .thing_arc("actions", thing.clone())
            .build()
            .unwrap();
        let lock = runtime.global_lock().unwrap();
        let guard = lock.try_acquire(uuid::Uuid::new_v4()).unwrap();
        let handle = runtime.thing("actions").unwrap();
        let invocation = handle.action("z_echo").unwrap().invoke(json!(1)).unwrap();
        assert_eq!(invocation.wait().await, InvocationStatus::Error);
        assert_eq!(invocation.record().time_started, None);
        assert_eq!(invocation.error().unwrap().status, Some(409));
        assert_eq!(thing.calls.load(Ordering::SeqCst), 0);
        assert!(
            invocation
                .logs()
                .iter()
                .any(|log| log.levelname == name && log.message.contains("Global lock was busy"))
        );
        let unlocked = handle
            .action("unlocked")
            .unwrap()
            .invoke(Value::Null)
            .unwrap();
        assert_eq!(unlocked.wait().await, InvocationStatus::Completed);
        assert_eq!(thing.calls.load(Ordering::SeqCst), 1);
        drop(guard);
    }
}

#[tokio::test]
async fn invocation_inherits_parent_lock_owner_and_releases_its_own_guard() {
    let (_, runtime) = fixture(true);
    let lock = runtime.global_lock().unwrap();
    let scope = InvocationScope::fake();
    let guard = lock.try_acquire(scope.lock_owner()).unwrap();
    scope
        .clone()
        .run(async {
            let invocation = runtime
                .thing("actions")
                .unwrap()
                .action("z_echo")
                .unwrap()
                .invoke(json!(1))
                .unwrap();
            assert_eq!(invocation.wait().await, InvocationStatus::Completed);
        })
        .await;
    assert_eq!(lock.owner(), Some(scope.lock_owner()));
    drop(guard);
    assert_eq!(lock.owner(), None);
}

#[tokio::test(start_paused = true)]
async fn invoking_an_action_expires_finished_invocations_at_their_retention() {
    let (_, runtime) = fixture(false);
    let action = runtime.thing("actions").unwrap().action("z_echo").unwrap();
    let first = action.invoke(json!(1)).unwrap();
    first.wait().await;
    tokio::time::advance(Duration::from_secs(4)).await;
    assert!(runtime.invocations().get(first.id()).is_some());
    tokio::time::advance(Duration::from_secs(1)).await;
    let second = action.invoke(json!(2)).unwrap();
    assert!(runtime.invocations().get(first.id()).is_none());
    assert!(runtime.invocations().get(second.id()).is_some());
    second.wait().await;
}

#[tokio::test]
async fn shutdown_cancels_unfinished_actions_and_waits_before_stopping() {
    let (thing, runtime) = fixture(false);
    runtime.start().await.unwrap();
    let invocation = runtime
        .thing("actions")
        .unwrap()
        .action("wait")
        .unwrap()
        .invoke(Value::Null)
        .unwrap();
    thing.started.notified().await;
    runtime.shutdown(Duration::from_secs(2)).await;
    assert_eq!(invocation.status(), InvocationStatus::Cancelled);
    assert_eq!(
        invocation.error().unwrap().title.as_deref(),
        Some("InvocationCancelledError")
    );
    assert_eq!(thing.stops.load(Ordering::SeqCst), 1);
    runtime.stop().await;
    assert_eq!(thing.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_after_grace_even_if_an_action_ignores_cancellation() {
    let (thing, runtime) = fixture(false);
    runtime.start().await.unwrap();
    let invocation = runtime
        .thing("actions")
        .unwrap()
        .action("stubborn")
        .unwrap()
        .invoke(Value::Null)
        .unwrap();
    thing.started.notified().await;
    let started = tokio::time::Instant::now();
    runtime.shutdown(Duration::from_secs(5)).await;
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(invocation.status(), InvocationStatus::Running);
    assert!(invocation.cancel_token().is_cancelled());
    assert_eq!(thing.stops.load(Ordering::SeqCst), 1);
    thing.release.notify_one();
    assert_eq!(invocation.wait().await, InvocationStatus::Completed);
}

#[test]
fn thing_description_sorts_affordances_and_contains_metadata_schemas_and_forms() {
    let (_, runtime) = fixture(false);
    let handle = runtime.thing("actions").unwrap();
    assert_eq!(
        TdOptions::for_name("actions"),
        TdOptions {
            path: "/actions/".into(),
            base: None,
            id: None,
            observation: false,
            websocket: None,
            links: false,
        }
    );
    let td = serde_json::to_value(
        handle
            .thing_description(&TdOptions {
                path: "/api/actions/".into(),
                base: Some("https://example.com/".into()),
                id: Some("thing_id".into()),
                observation: false,
                websocket: None,
                links: false,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(td["title"], "Action fixture");
    assert_eq!(td["description"], "Runtime test fixture.");
    assert_eq!(td["base"], "https://example.com/");
    assert_eq!(td["id"], "thing_id");
    assert_eq!(
        td["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["a_readonly", "z_value"]
    );
    let names = td["actions"]
        .as_object()
        .unwrap()
        .keys()
        .collect::<Vec<_>>();
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    let property = &td["properties"]["z_value"];
    assert_eq!(property["title"], "Value");
    assert_eq!(property["description"], "Stored value");
    assert_eq!(property["default"], 2);
    assert_eq!(property["unit"], "items");
    assert_eq!(property["@type"], json!("Quantity"));
    assert_eq!(property["readOnly"], false);
    assert_eq!(property["writeOnly"], false);
    assert_eq!(property["forms"][0]["href"], "/api/actions/z_value");
    assert_eq!(
        property["forms"][0]["op"],
        json!(["readproperty", "writeproperty"])
    );
    assert_eq!(td["properties"]["a_readonly"]["readOnly"], true);
    assert_eq!(
        td["properties"]["a_readonly"]["forms"][0]["op"],
        json!(["readproperty"])
    );
    let action = &td["actions"]["z_echo"];
    assert_eq!(action["title"], "Echo");
    assert_eq!(action["description"], "Adds one");
    assert_eq!(action["@type"], "EchoAction");
    assert_eq!(action["input"]["title"], "z_echo_input");
    assert_eq!(action["output"]["title"], "z_echo_output");
    assert_eq!(action["forms"][0]["href"], "/api/actions/z_echo");
    assert_eq!(action["forms"][0]["op"], json!(["invokeaction"]));
    assert_eq!(td["securityDefinitions"]["no_security"]["scheme"], "nosec");
    assert_eq!(td["security"], "no_security");
}

struct BadDefinition(Prop<i64>);
impl Thing for BadDefinition {
    fn definition() -> ThingDefinition<Self> {
        ThingDefinition::new("Bad").property("bad", DataProperty::new(|t: &Self| &t.0))
    }
}
#[test]
fn invalid_affordance_definition_is_returned_from_build() {
    let thing = BadDefinition(Prop::new(0).with_constraints(Constraints::new().pattern("x")));
    assert!(
        matches!(Runtime::builder().thing("bad_thing", thing).build(),
        Err(BuildError::Definition(error)) if error.thing == "bad_thing" && error.affordance == "bad")
    );
}
