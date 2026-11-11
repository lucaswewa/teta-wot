# custom-auth

**Concept:** the application's own authentication and authorisation. It decides who is calling and what each caller may do, and its Things know who called.

**Technologies:**

- an `Authenticator` that turns API keys into `Principal`s with a role;
- an `Authorizer` that lets operators do anything and viewers only what changes nothing (`Access::changes_state`);
- `ThingServerBuilder::authenticator` and `ThingServerBuilder::authorizer`;
- `ActionCtx::principal`, to sign a logbook's entries.

## Run it

```
cargo run -p custom-auth
```

Three callers each read the logbook and try to write in it:

- **nobody**, without a key, gets `401` for both;
- **ben**, a viewer, can read but gets `403` for writing;
- **ada**, an operator, can do both, and the entry is signed `ada`.

It prints each answer and the logbook, and exits with a non-zero status if an answer is wrong.
