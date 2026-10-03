# node-wot-consumer

**Concept:** interoperability with another WoT implementation. [Eclipse Thingweb node-wot](https://github.com/eclipse-thingweb/node-wot) 0.9.2, in JavaScript, consumes a Thing served by `teta-wot`, knowing only its TD: it reads and writes a property, invokes an action and observes the property.

**Technologies:**

- node-wot's `Servient` and HTTP binding, from npm (`package.json`, with `package-lock.json`);
- Node.js;
- the two wire profiles.

## Run it

The Rust side checks, in-process, that both TDs have the forms node-wot uses:

```
cargo run -p node-wot-consumer
```

With Node.js installed:

```
cd examples/node-wot-consumer
npm ci
node consumer.mjs --spawn ../../target/debug/node-wot-consumer.exe
```

`--spawn` starts the server on free ports and stops it afterwards. Without it, the script uses a server started with `cargo run -p node-wot-consumer -- --serve`, which serves:

- the `tetathing` profile on port 8000;
- the `wot` profile on 8001.

The script prints what worked in each profile, and exits with a non-zero status if that isn't what is described below. CI runs it without gating on it.
