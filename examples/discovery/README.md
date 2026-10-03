# discovery

**Concept:** being found (W3C WoT Discovery.

- The server advertises itself on the local network with DNS-SD over mDNS, as a Thing Description Directory (`_directory._sub._wot._tcp`).
- It serves the directory's TD at `/.well-known/wot`, and its Things' TDs at `/directory/things`.
- A client that knows nothing about it browses for directories, reads the well-known URL, and follows the directory's `things` form to every TD.

**Technologies:**

- the `mdns` feature and `ThingServerBuilder::mdns(true)` (or `"mdns": true` in a configuration file);
- the `mdns-sd` crate for browsing, in Rust;
- the `zeroconf` and `httpx` packages, in Python.

## Run it

```
cargo run -p discovery
```

This does both in one process: it serves two Things with mDNS advertising, browses the network, finds itself, and lists the TDs. It exits with a non-zero status if the advertisement isn't found within 20 seconds, which needs multicast on the network.

To be found by something else, serve until Ctrl-C:

```
cargo run -p discovery -- --serve
```
