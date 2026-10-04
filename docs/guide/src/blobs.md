# Blobs and MJPEG streams

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
