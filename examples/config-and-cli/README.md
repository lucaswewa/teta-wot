# config-and-cli

**Concept:** an application binary that serves Things from a configuration files. The application registers the Thing types that its files may name; everything else is the configuration's.

**Technologies:** `ThingRegistry`, `teta_wot::server::cli::serve_from_cli` (`-c`, `-j`, `--host`, `--port`, `--fallback`, `--debug`), and the fallback server.

## Run it

```
cargo run -p config-and-cli                                                   # a demonstration, then exit
cargo run -p config-and-cli -- -c examples/config-and-cli/config.json         # serve until Ctrl-C
cargo run -p config-and-cli -- -c examples/config-and-cli/failing.json --fallback
```

The demonstration runs the command line in-process three times: with a working configuration, with a Thing that can't start and `--fallback`, and with an unknown class. It prints what the server answered and the exit codes, and exits with a non-zero status if one isn't as expected.
