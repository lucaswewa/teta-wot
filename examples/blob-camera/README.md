# blob-camera

**Concept:** Blobs, for binary data that JSON can't carry. A simulated camera returns its captures as JPEG Blobs, one from memory and one from a file in a temporary directory. Clients download them from `/blob/{id}`, and pass them back as the input of another action.

**Technologies:** `Blob<Jpeg>` as an action's output and input; `Blob::from_bytes` and `Blob::from_temporary_directory`; `jpeg-encoder` for the simulated images.

## Run it

```
cargo run -p blob-camera                      # an in-process demo, then exit
cargo run -p blob-camera -- --serve           # serve on http://127.0.0.1:5000 until Ctrl-C
```
