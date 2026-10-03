//! Blobs and MJPEG streams over HTTP.
//!
//! - `GET {prefix}/blob/{id}` serves a Blob's data, from memory or streamed
//!   from its file, with its media type (Starlette adds `charset=utf-8` to
//!   `text/*` types, and so does this).
//! - An invocation's `/output` serves the data itself when the output is
//!   one Blob, and redirects to a remote Blob's URL.
//! - Blob `href`s are written by the core as `blob/<id>`; every JSON
//!   response makes them absolute, with the request's host and the API
//!   prefix.
//! - `GET {prefix}/{thing}/{stream}` is MJPEG response, and
//!   `…/viewer` its page.

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Body;
use axum::response::Response;
use bytes::{Bytes, BytesMut};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderValue, StatusCode};
use serde_json::Value;
use teta_wot_core::blob::{BlobContent, BlobData, local_href_id};
use teta_wot_core::{MessageBroker, MjpegStream};
use tokio::io::AsyncReadExt;

use crate::render::{Urls, detail, redirect};

/// The size of the chunks a file is streamed in.
const CHUNK: usize = 64 * 1024;

/// The content type of an MJPEG response.
const MJPEG_CONTENT_TYPE: &str = "multipart/x-mixed-replace; boundary=frame";

/// What precedes each frame of an MJPEG response.
const FRAME_HEADER: &[u8] = b"--frame\r\nContent-Type: image/jpeg\r\n\r\n";

/// A `Content-Type` as Starlette writes it: `text/*` types get
/// `charset=utf-8` unless they have a charset.
fn content_type(media_type: &str) -> HeaderValue {
    let value = if media_type.starts_with("text/")
        && !media_type.to_ascii_lowercase().contains("charset=")
    {
        format!("{media_type}; charset=utf-8")
    } else {
        media_type.to_owned()
    };
    HeaderValue::from_str(&value).unwrap_or(HeaderValue::from_static("application/octet-stream"))
}

/// A Blob's data as a response: bytes, a file streamed in chunks, or a 307
/// to a remote Blob's URL.
pub(crate) async fn blob_response(data: Arc<BlobData>) -> Response {
    let body = match data.content() {
        BlobContent::Remote(href) => return redirect(href),
        BlobContent::Bytes(bytes) => (Body::from(bytes.clone()), Some(bytes.len() as u64)),
        BlobContent::File(path) => {
            let file = match tokio::fs::File::open(path).await {
                Ok(file) => file,
                Err(error) => {
                    tracing::error!("can't read the file of Blob {}: {error}", data.id());
                    return detail(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
                }
            };
            let length = file.metadata().await.ok().map(|m| m.len());
            // The stream holds the data, so a temporary directory outlives
            // the download.
            let chunks = futures_util::stream::unfold(
                (file, Arc::clone(&data)),
                |(mut file, data)| async move {
                    let mut buffer = BytesMut::zeroed(CHUNK);
                    match file.read(&mut buffer).await {
                        Ok(0) => None,
                        Ok(n) => {
                            buffer.truncate(n);
                            Some((Ok(buffer.freeze()), (file, data)))
                        }
                        Err(error) => Some((Err(error), (file, data))),
                    }
                },
            );
            (Body::from_stream(chunks), length)
        }
        _ => return detail(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"),
    };
    let (body, length) = body;
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, content_type(data.media_type()));
    if let Some(length) = length {
        headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    }
    response
}

/// Makes the Blob `href`s in a JSON value absolute: `blob/<id>` becomes
/// `{scheme}://{host}{prefix}/blob/<id>`, for Blobs that still exist.
pub(crate) fn resolve_blobs(value: &mut Value, urls: &Urls) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(href)) = map.get_mut("href")
                && let Some(id) = local_href_id(href)
            {
                *href = urls.absolute(&format!("{}/blob/{id}", urls.prefix));
            }
            map.values_mut().for_each(|v| resolve_blobs(v, urls));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| resolve_blobs(v, urls)),
        _ => {}
    }
}

/// MJPEG response: each frame, as it comes, after `--frame` and
/// its content type. It ends when the stream stops or the server shuts
/// down; a slow client skips frames.
pub(crate) fn mjpeg_response(stream: &MjpegStream, broker: Arc<MessageBroker>) -> Response {
    let frames = futures_util::stream::unfold(
        (stream.frames(), broker),
        |(mut frames, broker)| async move {
            let frame = tokio::select! {
                frame = frames.next() => frame?,
                () = broker.closed() => return None,
            };
            let mut chunk = BytesMut::with_capacity(FRAME_HEADER.len() + frame.data.len() + 2);
            chunk.extend_from_slice(FRAME_HEADER);
            chunk.extend_from_slice(&frame.data);
            chunk.extend_from_slice(b"\r\n");
            Some((Ok::<Bytes, Infallible>(chunk.freeze()), (frames, broker)))
        },
    );
    let mut response = Response::new(Body::from_stream(frames));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(MJPEG_CONTENT_TYPE));
    response
}

/// viewer page: an `<img>` of the stream.
pub(crate) fn viewer_page(stream_path: &str) -> Response {
    let mut response = Response::new(Body::from(format!(
        "<html><body><img src='{stream_path}'></body></html>"
    )));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_types_get_a_charset() {
        assert_eq!(content_type("text/plain"), "text/plain; charset=utf-8");
        assert_eq!(
            content_type("text/csv; charset=latin-1"),
            "text/csv; charset=latin-1"
        );
        assert_eq!(content_type("image/jpeg"), "image/jpeg");
    }
}
