# dependency-injection

**Concept:** sharing a service between Things, and reading the server from an action. A record store is registered once with the server and injected into the actions that need it. Each record embeds the state of every Thing and the application configuration.

**Technologies:** `ThingServerBuilder::service`, `Dep<T>` and `Server` action parameters, `#[thing_state]`, `Server::thing_states` and `Server::application_config`.

## Run it

```
cargo run -p dependency-injection
```

It invokes the actions over HTTP and in-process, prints the records, then shows the error when the service is missing. It exits with a non-zero status if a record isn't as expected.
