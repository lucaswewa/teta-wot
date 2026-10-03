//! Blobs: binary data that actions take and return, downloaded from
//! `/blob/{id}`.
//!
//! A [`Blob<M>`] is a handle to data held in memory, in a file, in a file
//! of a temporary directory, or at a remote URL. `M` is its
//! [`MediaType`]: [`Jpeg`], [`TextPlain`], or one of your own.
//!
//! In JSON, a Blob is a link object:
//!
//! ```json
//! {"href": "http://host/blob/<id>", "media_type": "image/jpeg", "rel": "output", "description": "…"}
//! ```
//!
//! **Lifetime.** Local data is registered under a random ID in a registry
//! of weak referencess: it can be downloaded for as long
//! as something holds the Blob. An invocation holds the Blobs of its input
//! and output until it expires, so a Blob output can be downloaded for the
//! action's retention time. When the last handle goes, the data is freed,
//! and a temporary directory is deleted.
//!
//! **Serialisation.** Serialised outside a request, a local Blob's `href`
//! is the relative reference `blob/<id>`. The HTTP binding makes it
//! absolute, with the request's host and API prefix, when it writes a
//! response. Deserialising reads the ID from the `href` (anything ending
//! in `blob/<id>`) and finds the data in the registry.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use bytes::Bytes;
use regex::Regex;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

/// The `rel` of a Blob's JSON form.
const REL: &str = "output";

/// default description of a serialised Blob.
pub const DEFAULT_DESCRIPTION: &str = "The output from this action is not serialised to JSON, so it must be retrieved as a file. This link will return the file.";

/// The description of a Blob's JSON form in schemas
const MODEL_DESCRIPTION: &str = "A model for JSON-serialised `.Blob` objects.\n\nThis model describes the JSON representation of a `.Blob`\nand is used to describe the `.Blob` object in JSON responses.\n\nThe binary data may be retrieved with a ``GET`` request to the URL\nspecified in ``href`` which should return it directly in the body of\nthe response.";

/// Marks a deserialisation error as a Blob's, so that validation can
/// report it where pydantic does (see `crate::property::from_client`).
pub(crate) const ERROR_MARK: &str = "\u{1}blob\u{1}";

// ---- Media types ------------------------------------------------------------

/// The media type of a kind of Blob: with
/// their `media_type` and `description` class attributes.
///
/// ```
/// use teta_wot_core::blob::{Blob, MediaType};
///
/// /// Spectra, as comma-separated values.
/// struct Spectrum;
///
/// impl MediaType for Spectrum {
///     const MEDIA_TYPE: &'static str = "text/csv";
///     const TITLE: &'static str = "SpectrumBlob";
///     const DESCRIPTION: Option<&'static str> = Some("A spectrum, one wavelength per line.");
/// }
///
/// let blob: Blob<Spectrum> = Blob::from_bytes("nm,counts\n500,12\n");
/// assert_eq!(blob.media_type(), "text/csv");
/// ```
pub trait MediaType: Send + Sync + 'static {
    /// The media type, which may have wildcards (`image/*`, `*/*`). Data
    /// of a more specific matching type is accepted.
    const MEDIA_TYPE: &'static str;
    /// The title of the Blob's schema.
    const TITLE: &'static str = "Blob";
    /// The description written in the Blob's JSON form, instead of
    /// [`DEFAULT_DESCRIPTION`].
    const DESCRIPTION: Option<&'static str> = None;
}

macro_rules! media_types {
    ($($(#[$doc:meta])* $name:ident = $media_type:literal, $title:literal;)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
            pub struct $name;

            impl MediaType for $name {
                const MEDIA_TYPE: &'static str = $media_type;
                const TITLE: &'static str = $title;
            }
        )*
    };
}

media_types! {
    /// Any media type (`*/*`).
    AnyMedia = "*/*", "Blob";
    /// JPEG images (`image/jpeg`).
    Jpeg = "image/jpeg", "JPEGBlob";
    /// PNG images (`image/png`).
    Png = "image/png", "PNGBlob";
    /// TIFF images (`image/tiff`).
    Tiff = "image/tiff", "TIFFBlob";
    /// Plain text (`text/plain`).
    TextPlain = "text/plain", "TextBlob";
    /// Comma-separated values (`text/csv`).
    Csv = "text/csv", "CSVBlob";
    /// JSON documents (`application/json`).
    Json = "application/json", "JSONBlob";
    /// Arbitrary bytes (`application/octet-stream`).
    OctetStream = "application/octet-stream", "BinaryBlob";
}

/// A media type is malformed, or doesn't match a Blob's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaTypeError {
    /// Not `type/subtype`.
    #[error("Invalid media type: {media_type} {problem}")]
    Invalid {
        /// The media type, without parameters.
        media_type: String,
        /// What is wrong.
        problem: &'static str,
    },
    /// The data's media type isn't the Blob's.
    #[error("Can't create a {blob} as media type '{media_type}' doesn't match '{expected}'.")]
    Mismatch {
        /// The Blob's title.
        blob: &'static str,
        /// The data's media type.
        media_type: String,
        /// The Blob's media type.
        expected: &'static str,
    },
}

/// Splits a media type into its type and subtype, ignoring parameters
/// (`text/plain; charset=utf-8` → `("text", "plain")`).
pub fn parse_media_type(media_type: &str) -> Result<(String, String), MediaTypeError> {
    let bare = media_type
        .trim()
        .split(';')
        .next()
        .unwrap_or_default()
        .to_owned();
    let invalid = |problem| MediaTypeError::Invalid {
        media_type: bare.clone(),
        problem,
    };
    let parts: Vec<&str> = bare.split('/').collect();
    let [main, sub] = parts[..] else {
        return Err(invalid("must contain exactly one '/'."));
    };
    let (main, sub) = (main.trim(), sub.trim());
    if main.is_empty() || sub.is_empty() {
        return Err(invalid("must have both type and subtype."));
    }
    if main == "*" && sub != "*" {
        return Err(invalid("has no type but has a subtype."));
    }
    Ok((main.to_owned(), sub.to_owned()))
}

/// Whether a media type matches a pattern that may have wildcards
/// (`image/png` matches `image/*` and `*/*`).
pub fn media_types_match(media_type: &str, pattern: &str) -> Result<bool, MediaTypeError> {
    let (main, sub) = parse_media_type(media_type)?;
    let (pattern_main, pattern_sub) = parse_media_type(pattern)?;
    Ok((pattern_main == "*" || main == pattern_main) && (pattern_sub == "*" || sub == pattern_sub))
}

// ---- The data and its registry ------------------------------------------------

/// Where a Blob's data is.
enum Source {
    Bytes(Bytes),
    File {
        path: PathBuf,
        /// A temporary directory, deleted with the data.
        _folder: Option<tempfile::TempDir>,
    },
    Remote(String),
}

/// A Blob's data: registered under its ID while anything holds it, unless
/// it is remote.
pub struct BlobData {
    id: Uuid,
    media_type: String,
    source: Source,
}

/// What a Blob's data is, to serve it.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum BlobContent<'a> {
    /// Bytes in memory.
    Bytes(&'a Bytes),
    /// A file on disk.
    File(&'a Path),
    /// Data at a URL, not on this server.
    Remote(&'a str),
}

impl fmt::Debug for BlobData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobData")
            .field("id", &self.id)
            .field("media_type", &self.media_type)
            .field("content", &self.content())
            .finish()
    }
}

type Registry = HashMap<Uuid, Weak<BlobData>>;

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

impl BlobData {
    fn register(media_type: String, source: Source) -> Arc<Self> {
        let data = Arc::new(Self {
            id: Uuid::new_v4(),
            media_type,
            source,
        });
        if !data.is_remote() {
            registry().insert(data.id, Arc::downgrade(&data));
        }
        data
    }

    /// The data registered under `id`, if anything still holds it.
    pub fn find(id: Uuid) -> Option<Arc<Self>> {
        // Take the weak reference out before upgrading, so that the lock
        // isn't held if the upgraded reference is the last one dropped.
        let weak = registry().get(&id).cloned()?;
        weak.upgrade()
    }

    /// The ID it is downloaded by (a remote Blob has one too, unused).
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The media type of the data.
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Where the data is.
    pub fn content(&self) -> BlobContent<'_> {
        match &self.source {
            Source::Bytes(bytes) => BlobContent::Bytes(bytes),
            Source::File { path, .. } => BlobContent::File(path),
            Source::Remote(href) => BlobContent::Remote(href),
        }
    }

    /// Whether the data is at a remote URL.
    pub fn is_remote(&self) -> bool {
        matches!(self.source, Source::Remote(_))
    }

    /// The `href` written when the Blob is serialised: the remote URL, or
    /// `blob/<id>`, which the HTTP binding makes absolute.
    pub fn href(&self) -> String {
        match &self.source {
            Source::Remote(href) => href.clone(),
            _ => local_href(self.id),
        }
    }
}

impl Drop for BlobData {
    fn drop(&mut self) {
        if !self.is_remote() {
            registry().remove(&self.id);
        }
    }
}

/// The relative `href` of local data: `blob/<id>`.
pub fn local_href(id: Uuid) -> String {
    format!("blob/{id}")
}

/// The ID in a local `href` written by [`local_href`], if `href` is exactly
/// that and the data still exists.
pub fn local_href_id(href: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(href.strip_prefix("blob/")?).ok()?;
    BlobData::find(id).map(|_| id)
}

/// The media type of a Blob, if `schema` is a Blob's DataSchema (as
/// `Blob<M>` describes itself): an object with an `href`, a `media_type`
/// whose default is the media type, and a `rel` fixed to `output`.
pub fn schema_media_type(schema: &teta_wot_td::DataSchema) -> Option<&str> {
    let properties = schema.properties.as_ref()?;
    properties.get("href")?;
    let rel = properties.get("rel")?.constant.as_ref()?;
    if rel != REL {
        return None;
    }
    properties.get("media_type")?.default.as_ref()?.as_str()
}

// ---- Keeping serialised Blobs alive -------------------------------------------

thread_local! {
    static CAPTURE: RefCell<Option<Vec<Arc<BlobData>>>> = const { RefCell::new(None) };
}

/// A value serialised to JSON, with the data of the Blobs in it, which it
/// keeps alive: an invocation's input and output.
#[derive(Debug, Clone, Default)]
pub struct Serialised {
    /// The JSON value.
    pub value: Value,
    /// The data of the Blobs serialised into it.
    pub blobs: Vec<Arc<BlobData>>,
}

impl Serialised {
    /// Serialises `value`, collecting the data of the Blobs in it.
    pub fn new<T: Serialize + ?Sized>(value: &T) -> Result<Self, serde_json::Error> {
        struct Restore(Option<Vec<Arc<BlobData>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let previous = self.0.take();
                CAPTURE.with(|c| *c.borrow_mut() = previous);
            }
        }
        let restore = Restore(CAPTURE.with(|c| c.replace(Some(Vec::new()))));
        let result = serde_json::to_value(value);
        let blobs = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
        drop(restore);
        Ok(Self {
            value: result?,
            blobs,
        })
    }

    /// The Blob's data, if the whole value is one Blob.
    pub fn as_blob(&self) -> Option<&Arc<BlobData>> {
        let href = self.value.as_object()?.get("href")?.as_str()?;
        match &self.blobs[..] {
            [data] if data.href() == href && self.value.get("rel") == Some(&Value::from(REL)) => {
                Some(data)
            }
            _ => None,
        }
    }
}

// ---- Blob ----------------------------------------------------------------------

/// Binary data that an action takes or returns, with
/// the media type `M`.
///
/// ```
/// use teta_wot_core::blob::{Blob, Jpeg};
///
/// let image: Blob<Jpeg> = Blob::from_bytes(vec![0xff, 0xd8, 0xff, 0xd9]);
/// assert_eq!(image.media_type(), "image/jpeg");
/// assert!(image.id().is_some());
/// ```
///
/// A Blob is a cheap handle: clones share the data, which is freed with the
/// last one. See the [module documentation](self) for its lifetime and JSON
/// form.
pub struct Blob<M: MediaType = AnyMedia> {
    data: Arc<BlobData>,
    description: Option<String>,
    _media: PhantomData<fn() -> M>,
}

impl<M: MediaType> Clone for Blob<M> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            description: self.description.clone(),
            _media: PhantomData,
        }
    }
}

impl<M: MediaType> fmt::Debug for Blob<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("media_type", &self.data.media_type)
            .field("href", &self.data.href())
            .finish()
    }
}

fn not_found(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, message.to_owned())
}

impl<M: MediaType> Blob<M> {
    fn from_data(data: Arc<BlobData>) -> Self {
        Self {
            data,
            description: None,
            _media: PhantomData,
        }
    }

    /// Checks a media type against `M`'s; `None` means `M`'s.
    fn media_type_for(media_type: Option<&str>) -> Result<String, MediaTypeError> {
        let Some(media_type) = media_type else {
            return Ok(M::MEDIA_TYPE.to_owned());
        };
        if media_types_match(media_type, M::MEDIA_TYPE)? {
            Ok(media_type.to_owned())
        } else {
            Err(MediaTypeError::Mismatch {
                blob: M::TITLE,
                media_type: media_type.to_owned(),
                expected: M::MEDIA_TYPE,
            })
        }
    }

    /// Data in memory, of `M`'s media type.
    pub fn from_bytes(data: impl Into<Bytes>) -> Self {
        Self::from_data(BlobData::register(
            M::MEDIA_TYPE.to_owned(),
            Source::Bytes(data.into()),
        ))
    }

    /// Data in memory, of a more specific media type than `M`'s (such as
    /// `text/csv` for a `text/*` Blob).
    pub fn from_bytes_as(data: impl Into<Bytes>, media_type: &str) -> Result<Self, MediaTypeError> {
        let media_type = Self::media_type_for(Some(media_type))?;
        Ok(Self::from_data(BlobData::register(
            media_type,
            Source::Bytes(data.into()),
        )))
    }

    /// A file that stays on disk for at least as long as the Blob. For a
    /// file that should be deleted afterwards, use
    /// [`from_temporary_directory`](Self::from_temporary_directory).
    pub fn from_file(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::file(path.into(), None, None)
    }

    /// A file of a more specific media type than `M`'s.
    pub fn from_file_as(path: impl Into<PathBuf>, media_type: &str) -> io::Result<Self> {
        Self::file(path.into(), None, Some(media_type))
    }

    /// A file in a temporary directory, which is deleted with the Blob's
    /// data: the safe way to return a file made for the purpose.
    pub fn from_temporary_directory(
        folder: tempfile::TempDir,
        file: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let path = folder.path().join(file);
        Self::file(path, Some(folder), None)
    }

    fn file(
        path: PathBuf,
        folder: Option<tempfile::TempDir>,
        media_type: Option<&str>,
    ) -> io::Result<Self> {
        let media_type = Self::media_type_for(media_type)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if !path.is_file() {
            return Err(not_found("Tried to return a file that doesn't exist."));
        }
        Ok(Self::from_data(BlobData::register(
            media_type,
            Source::File {
                path,
                _folder: folder,
            },
        )))
    }

    /// Data at a URL, not on this server. Its JSON form links to the URL,
    /// and the invocation's `/output` redirects there.
    pub fn from_url(href: impl Into<String>) -> Self {
        Self::from_data(BlobData::register(
            M::MEDIA_TYPE.to_owned(),
            Source::Remote(href.into()),
        ))
    }

    /// Sets the description written in the Blob's JSON form.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The media type of the data: `M`'s, or a more specific one.
    pub fn media_type(&self) -> &str {
        &self.data.media_type
    }

    /// The ID the data is downloaded by, unless it is remote.
    pub fn id(&self) -> Option<Uuid> {
        (!self.data.is_remote()).then_some(self.data.id)
    }

    /// The description written in the Blob's JSON form.
    pub fn description(&self) -> &str {
        self.description
            .as_deref()
            .or(M::DESCRIPTION)
            .unwrap_or(DEFAULT_DESCRIPTION)
    }

    /// The data's registry entry, as the HTTP binding serves it.
    pub fn data(&self) -> &Arc<BlobData> {
        &self.data
    }

    /// Reads the data into memory. Remote data can't be read.
    pub async fn bytes(&self) -> io::Result<Bytes> {
        match &self.data.source {
            Source::Bytes(bytes) => Ok(bytes.clone()),
            Source::File { path, .. } => Ok(tokio::fs::read(path).await?.into()),
            Source::Remote(href) => Err(remote(href)),
        }
    }

    /// Reads the data into memory, from synchronous code (a device's
    /// thread, a blocking action).
    pub fn bytes_blocking(&self) -> io::Result<Bytes> {
        match &self.data.source {
            Source::Bytes(bytes) => Ok(bytes.clone()),
            Source::File { path, .. } => Ok(std::fs::read(path)?.into()),
            Source::Remote(href) => Err(remote(href)),
        }
    }

    /// Writes the data to a file.
    pub async fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        match &self.data.source {
            Source::Bytes(bytes) => tokio::fs::write(path, bytes).await,
            Source::File { path: from, .. } => tokio::fs::copy(from, path).await.map(|_| ()),
            Source::Remote(href) => Err(remote(href)),
        }
    }

    /// The same data as a Blob of another media type, if the data's type
    /// matches it.
    pub fn cast<N: MediaType>(self) -> Result<Blob<N>, MediaTypeError> {
        Blob::<N>::media_type_for(Some(&self.data.media_type))?;
        Ok(Blob {
            data: self.data,
            description: self.description,
            _media: PhantomData,
        })
    }
}

fn remote(href: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("the Blob's data is at {href}, not on this server"),
    )
}

impl<M: MediaType> Serialize for Blob<M> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CAPTURE.with(|c| {
            if let Some(blobs) = c.borrow_mut().as_mut() {
                blobs.push(Arc::clone(&self.data));
            }
        });
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("href", &self.data.href())?;
        map.serialize_entry("media_type", &self.data.media_type)?;
        map.serialize_entry("rel", REL)?;
        map.serialize_entry("description", self.description())?;
        map.end()
    }
}

/// A Blob's JSON form.
#[derive(Deserialize)]
struct BlobModel {
    href: String,
    #[allow(dead_code)] // required
    media_type: String,
}

fn blob_error<E: de::Error>(message: impl fmt::Display) -> E {
    E::custom(format!("{ERROR_MARK}{message}"))
}

/// The Blob ID in a URL.
fn url_to_id(href: &str) -> Option<Result<Uuid, ()>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(r"blob/([0-9a-z\-]+)").expect("a valid regex"));
    let id = pattern.captures(href)?.get(1)?.as_str();
    Some(Uuid::parse_str(id).map_err(|_| ()))
}

impl<'de, M: MediaType> Deserialize<'de> for Blob<M> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let model = BlobModel::deserialize(deserializer)?;
        let id = match url_to_id(&model.href) {
            None => return Err(blob_error("Blob URLs must contain a Blob ID.")),
            Some(Err(())) => return Err(blob_error("badly formed hexadecimal UUID string")),
            Some(Ok(id)) => id,
        };
        let data = BlobData::find(id)
            .ok_or_else(|| blob_error(format!("Blob ID {id} wasn't found on this server.")))?;
        if !media_types_match(&data.media_type, M::MEDIA_TYPE).map_err(blob_error)? {
            return Err(blob_error(format!(
                "Blob data media_type '{}' does not match Blob media_type '{}'.",
                data.media_type,
                M::MEDIA_TYPE
            )));
        }
        Ok(Self::from_data(data))
    }
}

impl<M: MediaType> JsonSchema for Blob<M> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        M::TITLE.into()
    }

    fn inline_schema() -> bool {
        true
    }

    /// a specific media type is a string `const`, not a one-element array.
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut media_type = Map::new();
        media_type.insert("title".into(), "Media Type".into());
        if !M::MEDIA_TYPE.contains('*') {
            media_type.insert("const".into(), M::MEDIA_TYPE.into());
        }
        media_type.insert("default".into(), M::MEDIA_TYPE.into());
        media_type.insert("type".into(), "string".into());
        json_schema!({
            "description": MODEL_DESCRIPTION,
            "title": M::TITLE,
            "type": "object",
            "properties": {
                "href": {"title": "Href", "type": "string"},
                "media_type": media_type,
                "rel": {"title": "Rel", "const": REL, "default": REL, "type": "string"},
                "description": {
                    "title": "Description",
                    "default": M::DESCRIPTION.unwrap_or(DEFAULT_DESCRIPTION),
                    "type": "string"
                }
            },
            "required": ["href", "media_type"]
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn media_types_parse() {
        let parsed = |m: &str| parse_media_type(m).unwrap();
        assert_eq!(parsed("text/plain"), ("text".into(), "plain".into()));
        assert_eq!(
            parsed("text/plain; charset=utf-8"),
            ("text".into(), "plain".into())
        );
        assert_eq!(parsed("*/*"), ("*".into(), "*".into()));
        for (bad, message) in [
            ("too/many/slashes", "exactly one '/'"),
            ("/leadingslash", "both type and subtype"),
            ("*/plain", "has no type"),
        ] {
            assert!(
                parse_media_type(bad)
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
        for (media_type, pattern, expected) in [
            ("text/plain", "text/plain", true),
            ("text/html", "text/*", true),
            ("image/png", "image/*", true),
            ("application/json", "*/*", true),
            ("text/plain", "text/html", false),
            ("image/jpeg", "image/png", false),
            ("text/plain", "image/*", false),
        ] {
            assert_eq!(media_types_match(media_type, pattern).unwrap(), expected);
        }
    }

    struct VagueText;
    impl MediaType for VagueText {
        const MEDIA_TYPE: &'static str = "text/*";
        const TITLE: &'static str = "VagueTextBlob";
        const DESCRIPTION: Option<&'static str> = Some("Some vague text.");
    }

    #[test]
    fn media_types_are_checked_when_blobs_are_made() {
        let csv = Blob::<VagueText>::from_bytes_as("a,b", "text/csv").unwrap();
        assert_eq!(csv.media_type(), "text/csv");
        let error = Blob::<TextPlain>::from_bytes_as("x", "image/png").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Can't create a TextBlob as media type 'image/png' doesn't match 'text/plain'."
        );
        assert!(csv.cast::<TextPlain>().is_err());
    }

    #[test]
    fn data_is_freed_with_the_last_handle() {
        let blob = Blob::<TextPlain>::from_bytes("hello");
        let id = blob.id().unwrap();
        let clone = blob.clone();
        drop(blob);
        assert!(BlobData::find(id).is_some());
        drop(clone);
        assert!(BlobData::find(id).is_none());
    }

    #[test]
    fn temporary_directories_are_deleted_with_the_data() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("a.txt"), "file").unwrap();
        let path = folder.path().to_owned();
        let blob = Blob::<TextPlain>::from_temporary_directory(folder, "a.txt").unwrap();
        assert_eq!(blob.bytes_blocking().unwrap(), "file");
        drop(blob);
        assert!(!path.exists());
        let missing = Blob::<TextPlain>::from_file(path.join("gone.txt")).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "Tried to return a file that doesn't exist."
        );
    }

    #[test]
    fn serialised_blobs_are_kept_with_their_json() {
        let blob = Blob::<TextPlain>::from_bytes("x");
        let id = blob.id().unwrap();
        let serialised = Serialised::new(&blob).unwrap();
        drop(blob);
        assert_eq!(
            serialised.value,
            json!({
                "href": format!("blob/{id}"),
                "media_type": "text/plain",
                "rel": "output",
                "description": DEFAULT_DESCRIPTION,
            })
        );
        assert!(serialised.as_blob().is_some());
        let back: Blob<TextPlain> = serde_json::from_value(serialised.value.clone()).unwrap();
        assert_eq!(back.id(), Some(id));
        let nested = Serialised::new(&json!({"a": 1})).unwrap();
        assert!(nested.as_blob().is_none() && nested.blobs.is_empty());
    }

    #[test]
    fn deserialising_finds_the_data_by_its_href() {
        let blob = Blob::<Jpeg>::from_bytes(vec![0xff]);
        let href = format!("http://host/api/blob/{}", blob.id().unwrap());
        let found: Blob<AnyMedia> =
            serde_json::from_value(json!({"href": href, "media_type": "image/jpeg"})).unwrap();
        assert_eq!(found.media_type(), "image/jpeg");
        let message = |value: Value| {
            serde_json::from_value::<Blob<TextPlain>>(value)
                .unwrap_err()
                .to_string()
        };
        assert!(message(json!({"href": href, "media_type": "x"})).contains(
            "Blob data media_type 'image/jpeg' does not match Blob media_type 'text/plain'."
        ));
        assert!(
            message(json!({"href": "http://x/bogus", "media_type": "x"}))
                .contains("Blob URLs must contain a Blob ID.")
        );
        assert!(
            message(json!({"href": format!("blob/{}", Uuid::nil()), "media_type": "x"}))
                .contains("wasn't found on this server.")
        );
    }

    #[test]
    fn the_schema_with_a_string_const() {
        let schema = serde_json::to_value(schemars::schema_for!(Blob<Jpeg>)).unwrap();
        assert_eq!(schema["properties"]["media_type"]["const"], "image/jpeg");
        let vague = serde_json::to_value(schemars::schema_for!(Blob<VagueText>)).unwrap();
        assert_eq!(vague["properties"]["media_type"].get("const"), None);
        assert_eq!(
            vague["properties"]["description"]["default"],
            "Some vague text."
        );
    }
}
