//! Settings: properties saved to disk.
//!
//! Each Thing with settings has a settings store for the file
//! `{settings_folder}/{thing}/Settings-{Class}.json`. The file holds every
//! setting, with keys in alphabetical order, as pydantic writes it with
//! `model_dump_json(indent=4)`, so that files move freely between the
//! Python and Rust servers.
//!
//! - **Saving** happens after every change: a data setting's `Prop::set`
//!   (from Rust or a client), or a functional setting written through the
//!   property. Files are written atomically (a temporary file, then a
//!   rename). Nothing is saved before the file has been loaded once.
//! - **Loading** happens when the runtime starts, before any Thing starts.
//!   A missing file leaves the defaults. An unreadable file, one that isn't
//!   a JSON object, an unknown key or an invalid value only logs a warning.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::property::PropertyError;

/// Reads and loads one setting.
pub(crate) trait SettingAccess: Send + Sync {
    /// Prepares for saving, before the file is loaded (a functional
    /// setting reads its getter once).
    fn prepare(&self) -> BoxFuture<'_, ()>;
    /// The value to save.
    fn current(&self) -> Option<Value>;
    /// Sets a value read from the file, validated as a client's value.
    fn load<'a>(&'a self, value: &'a Value) -> BoxFuture<'a, Result<(), PropertyError>>;
}

/// The settings file of one Thing.
pub(crate) struct SettingsStore {
    thing: Arc<str>,
    class: String,
    path: PathBuf,
    enabled: AtomicBool,
    /// Sorted by name.
    settings: Mutex<Vec<(String, Arc<dyn SettingAccess>)>>,
    writing: Mutex<()>,
}

impl SettingsStore {
    pub(crate) fn new(folder: &Path, thing: &Arc<str>, class: &str) -> Self {
        Self {
            thing: Arc::clone(thing),
            class: class.to_owned(),
            path: folder.join(&**thing).join(format!("Settings-{class}.json")),
            enabled: AtomicBool::new(false),
            settings: Mutex::new(Vec::new()),
            writing: Mutex::new(()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn register(&self, name: &str, access: Arc<dyn SettingAccess>) {
        let mut settings = lock(&self.settings);
        let at = settings.partition_point(|(n, _)| n.as_str() < name);
        settings.insert(at, (name.to_owned(), access));
    }

    pub(crate) fn has_settings(&self) -> bool {
        !lock(&self.settings).is_empty()
    }

    fn accesses(&self) -> Vec<(String, Arc<dyn SettingAccess>)> {
        lock(&self.settings).clone()
    }

    /// Writes every setting to the file, if loading has happened.
    pub(crate) fn save(&self) -> Result<(), PropertyError> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let _writing = lock(&self.writing);
        let mut values = Map::new();
        for (name, access) in self.accesses() {
            if let Some(value) = access.current() {
                values.insert(name, value);
            }
        }
        let text = to_settings_json(&Value::Object(values)).map_err(PropertyError::failed)?;
        // writes the file in text mode, so with CRLF on Windows.
        // JSON strings can't contain raw newlines, so this changes only the
        // layout.
        let text = if cfg!(windows) {
            text.replace('\n', "\r\n")
        } else {
            text
        };
        write_atomically(&self.path, text.as_bytes()).map_err(|error| {
            PropertyError::failed(anyhow::anyhow!(
                "couldn't save the settings of `{}` to {}: {error}",
                self.thing,
                self.path.display()
            ))
        })
    }

    /// Loads the file, then allows saving.
    pub(crate) async fn load(&self) {
        let accesses = self.accesses();
        for (_, access) in &accesses {
            access.prepare().await;
        }
        if let Some(settings) = self.read() {
            for (name, value) in &settings {
                match accesses.iter().find(|(n, _)| n == name) {
                    Some((_, access)) => {
                        if let Err(error) = access.load(value).await {
                            tracing::warn!(
                                target: "wot::things",
                                thing = %self.thing,
                                "Could not load setting {name} from settings file because of a validation error: {error}"
                            );
                        }
                    }
                    None => tracing::warn!(
                        target: "wot::things",
                        thing = %self.thing,
                        "An extra key {name} was found in the settings file. It will be deleted the next time settings are saved."
                    ),
                }
            }
        }
        self.enabled.store(true, Ordering::SeqCst);
    }

    /// The file's settings, or `None` (with a warning if it exists but can't
    /// be used).
    fn read(&self) -> Option<Map<String, Value>> {
        if !self.path.exists() {
            return None;
        }
        let parsed = fs::read(&self.path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()));
        match parsed {
            Ok(Value::Object(settings)) => Some(settings),
            Ok(_) => {
                tracing::warn!(
                    target: "wot::things",
                    thing = %self.thing,
                    "Error loading settings for {} from {}. The file does not contain a Mapping",
                    self.class,
                    self.path.display()
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    target: "wot::things",
                    thing = %self.thing,
                    "Error loading settings for {} from {}, could not load a JSON object. Settings for this Thing will be reset to default. ({error})",
                    self.class,
                    self.path.display()
                );
                None
            }
        }
    }
}

/// JSON as pydantic's `model_dump_json(indent=4)` writes it: 4-space
/// indentation, `": "` and `","` separators, UTF-8 kept, no trailing
/// newline. serde_json and pydantic-core format floats the same way
/// (`1e-7`, `1e+20`, `0.00001`), as `conformance/fixtures/settings` shows.
pub fn to_settings_json(value: &Value) -> serde_json::Result<String> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    value.serialize(&mut serializer)?;
    Ok(String::from_utf8(out).expect("serde_json writes UTF-8"))
}

/// Writes a file by writing a temporary file next to it and renaming it
/// over the target, so readers never see a partial file.
fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
