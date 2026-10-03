//! MJPEG streams: JPEG frames pushed from any thread, and watched by any
//! number of clients.
//!
//! A Thing holds an [`MjpegStream`] in a field marked `#[stream]` (or added
//! with `ThingDefinition::stream`). The HTTP binding serves it at
//! `/{thing}/{stream}` as `multipart/x-mixed-replace`, which a browser's
//! `<img>` plays, with a viewer page at `/{thing}/{stream}/viewer`.
//!
//! - [`add_frame`](MjpegStream::add_frame) never waits, and works from a
//!   camera's thread. It checks the JPEG start and end markers.
//! - The last frames (10 by default) are kept in a ring buffer, by index.
//! - Waiting for a frame ([`next_frame`](MjpegStream::next_frame),
//!   [`grab_frame`](MjpegStream::grab_frame),
//!   [`next_frame_size`](MjpegStream::next_frame_size)) waits for the next
//!   one to arrive.
//! - Each client gets the latest frame whenever it is ready for one; a slow
//!   client skips frames, and never holds up the camera or other clients.
//! - [`stop`](MjpegStream::stop) ends every client's stream;
//!   [`reset`](MjpegStream::reset) starts again.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use tokio::sync::watch;

/// The media type of an MJPEG stream's HTTP response, without its
/// `boundary=frame` parameter.
pub const MJPEG_MEDIA_TYPE: &str = "multipart/x-mixed-replace";

/// default ring buffer size.
pub const DEFAULT_BUFFER: usize = 10;

/// One frame of a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The JPEG data.
    pub data: Bytes,
    /// When the frame was captured, or else when it was added.
    pub timestamp: DateTime<Utc>,
    /// The frame's index: 0 for the first frame after the stream was made
    /// or reset, then counting up.
    pub index: u64,
}

/// A stream operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StreamError {
    /// The frame doesn't start and end with the JPEG markers.
    #[error("Invalid JPEG")]
    InvalidJpeg,
    /// The stream was stopped while waiting.
    #[error("the stream has stopped")]
    Stopped,
    /// The frame has left the ring buffer.
    #[error("the ith frame has been overwritten")]
    Overwritten,
    /// The frame hasn't been added yet.
    #[error("the ith frame has not yet been acquired")]
    NotYetAcquired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    /// The index of the latest frame, if any.
    last: Option<u64>,
    streaming: bool,
}

struct Ring {
    frames: Vec<Option<Frame>>,
    last: Option<u64>,
}

struct Shared {
    ring: Mutex<Ring>,
    state: watch::Sender<State>,
}

/// An MJPEG stream. A cheap handle: clones are
/// the same stream, so a camera thread can have its own.
///
/// ```
/// use teta_wot_core::MjpegStream;
///
/// let stream = MjpegStream::new();
/// let camera = stream.clone();
/// std::thread::spawn(move || camera.add_frame(vec![0xff, 0xd8, 0xff, 0xd9]))
///     .join()
///     .unwrap()?;
/// assert_eq!(stream.latest_frame().unwrap().data.len(), 4);
/// # Ok::<(), teta_wot_core::stream::StreamError>(())
/// ```
#[derive(Clone)]
pub struct MjpegStream {
    shared: Arc<Shared>,
}

impl fmt::Debug for MjpegStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = *self.shared.state.borrow();
        f.debug_struct("MjpegStream")
            .field("buffer", &self.ring().frames.len())
            .field("last", &state.last)
            .field("streaming", &state.streaming)
            .finish()
    }
}

impl Default for MjpegStream {
    fn default() -> Self {
        Self::new()
    }
}

impl MjpegStream {
    /// A stream keeping the last [`DEFAULT_BUFFER`] (10) frames.
    pub fn new() -> Self {
        Self::with_buffer(DEFAULT_BUFFER)
    }

    /// A stream keeping the last `size` frames (at least one).
    pub fn with_buffer(size: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                ring: Mutex::new(Ring {
                    frames: vec![None; size.max(1)],
                    last: None,
                }),
                state: watch::Sender::new(State {
                    last: None,
                    streaming: true,
                }),
            }),
        }
    }

    fn ring(&self) -> MutexGuard<'_, Ring> {
        self.shared.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a JPEG frame, timestamped now, and returns its index. Never
    /// waits: call it from any thread.
    pub fn add_frame(&self, frame: impl Into<Bytes>) -> Result<u64, StreamError> {
        self.add_frame_at(frame, Utc::now())
    }

    /// Adds a JPEG frame captured at `timestamp`.
    pub fn add_frame_at(
        &self,
        frame: impl Into<Bytes>,
        timestamp: DateTime<Utc>,
    ) -> Result<u64, StreamError> {
        let data = frame.into();
        let n = data.len();
        if n < 2 || data[..2] != [0xff, 0xd8] || data[n - 2..] != [0xff, 0xd9] {
            return Err(StreamError::InvalidJpeg);
        }
        let index = {
            let mut ring = self.ring();
            let index = ring.last.map_or(0, |last| last + 1);
            let slot = (index % ring.frames.len() as u64) as usize;
            ring.frames[slot] = Some(Frame {
                data,
                timestamp,
                index,
            });
            ring.last = Some(index);
            index
        };
        self.shared
            .state
            .send_modify(|state| state.last = Some(index));
        Ok(index)
    }

    /// The frame with this index, if it is still in the ring buffer. The
    /// oldest slot counts as overwritten, as the next frame is about to replace it.
    pub fn frame(&self, index: u64) -> Result<Frame, StreamError> {
        let ring = self.ring();
        let Some(last) = ring.last.filter(|last| index <= *last) else {
            return Err(StreamError::NotYetAcquired);
        };
        let size = ring.frames.len() as u64;
        if index + size < last + 2 {
            return Err(StreamError::Overwritten);
        }
        match &ring.frames[(index % size) as usize] {
            Some(frame) if frame.index == index => Ok(frame.clone()),
            _ => Err(StreamError::Overwritten),
        }
    }

    /// The latest frame, if there is one.
    pub fn latest_frame(&self) -> Option<Frame> {
        let ring = self.ring();
        let last = ring.last?;
        ring.frames[(last % ring.frames.len() as u64) as usize].clone()
    }

    /// Waits for the next frame, and returns its index. On a stopped stream
    /// it fails at once.
    pub async fn next_frame(&self) -> Result<u64, StreamError> {
        let mut state = self.shared.state.subscribe();
        if !state.borrow().streaming {
            return Err(StreamError::Stopped);
        }
        loop {
            state.changed().await.map_err(|_| StreamError::Stopped)?;
            let current = *state.borrow_and_update();
            if !current.streaming {
                return Err(StreamError::Stopped);
            }
            if let Some(last) = current.last {
                return Ok(last);
            }
        }
    }

    /// Waits for the next frame, and returns it.
    pub async fn grab_frame(&self) -> Result<Bytes, StreamError> {
        let index = self.next_frame().await?;
        Ok(self.frame(index)?.data)
    }

    /// Waits for the next frame, and returns its size in bytes (a
    /// sharpness measure, for autofocus).
    pub async fn next_frame_size(&self) -> Result<usize, StreamError> {
        let index = self.next_frame().await?;
        Ok(self.frame(index)?.data.len())
    }

    /// Stops the stream: every client's stream ends, and waiters get
    /// [`StreamError::Stopped`].
    pub fn stop(&self) {
        self.shared
            .state
            .send_modify(|state| state.streaming = false);
    }

    /// Empties the ring buffer, optionally resizes it, restarts the index
    /// at 0, and streams again.
    pub fn reset(&self, size: Option<usize>) {
        {
            let mut ring = self.ring();
            let size = size.unwrap_or(ring.frames.len()).max(1);
            ring.frames = vec![None; size];
            ring.last = None;
        }
        self.shared.state.send_replace(State {
            last: None,
            streaming: true,
        });
    }

    /// Whether the stream is streaming (not stopped).
    pub fn is_streaming(&self) -> bool {
        self.shared.state.borrow().streaming
    }

    /// Receives the frames added from now on, as a client of the stream
    /// does. A slow receiver gets the latest frame when it asks, skipping
    /// the ones in between.
    pub fn frames(&self) -> FrameReceiver {
        FrameReceiver {
            stream: self.clone(),
            state: self.shared.state.subscribe(),
        }
    }
}

/// Receives a stream's frames: see [`MjpegStream::frames`].
#[derive(Debug)]
pub struct FrameReceiver {
    stream: MjpegStream,
    state: watch::Receiver<State>,
}

impl FrameReceiver {
    /// The next frame, or `None` once the stream is stopped.
    pub async fn next(&mut self) -> Option<Frame> {
        if !self.state.borrow().streaming {
            return None;
        }
        loop {
            self.state.changed().await.ok()?;
            let state = *self.state.borrow_and_update();
            if !state.streaming {
                return None;
            }
            if state.last.is_some()
                && let Some(frame) = self.stream.latest_frame()
            {
                return Some(frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(n: u8) -> Vec<u8> {
        vec![0xff, 0xd8, n, 0xff, 0xd9]
    }

    #[test]
    fn frames_must_be_jpegs() {
        let stream = MjpegStream::new();
        for bad in [
            vec![],
            vec![0xff],
            vec![0xff, 0xd8],
            vec![0xff, 0xd8, 0xd9],
            vec![0, 0xd8, 0xff, 0xd9],
        ] {
            assert_eq!(stream.add_frame(bad), Err(StreamError::InvalidJpeg));
        }
        assert_eq!(stream.add_frame(vec![0xff, 0xd8, 0xff, 0xd9]), Ok(0));
    }

    #[test]
    fn the_ring_buffer_keeps_the_last_frames_by_index() {
        let stream = MjpegStream::with_buffer(3);
        for n in 0..5 {
            assert_eq!(stream.add_frame(jpeg(n)), Ok(u64::from(n)));
        }
        assert_eq!(stream.frame(5), Err(StreamError::NotYetAcquired));
        assert_eq!(stream.frame(4).unwrap().data[2], 4);
        assert_eq!(stream.frame(3).unwrap().data[2], 3);
        // The oldest slot is about to be overwritten.
        assert_eq!(stream.frame(2), Err(StreamError::Overwritten));
        assert_eq!(stream.latest_frame().unwrap().index, 4);
        stream.reset(Some(2));
        assert_eq!(stream.latest_frame(), None);
        assert_eq!(stream.add_frame(jpeg(9)), Ok(0));
    }

    #[tokio::test]
    async fn waiters_get_the_next_frame_or_stopped() {
        let stream = MjpegStream::new();
        let waiter = tokio::spawn({
            let stream = stream.clone();
            async move { stream.next_frame_size().await }
        });
        tokio::task::yield_now().await;
        let camera = stream.clone();
        std::thread::spawn(move || camera.add_frame(jpeg(1)).unwrap())
            .join()
            .unwrap();
        assert_eq!(waiter.await.unwrap(), Ok(5));

        let waiter = tokio::spawn({
            let stream = stream.clone();
            async move { stream.grab_frame().await }
        });
        tokio::task::yield_now().await;
        stream.stop();
        assert_eq!(waiter.await.unwrap(), Err(StreamError::Stopped));
    }

    #[tokio::test]
    async fn receivers_get_the_latest_frame_and_end_when_stopped() {
        let stream = MjpegStream::new();
        let mut receiver = stream.frames();
        for n in 0..3 {
            stream.add_frame(jpeg(n)).unwrap();
        }
        // A slow receiver skips to the latest frame.
        assert_eq!(receiver.next().await.unwrap().index, 2);
        stream.stop();
        assert_eq!(receiver.next().await, None);
        let mut late = stream.frames();
        assert_eq!(late.next().await, None, "a stopped stream has nothing");
        stream.reset(None);
        let mut again = stream.frames();
        stream.add_frame(jpeg(7)).unwrap();
        assert_eq!(again.next().await.unwrap().data[2], 7);
    }
}
