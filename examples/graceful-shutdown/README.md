# graceful-shutdown

**Concept:** stopping a server without leaving hardware in a bad state. A stop request cancels running actions cooperatively, gives them time to finish, then stops the Things and closes their devices, in reverse order of starting.

**Technologies:** `ThingServer::serve_with` and `teta_wot::server::shutdown_signal` (Ctrl-C, Ctrl-Break, closing the console window, log-off and system shutdown on Windows, through `tokio::signal::windows`), `Device`, and `Thing::start`/`stop`.

## Run it

```
cargo run -p graceful-shutdown             # stops itself after about 600 ms
cargo run -p graceful-shutdown -- --wait   # waits for you to press Ctrl-C
```

It prints a timestamped line for each lifecycle event and exits with a non-zero status if the running action wasn't cancelled.

A typical run:

```
[    0 ms] building the server
[    2 ms] serving on http://127.0.0.1:52240
[    2 ms] left: shutter opened (on "device:left:shutter")
[    2 ms] left: started
[    2 ms] right: shutter opened (on "device:right:shutter")
[    3 ms] right: started
[  108 ms] left: time lapse started
[  606 ms] shutdown requested
[  606 ms] left: time lapse cancelled after 4 frames; finishing the file
[  607 ms] right: stopping
[  607 ms] right: shutter closed
[  607 ms] left: stopping
[  607 ms] left: shutter closed
[  607 ms] left's time lapse ended as `cancelled`
```
