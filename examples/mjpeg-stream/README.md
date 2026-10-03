# mjpeg-stream

**Concept:** MJPEG streams. A simulated camera thread feeds JPEG frames into a stream, which any number of browsers play in an `<img>`. Actions wait for frames: one measures the next frame's size, an autofocus sharpness measure, and another saves a frame as a Blob.

**Technologies:** `#[stream]` on an `MjpegStream` field; `add_frame` from a plain `std::thread`; `next_frame_size` and `grab_frame`; `multipart/x-mixed-replace`; `#[on_start]` and `#[on_stop]` hooks; `jpeg-encoder` for the frames.

## Run it

```
cargo run -p mjpeg-stream                      # an in-process demo, then exit
cargo run -p mjpeg-stream -- --serve           # serve on http://127.0.0.1:5000 until Ctrl-C
```

While it serves, open <http://127.0.0.1:5000/camera/preview/viewer>. The bar sweeps across, and the counter along the top counts frames in binary. The stream itself is at <http://127.0.0.1:5000/camera/preview>. Change the frame rate with `curl -X PUT -H "Content-Type: application/json" -d 25 http://127.0.0.1:5000/camera/fps`.
