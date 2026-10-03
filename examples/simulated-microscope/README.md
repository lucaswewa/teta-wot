# simulated-microscope

**Concept:** the capstone. A stage, a camera and an autofocus, composed with slots, with settings, an MJPEG stream, Blobs, events and arrays, served from a configuration file and driven by a Python client.

- **`Stage`**
  - moves x, y and z at a `speed` that is a setting;
  - each move takes time, in small steps, and can be cancelled;
  - it emits `arrived`.
- **`Camera`** looks at the slide through the stage (a slot). A thread renders what it sees into an MJPEG preview: a field of cells, sharp at z = 250 and blurred away from it.
  - `capture` returns a JPEG Blob;
  - `sharpness` measures the image;
  - `exposure` is a setting;
  - it emits `captured`.
- **`Autofocus`** sweeps the stage's z (one slot) through the camera's sharpness (another), moves to the sharpest position, returns the focus curve as an `NdArray`, and emits `focused`.

**Technologies:**

- `#[slot]`, `#[setting]`, `#[stream]`, `#[event]`, `Blob<Jpeg>`, `NdArray<f64, Ix2>`, `#[on_start]`/`#[on_stop]`, a feeding thread;
- `ThingRegistry` and `cli::serve_from_cli`;
- `jpeg-encoder`;
- `ThingClient` and `httpx` in `demo.py`.

The library (`src/lib.rs`) is shared by [`microscope-service`](../microscope-service/README.md) and the soak tool (`tools/soak`).

## Run it

```
cargo run -p simulated-microscope
```

Without arguments, it runs the demonstration in-process and exits with a non-zero status if a step fails:

1. it moves the stage and changes a setting;
2. it captures and downloads an image;
3. it runs the autofocus, and receives its event;
4. it reads two preview frames.

With arguments, it is a Thing server with command line:

```
cargo run -p simulated-microscope -- -c examples/simulated-microscope/microscope.json
```

Then:

- <http://127.0.0.1:5000/camera/preview/viewer> shows the live preview (move the stage from <http://127.0.0.1:5000/docs> and watch it change);
- `demo.py` runs a scripted session with a Python client:

  ```
  uv run --locked python examples/simulated-microscope/demo.py
  uv run --locked python examples/simulated-microscope/demo.py --spawn target/debug/simulated-microscope.exe
  ```

  The first form uses the running server. With `--spawn` (as CI runs it), it starts one on a free port with a temporary settings folder. It saves the captured image as `capture.jpg`.
