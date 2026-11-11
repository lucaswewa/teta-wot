# binary-stream

**Concept:** a binary stream, a WebSocket of binary messages for each client, of the size and at the rate each client asks for. A client that falls behind gets the newest message, and never holds up the producer or the other clients. It also measures throughput.

**Technologies:**

- `#[binary_stream]` in `#[thing_impl]`;
- the `teta_wot::http::Subscriber` (`params`, `changed`, `send`, `sent`, `dropped`);
- parameters as a `Deserialize + JsonSchema` type, which the TD link describes;
- a `tokio-tungstenite` WebSocket client.

## Run it

```
cargo run --release -p binary-stream                 # 3 seconds
cargo run --release -p binary-stream -- --seconds 10
```

A simulated camera streams frames to five clients:

- **Four fast clients** take the default parameters: 1 MB frames at 60 Hz, about 250 MB/s in all. This is the benchmark.
- **A slow client** first sends bad parameters and prints the error that comes back, then reads one frame every 200 ms.

It prints what each client received, and the server prints, for each client that leaves, how many frames it sent and dropped. It exits with a non-zero status if a fast client gets fewer than 30 frames/s.

On the owner's Windows laptop (release build):

| Client | Received | Server sent | Server dropped |
|---|---|---|---|
| Each fast client | 60.0 frames/s, 62.9 MB/s | 181 | 0 |
| The slow client | 5.0 frames/s | 17 | 164 |
