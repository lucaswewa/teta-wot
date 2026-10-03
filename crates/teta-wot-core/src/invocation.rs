//! Action invocations and their manager.
//!
//! An invocation goes `pending` → `running` → `completed`, `error` or
//! `cancelled`; it can also go straight from `pending` to `error` when the
//! global lock is busy. Finished invocations are kept for their action's
//! retention time (300 s by default), then removed by
//! [`InvocationManager::expire`], which runs on every new invocation.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

use crate::blob::{BlobData, Serialised};
use crate::broker::{Message, MessageBroker, MessageKind};
use crate::cancel::CancelToken;
use crate::logs::{LogBuffer, LogRecord};
use crate::problem::ProblemDetails;

/// Where an invocation is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InvocationStatus {
    /// Created, not yet running (waiting for the global lock, if enabled).
    Pending,
    /// The action is running.
    Running,
    /// The action returned a value.
    Completed,
    /// The action was cancelled and has finished.
    Cancelled,
    /// The action failed, or could not start.
    Error,
}

impl InvocationStatus {
    /// The name used on the wire, such as `"running"`.
    pub fn as_str(self) -> &'static str {
        match self {
            InvocationStatus::Pending => "pending",
            InvocationStatus::Running => "running",
            InvocationStatus::Completed => "completed",
            InvocationStatus::Cancelled => "cancelled",
            InvocationStatus::Error => "error",
        }
    }

    /// Whether the invocation has finished.
    pub fn is_finished(self) -> bool {
        !matches!(self, InvocationStatus::Pending | InvocationStatus::Running)
    }
}

impl std::fmt::Display for InvocationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug)]
struct State {
    status: InvocationStatus,
    requested: DateTime<Utc>,
    started: Option<DateTime<Utc>>,
    completed: Option<DateTime<Utc>>,
    output: Option<Serialised>,
    error: Option<ProblemDetails>,
    expires: Option<Instant>,
}

/// One invocation of an action.
#[derive(Debug)]
pub struct Invocation {
    id: Uuid,
    thing: Arc<str>,
    action: Arc<str>,
    input: Serialised,
    retention: Duration,
    cancel: CancelToken,
    logs: Arc<LogBuffer>,
    state: Mutex<State>,
    finished: watch::Sender<bool>,
    broker: Arc<MessageBroker>,
}

/// A snapshot of an invocation, with everything.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InvocationRecord {
    /// The status.
    pub status: InvocationStatus,
    /// The invocation's ID.
    pub id: Uuid,
    /// The Thing's name.
    pub thing: String,
    /// The action's name.
    pub action: String,
    /// When the invocation was requested.
    pub time_requested: DateTime<Utc>,
    /// When the action started running.
    pub time_started: Option<DateTime<Utc>>,
    /// When the invocation finished.
    pub time_completed: Option<DateTime<Utc>>,
    /// The validated input.
    pub input: Value,
    /// The output, once completed.
    pub output: Option<Value>,
    /// The log.
    pub log: Vec<LogRecord>,
    /// What went wrong, for `error` and `cancelled`.
    pub error: Option<ProblemDetails>,
}

impl Invocation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Uuid,
        thing: Arc<str>,
        action: Arc<str>,
        input: Serialised,
        retention: Duration,
        cancel: CancelToken,
        logs: Arc<LogBuffer>,
        broker: Arc<MessageBroker>,
    ) -> Self {
        Self {
            id,
            thing,
            action,
            input,
            retention,
            cancel,
            logs,
            state: Mutex::new(State {
                status: InvocationStatus::Pending,
                requested: Utc::now(),
                started: None,
                completed: None,
                output: None,
                error: None,
                expires: None,
            }),
            finished: watch::Sender::new(false),
            broker,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The invocation's ID.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The Thing's name.
    pub fn thing(&self) -> &str {
        &self.thing
    }

    /// The action's name.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// The validated input.
    pub fn input(&self) -> &Value {
        &self.input.value
    }

    /// The current status.
    pub fn status(&self) -> InvocationStatus {
        self.state().status
    }

    /// The output, once completed.
    pub fn output(&self) -> Option<Value> {
        self.state().output.as_ref().map(|o| o.value.clone())
    }

    /// The output's data.
    pub fn output_blob(&self) -> Option<Arc<BlobData>> {
        self.state()
            .output
            .as_ref()
            .and_then(|o| o.as_blob().cloned())
    }

    /// What went wrong, for `error` and `cancelled` invocations.
    pub fn error(&self) -> Option<ProblemDetails> {
        self.state().error.clone()
    }

    /// The log so far.
    pub fn logs(&self) -> Vec<LogRecord> {
        self.logs.records()
    }

    /// The cancel token.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// A snapshot of everything about the invocation.
    pub fn record(&self) -> InvocationRecord {
        let state = self.state();
        InvocationRecord {
            status: state.status,
            id: self.id,
            thing: self.thing.to_string(),
            action: self.action.to_string(),
            time_requested: state.requested,
            time_started: state.started,
            time_completed: state.completed,
            input: self.input.value.clone(),
            output: state.output.as_ref().map(|o| o.value.clone()),
            log: self.logs.records(),
            error: state.error.clone(),
        }
    }

    /// Waits until the invocation has finished, and returns its status.
    pub async fn wait(&self) -> InvocationStatus {
        let mut finished = self.finished.subscribe();
        let _ = finished.wait_for(|done| *done).await;
        self.status()
    }

    fn publish(&self, status: InvocationStatus) {
        self.broker.publish(Message::new(
            &*self.thing,
            &*self.action,
            MessageKind::Action,
            Value::from(status.as_str()),
        ));
    }

    pub(crate) fn announce(&self) {
        self.publish(InvocationStatus::Pending);
    }

    pub(crate) fn set_running(&self) {
        {
            let mut state = self.state();
            state.status = InvocationStatus::Running;
            state.started = Some(Utc::now());
        }
        self.publish(InvocationStatus::Running);
    }

    pub(crate) fn finish(
        &self,
        status: InvocationStatus,
        output: Option<Serialised>,
        error: Option<ProblemDetails>,
    ) {
        debug_assert!(status.is_finished());
        {
            let mut state = self.state();
            state.status = status;
            state.output = output;
            state.error = error;
            state.completed = Some(Utc::now());
            state.expires = Some(Instant::now() + self.retention);
        }
        self.publish(status);
        self.finished.send_replace(true);
    }

    fn expired(&self, now: Instant) -> bool {
        self.state().expires.is_some_and(|expires| expires <= now)
    }
}

/// An invocation can't be cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CancelError {
    /// There is no such invocation (reference bug B1: the ID is filled in).
    #[error("No action invocation found with ID {0}")]
    NotFound(Uuid),
    /// It has already finished (reference bug B2: plain status name).
    #[error("The invocation is {0} and may not be cancelled.")]
    NotCancellable(InvocationStatus),
}

/// All invocations that are running or finished recently.
#[derive(Debug, Default)]
pub struct InvocationManager {
    invocations: Mutex<IndexMap<Uuid, Arc<Invocation>>>,
}

impl InvocationManager {
    /// No invocations.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, IndexMap<Uuid, Arc<Invocation>>> {
        self.invocations.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn insert(&self, invocation: Arc<Invocation>) {
        self.lock().insert(invocation.id, invocation);
    }

    /// An invocation by ID.
    pub fn get(&self, id: Uuid) -> Option<Arc<Invocation>> {
        self.lock().get(&id).cloned()
    }

    /// Every invocation, oldest first.
    pub fn list(&self) -> Vec<Arc<Invocation>> {
        self.lock().values().cloned().collect()
    }

    /// The invocations of one action of one Thing, oldest first.
    pub fn list_for(&self, thing: &str, action: &str) -> Vec<Arc<Invocation>> {
        self.lock()
            .values()
            .filter(|i| &*i.thing == thing && &*i.action == action)
            .cloned()
            .collect()
    }

    /// Asks a pending or running invocation to stop.
    pub fn cancel(&self, id: Uuid) -> Result<(), CancelError> {
        let invocation = self.get(id).ok_or(CancelError::NotFound(id))?;
        match invocation.status() {
            status @ (InvocationStatus::Completed
            | InvocationStatus::Cancelled
            | InvocationStatus::Error) => Err(CancelError::NotCancellable(status)),
            InvocationStatus::Pending | InvocationStatus::Running => {
                invocation.cancel.cancel();
                Ok(())
            }
        }
    }

    /// Asks every unfinished invocation to stop (for shutdown).
    pub fn cancel_all(&self) {
        for invocation in self.list() {
            if !invocation.status().is_finished() {
                invocation.cancel.cancel();
            }
        }
    }

    /// Waits until no invocation is unfinished.
    pub async fn wait_idle(&self) {
        for invocation in self.list() {
            invocation.wait().await;
        }
    }

    /// Removes finished invocations whose retention time has passed, and
    /// returns how many were removed.
    pub fn expire(&self) -> usize {
        let now = Instant::now();
        let mut invocations = self.lock();
        let before = invocations.len();
        invocations.retain(|_, invocation| !invocation.expired(now));
        before - invocations.len()
    }
}
