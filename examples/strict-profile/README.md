# strict-profile

**Concept:** the two wire profiles.

- **`tetathing`**, the default behavior.
- **`wot`** follows the W3C WoT HTTP Basic and SSE Profiles where the two conflict.

One Thing is served in both, and the same requests go to each.

**Technologies:** `ThingServerBuilder::wire_profile(WireProfile::Wot)` (or `"wire_profile": "wot"` in a configuration file); `#[action(synchronous)]`; `wot::testing::TestClient`.

## Run it

```
cargo run -p strict-profile
```

It prints each request with both answers (status, content type, body), and exits with a non-zero status if they don't differ as described below. No network is used.
