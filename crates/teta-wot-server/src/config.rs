//! The server configuration file.
//!
//! ```json
//! {
//!     "things": {
//!         "counter": "twins.counter:TestThing",
//!         "stage": {"class": "sangaboard:Stage", "kwargs": {"port": "COM3"}},
//!         "autofocus": {"cls": "autofocus:Autofocus", "thing_slots": {"camera": "cam2"}}
//!     },
//!     "settings_folder": "./settings",
//!     "api_prefix": "/api/v1",
//!     "enable_global_lock": true,
//!     "global_lock_log_level": "WARNING",
//!     "application_config": {"lab": "B12"}
//! }
//! ```
//!

use std::fmt;

use indexmap::IndexMap;
use serde_json::{Map, Value};
use teta_wot_core::SlotSelection;
use tracing::Level;

/// A server configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// The Things, by name, in the file's order (which is the start order,
    /// apart from slot dependencies).
    pub things: IndexMap<String, ThingConfig>,
    /// Where settings are kept (default is `./settings`, which
    /// [`ServerConfig::settings_folder_or_default`] applies).
    pub settings_folder: Option<String>,
    /// A prefix for every route, such as `/api/v1`.
    pub api_prefix: String,
    /// Whether actions and property writes take the global lock.
    pub enable_global_lock: bool,
    /// The level of the "Global lock was busy" log line.
    pub global_lock_log_level: Level,
    /// Anything the application wants Things to read.
    pub application_config: Option<Value>,
    /// `wot-rs`: identifies the server in TD `id`s.
    pub server_id: Option<String>,
    /// `teta-wot-rs`: the wire profile; only `"teta"` for now.
    pub wire_profile: Option<String>,
    /// Keys that were ignored.
    pub ignored_keys: Vec<String>,
}

/// One Thing of a [`ServerConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThingConfig {
    /// The Thing type, as registered.
    pub cls: String,
    /// Positional arguments.
    pub args: Vec<Value>,
    /// Keyword arguments.
    pub kwargs: Map<String, Value>,
    /// Slot overrides, by slot name.
    pub thing_slots: IndexMap<String, SlotSelection>,
}

/// Thing names teta-wot refuses.
pub const RESERVED_CONFIG_THING_NAMES: [&str; 2] = ["things", "cls"];

/// A configuration file that can't be used: every problem, located as
/// pydantic locates them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct ConfigError {
    /// `(location, message)` pairs, such as `("things.stage.kwargs", "Input should be a valid dictionary")`.
    pub errors: Vec<(String, String)>,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.errors.len();
        let plural = if count == 1 { "" } else { "s" };
        write!(f, "{count} validation error{plural} for ThingServerConfig")?;
        for (location, message) in &self.errors {
            if location.is_empty() {
                write!(f, "\n  {message}")?;
            } else {
                write!(f, "\n{location}\n  {message}")?;
            }
        }
        Ok(())
    }
}

impl ServerConfig {
    /// Reads a configuration from JSON text.
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        let value: Value = serde_json::from_str(text).map_err(|e| ConfigError {
            errors: vec![(String::new(), format!("Invalid JSON: {e}"))],
        })?;
        Self::from_value(&value)
    }

    /// Reads a configuration from a JSON value.
    pub fn from_value(value: &Value) -> Result<Self, ConfigError> {
        let mut errors = Vec::new();
        let Some(object) = value.as_object() else {
            return Err(ConfigError {
                errors: vec![(
                    String::new(),
                    "Input should be a valid dictionary or instance of ThingServerConfig".into(),
                )],
            });
        };
        let mut config = ServerConfig {
            things: IndexMap::new(),
            settings_folder: None,
            api_prefix: String::new(),
            enable_global_lock: false,
            global_lock_log_level: Level::INFO,
            application_config: None,
            server_id: None,
            wire_profile: None,
            ignored_keys: Vec::new(),
        };

        match object.get("things") {
            None => errors.push(("things".into(), "Field required".into())),
            Some(Value::Object(things)) => {
                for (name, entry) in things {
                    let location = format!("things.{name}");
                    if let Some(problem) = thing_name_problem(name) {
                        errors.push((format!("things.{name}.[key]"), problem));
                    }
                    match thing_config(entry, &location, &mut errors) {
                        Some(thing) => {
                            config.things.insert(name.clone(), thing);
                        }
                        None => continue,
                    }
                }
            }
            Some(_) => errors.push(("things".into(), "Input should be a valid dictionary".into())),
        }

        match object.get("settings_folder") {
            None | Some(Value::Null) => {}
            Some(Value::String(folder)) => config.settings_folder = Some(folder.clone()),
            Some(_) => errors.push((
                "settings_folder".into(),
                "Input should be a valid string".into(),
            )),
        }
        match object.get("api_prefix") {
            None => {}
            Some(Value::String(prefix)) if api_prefix_ok(prefix) => {
                config.api_prefix = prefix.clone()
            }
            Some(Value::String(_)) => errors.push((
                "api_prefix".into(),
                "String should match pattern '^(\\/[\\w-]+)*$'".into(),
            )),
            Some(_) => errors.push(("api_prefix".into(), "Input should be a valid string".into())),
        }
        match object.get("enable_global_lock").map(lax_bool) {
            None => {}
            Some(Some(enabled)) => config.enable_global_lock = enabled,
            Some(None) => errors.push((
                "enable_global_lock".into(),
                "Input should be a valid boolean".into(),
            )),
        }
        match object.get("global_lock_log_level") {
            None => {}
            Some(Value::String(level)) => match level.as_str() {
                "DEBUG" => config.global_lock_log_level = Level::DEBUG,
                "INFO" => config.global_lock_log_level = Level::INFO,
                "WARNING" => config.global_lock_log_level = Level::WARN,
                "ERROR" => config.global_lock_log_level = Level::ERROR,
                _ => errors.push((
                    "global_lock_log_level".into(),
                    "Input should be 'DEBUG', 'INFO', 'WARNING' or 'ERROR'".into(),
                )),
            },
            Some(_) => errors.push((
                "global_lock_log_level".into(),
                "Input should be 'DEBUG', 'INFO', 'WARNING' or 'ERROR'".into(),
            )),
        }
        match object.get("application_config") {
            None | Some(Value::Null) => {}
            Some(value @ Value::Object(_)) => config.application_config = Some(value.clone()),
            Some(_) => errors.push((
                "application_config".into(),
                "Input should be a valid dictionary".into(),
            )),
        }
        match object.get("server_id") {
            None | Some(Value::Null) => {}
            Some(Value::String(id)) => config.server_id = Some(id.clone()),
            Some(_) => errors.push(("server_id".into(), "Input should be a valid string".into())),
        }
        match object.get("wire_profile") {
            None | Some(Value::Null) => {}
            Some(Value::String(profile)) if profile == "teta" => {
                config.wire_profile = Some(profile.clone());
            }
            Some(_) => errors.push((
                "wire_profile".into(),
                "Input should be configuration file.".into(),
            )),
        }

        const KNOWN: [&str; 8] = [
            "things",
            "settings_folder",
            "api_prefix",
            "enable_global_lock",
            "global_lock_log_level",
            "application_config",
            "server_id",
            "wire_profile",
        ];
        config.ignored_keys = object
            .keys()
            .filter(|k| !KNOWN.contains(&k.as_str()))
            .cloned()
            .collect();

        if errors.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError { errors })
        }
    }

    /// The settings folder, default, `./settings`.
    pub fn settings_folder_or_default(&self) -> &str {
        self.settings_folder.as_deref().unwrap_or("./settings")
    }
}

/// rule for Thing names: `^[a-zA-Z0-9\-_]+$`, and not reserved.
fn thing_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("String should have at least 1 character".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some("String should match pattern '^([a-zA-Z0-9\\-_]+)$'".into());
    }
    if RESERVED_CONFIG_THING_NAMES.contains(&name) {
        return Some(format!(
            "Value error, {name} is not allowed as the name for a Thing."
        ));
    }
    None
}

/// `api_prefix` rule: `^(\/[\w-]+)*$`.
fn api_prefix_ok(prefix: &str) -> bool {
    prefix.is_empty()
        || (prefix.starts_with('/')
            && prefix.split('/').skip(1).all(|segment| {
                !segment.is_empty()
                    && segment
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            }))
}

/// pydantic's lax booleans from JSON.
fn lax_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) if n.as_i64() == Some(0) => Some(false),
        Value::Number(n) if n.as_i64() == Some(1) => Some(true),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            _ => None,
        },
        _ => None,
    }
}

fn thing_config(
    entry: &Value,
    location: &str,
    errors: &mut Vec<(String, String)>,
) -> Option<ThingConfig> {
    let object = match entry {
        Value::String(cls) => {
            return Some(ThingConfig {
                cls: cls.clone(),
                args: Vec::new(),
                kwargs: Map::new(),
                thing_slots: IndexMap::new(),
            });
        }
        Value::Object(object) => object,
        _ => {
            errors.push((
                location.to_owned(),
                "Input should be a Thing class (an import string) or a Thing configuration".into(),
            ));
            return None;
        }
    };
    let mut ok = true;
    let mut fail = |field: &str, message: &str| {
        errors.push((format!("{location}.{field}"), message.to_owned()));
        ok = false;
    };
    let cls = match object.get("cls").or_else(|| object.get("class")) {
        Some(Value::String(cls)) => cls.clone(),
        Some(_) => {
            fail("cls", "Input should be a valid string");
            String::new()
        }
        None => {
            fail("cls", "Field required");
            String::new()
        }
    };
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(args)) => args.clone(),
        Some(_) => {
            fail("args", "Input should be a valid list");
            Vec::new()
        }
    };
    let kwargs = match object.get("kwargs") {
        None => Map::new(),
        Some(Value::Object(kwargs)) => kwargs.clone(),
        Some(_) => {
            fail("kwargs", "Input should be a valid dictionary");
            Map::new()
        }
    };
    let mut thing_slots = IndexMap::new();
    match object.get("thing_slots") {
        None => {}
        Some(Value::Object(slots)) => {
            for (slot, target) in slots {
                let selection = match target {
                    Value::Null => Some(SlotSelection::Nothing),
                    Value::String(name) => Some(SlotSelection::Name(name.clone())),
                    Value::Array(names) => names
                        .iter()
                        .map(|n| n.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                        .map(SlotSelection::Names),
                    _ => None,
                };
                match selection {
                    Some(selection) => {
                        thing_slots.insert(slot.clone(), selection);
                    }
                    None => fail(
                        &format!("thing_slots.{slot}"),
                        "Input should be a Thing name, a list of names, or null",
                    ),
                }
            }
        }
        Some(_) => fail("thing_slots", "Input should be a valid dictionary"),
    }
    ok.then_some(ThingConfig {
        cls,
        args,
        kwargs,
        thing_slots,
    })
}
