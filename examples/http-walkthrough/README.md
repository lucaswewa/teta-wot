# http-walkthrough

**Concept:** the whole HTTP API of a Thing: every route, status code and header a client meets, over a real socket.

**Technologies:** `wot::server::ThingServer` on a `tokio` TCP listener, and `curl` in bash (`walkthrough.sh`) or PowerShell (`walkthrough.ps1`, using the `curl.exe` that ships with Windows).

## Run it

```
cargo run -p http-walkthrough                 # serves on a free port, walks through it, exits
```

Or serve on port 5000 and run a script against it from another terminal:

```
cargo run -p http-walkthrough -- --serve
bash examples/http-walkthrough/walkthrough.sh            # needs curl and jq
powershell -File examples/http-walkthrough/walkthrough.ps1
```
