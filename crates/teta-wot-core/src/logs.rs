//! Invocation logs: `tracing` events captured per invocation.
//!
//! Every invocation runs inside a `tracing` span named [`INVOCATION_SPAN`]
//! with an `invocation_id` field. The [`InvocationLogLayer`] copies each
//! event emitted inside such a span (at or above its level) into that
//! invocation's [`LogBuffer`], which keeps the last 1000 records. The records
//! have JSON shape ([`LogRecord`]).
//!
//! The layer must be part of the process's `tracing` subscriber; see
//! [`crate::logging::init`] or add [`InvocationLogLayer`] to your own.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use uuid::Uuid;

/// How many records an invocation keeps.
pub const LOG_CAPACITY: usize = 1000;

/// The name of the span every invocation runs in.
pub const INVOCATION_SPAN: &str = "invocation";

/// Field names with a special meaning on events inside an invocation.
pub(crate) const EXCEPTION_TYPE_FIELD: &str = "exception_type";
pub(crate) const TRACEBACK_FIELD: &str = "traceback";

/// One log record.
///
/// `created` is UTC with microseconds and a `Z` suffix. `levelname` and
/// `levelno` use Python's names and numbers: `ERROR` 40, `WARNING` 30,
/// `INFO` 20, `DEBUG` 10, and `TRACE` 5 for `tracing`'s extra level.
/// `exception_type` and `traceback` are set for errors that are logged with
/// their cause chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    /// The message, followed by any other fields as `key=value`.
    pub message: String,
    /// Python's level name.
    pub levelname: String,
    /// Python's level number.
    pub levelno: u8,
    /// Source line of the event.
    pub lineno: u32,
    /// Source file name (without its directory, as Python's `filename`).
    pub filename: String,
    /// When the event happened.
    #[serde(serialize_with = "serialize_utc")]
    pub created: DateTime<Utc>,
    /// The kind of error, for errors logged with their cause.
    pub exception_type: Option<String>,
    /// The error's cause chain, for errors logged with their cause.
    pub traceback: Option<String>,
}

/// Python's name and number for a `tracing` level.
pub fn python_level(level: &Level) -> (&'static str, u8) {
    match *level {
        Level::ERROR => ("ERROR", 40),
        Level::WARN => ("WARNING", 30),
        Level::INFO => ("INFO", 20),
        Level::DEBUG => ("DEBUG", 10),
        Level::TRACE => ("TRACE", 5),
    }
}

/// Formats a UTC time as pydantic does: microseconds (left out when zero)
/// and a `Z` suffix.
pub(crate) fn serialize_utc<S: Serializer>(time: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format_utc(time))
}

pub(crate) fn format_utc(time: &DateTime<Utc>) -> String {
    if time.timestamp_subsec_micros() == 0 {
        time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    } else {
        time.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
    }
}

/// The log of one invocation: the last [`LOG_CAPACITY`] records.
#[derive(Debug)]
pub struct LogBuffer {
    id: Uuid,
    records: Mutex<VecDeque<LogRecord>>,
}

static BUFFERS: LazyLock<Mutex<HashMap<Uuid, Weak<LogBuffer>>>> = LazyLock::new(Default::default);

impl LogBuffer {
    /// Creates the buffer for an invocation and makes it findable by the
    /// capture layer. It is forgotten when the last reference is dropped.
    pub fn register(id: Uuid) -> Arc<LogBuffer> {
        let buffer = Arc::new(LogBuffer {
            id,
            records: Mutex::new(VecDeque::new()),
        });
        lock(&BUFFERS).insert(id, Arc::downgrade(&buffer));
        buffer
    }

    fn find(id: &Uuid) -> Option<Arc<LogBuffer>> {
        lock(&BUFFERS).get(id).and_then(Weak::upgrade)
    }

    /// The invocation this buffer belongs to.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Adds a record, dropping the oldest if the buffer is full.
    pub fn push(&self, record: LogRecord) {
        let mut records = lock(&self.records);
        if records.len() == LOG_CAPACITY {
            records.pop_front();
        }
        records.push_back(record);
    }

    /// A copy of the records, oldest first.
    pub fn records(&self) -> Vec<LogRecord> {
        lock(&self.records).iter().cloned().collect()
    }

    /// The number of records held.
    pub fn len(&self) -> usize {
        lock(&self.records).len()
    }

    /// Whether no record is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for LogBuffer {
    fn drop(&mut self) {
        let mut buffers = lock(&BUFFERS);
        if buffers.get(&self.id).is_some_and(|w| w.strong_count() == 0) {
            buffers.remove(&self.id);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// A `tracing` layer that copies events inside invocation spans into the
/// invocations' logs.
///
/// Events are captured at `max_level` and above (INFO by default; DEBUG in debug mode).
/// Only the innermost invocation span counts, so a child invocation keeps its own log.
#[derive(Debug, Clone)]
pub struct InvocationLogLayer {
    max_level: Level,
}

impl Default for InvocationLogLayer {
    fn default() -> Self {
        Self::new(Level::INFO)
    }
}

impl InvocationLogLayer {
    /// A layer that captures events at `max_level` and more severe.
    pub fn new(max_level: Level) -> Self {
        Self { max_level }
    }
}

/// Stored in the extensions of an invocation span.
struct Capture(Arc<LogBuffer>);

impl<S> Layer<S> for InvocationLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() != INVOCATION_SPAN {
            return;
        }
        let mut visitor = IdVisitor(None);
        attrs.record(&mut visitor);
        if let Some(buffer) = visitor.0.and_then(|id| LogBuffer::find(&id))
            && let Some(span) = ctx.span(id)
        {
            span.extensions_mut().insert(Capture(buffer));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        // Levels compare by verbosity: ERROR < WARN < INFO < DEBUG < TRACE.
        if *metadata.level() > self.max_level {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        // The scope runs from the innermost span outwards.
        for span in scope {
            if let Some(Capture(buffer)) = span.extensions().get::<Capture>() {
                buffer.push(record_for(event));
                return;
            }
        }
    }
}

struct IdVisitor(Option<Uuid>);

impl Visit for IdVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "invocation_id" {
            self.0 = value.parse().ok();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "invocation_id" {
            self.0 = format!("{value:?}").parse().ok();
        }
    }
}

#[derive(Default)]
struct RecordVisitor {
    message: String,
    fields: String,
    exception_type: Option<String>,
    traceback: Option<String>,
}

impl RecordVisitor {
    fn record(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            EXCEPTION_TYPE_FIELD => self.exception_type = Some(value),
            TRACEBACK_FIELD => self.traceback = Some(value),
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.fields, " {name}={value}");
            }
        }
    }
}

impl Visit for RecordVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

fn record_for(event: &Event<'_>) -> LogRecord {
    let metadata = event.metadata();
    let mut visitor = RecordVisitor::default();
    event.record(&mut visitor);
    let (levelname, levelno) = python_level(metadata.level());
    let filename = metadata
        .file()
        .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f).to_owned())
        .unwrap_or_default();
    LogRecord {
        message: visitor.message + &visitor.fields,
        levelname: levelname.to_owned(),
        levelno,
        lineno: metadata.line().unwrap_or(0),
        filename,
        created: Utc::now(),
        exception_type: visitor.exception_type,
        traceback: visitor.traceback,
    }
}
