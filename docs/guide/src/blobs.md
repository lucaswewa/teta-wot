# Blobs, MJPEG and binary streams

## Blobs

A Blob is binary data an action returns or takes: an image, a data file, an archive. Rather than putting the bytes in JSON, the server serves them at their own URL, and the invocation's output links to them:

```json
{"href": "http://127.0.0.1:5000/blob/0f6c…", "media_type": "image/jpeg", "rel": "output", "description": "…"}
```

`Blob<M>` is typed by its media type (`Jpeg`, `Png`, `TextPlain`, … in `teta_wot::blob`, or your own `MediaType`):

```rust,ignore
{{#include ../../../examples/simulated-microscope/src/lib.rs:capture}}
```

A Blob's data can be:

- bytes in memory (`Blob::from_bytes`);
- a file (`from_file`);
- a file in a temporary folder that goes with it (`from_temporary_directory`);
- a URL elsewhere (`from_url`).

The data lives as long as something holds the Blob. An invocation holds its output until the invocation expires (5 minutes by default), so a client can download it until then: `GET /blob/{id}`, or `GET /action_invocations/{id}/output`.

**As an input,** a Blob is sent as its link. A client passes back a Blob it downloaded, and the action receives the same data without it crossing the network again.

## MJPEG streams

A live camera preview is an MJPEG stream: a `multipart/x-mixed-replace` response of JPEG frames, which an `<img>` element plays.

```rust,ignore
/// The live preview, as MJPEG.
#[stream]
preview: MjpegStream,
```

- **Serving.** It is served at `/camera/preview`, with a viewer page at `/camera/preview/viewer`, and the TD links to both.
- **Feeding.** A thread (or a device's callback) adds frames with `add_frame(jpeg)`. That never waits, whatever the viewers are doing. Each viewer gets the newest frame when it is ready for one, so a slow viewer skips frames rather than slowing the others.
- **Waiting for a frame.** Actions can wait for the next frame (`grab_frame`, `next_frame_size`), for example to capture an image or measure sharpness.
- **Stopping.** `stop()` ends every viewer's response, and so does the server's shutdown.

The simulated microscope's camera feeds its preview from a thread started in `#[on_start]` and stopped in `#[on_stop]`:

```rust,ignore
{{#include ../../../examples/simulated-microscope/src/lib.rs:preview}}
```

## Binary streams

An MJPEG stream sends every viewer the same JPEG frames. A **binary stream** gives each client its own stream instead: a WebSocket of binary messages, made for that client by a producer that runs for as long as the client stays connected. It suits frames of a size each client chooses, live data, or anything that isn't JPEG.

A `#[binary_stream]` method is the producer. It takes the connection's `Subscriber<P>` (`teta_wot::http::Subscriber`), where `P` is the parameters the client sends:

```rust,ignore
/// What a client asks for.
#[derive(Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
struct FrameParams {
    /// The size of each frame, in bytes.
    size: usize,
    /// Frames per second.
    rate: f64,
}

#[thing_impl]
impl Camera {
    /// Frames, of the size and at the rate each client asks for.
    #[binary_stream]
    async fn frames(&self, mut subscriber: Subscriber<FrameParams>) {
        'parameters: loop {
            let FrameParams { size, rate } = subscriber.params();
            let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / rate));
            loop {
                tokio::select! {
                    changed = subscriber.changed() => {
                        if changed { continue 'parameters } else { break 'parameters }
                    }
                    _ = tick.tick() => { subscriber.send(self.frame(size)); }
                }
            }
        }
    }
}
```

**Where it is.** The stream is at `/{thing}/frames` (or `#[binary_stream("path")]`), and clients connect with a WebSocket.

**Parameters.** A client sets its parameters by sending JSON text messages. Each message replaces the parameters: `#[serde(default)]` lets a client send only the fields it changes. A message that doesn't deserialise into `P` is answered with `{"error": …}` and changes nothing.

**Sending.** `send` puts a message in a one-slot sink, and never waits:

- A message the client hasn't taken yet is replaced by the next one, and counted as dropped.
- So a client that falls behind gets the newest message, and never holds up the producer or other clients. Messages small enough to fit the connection's buffers aren't dropped.
- `sent()` and `dropped()` count both.

**The end of a connection.** It ends when the client closes it or the server shuts down:

- `closed()` completes, and `changed()` returns `false`;
- the producer has a second to return before its task is cancelled.

**The Thing Description** links each binary stream, with the `subprotocol` `websocket`, the media type `application/octet-stream`, and `P`'s schema under `parameters`.

Without the macros, `Endpoint::binary_stream(path, producer)` makes the same endpoint. The [`binary-stream`](https://github.com/lucaswewa/teta-wot/tree/main/examples/binary-stream) example streams 1 MB frames at 60 Hz to four clients, beside a slow one that misses frames.
