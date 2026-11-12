# Working on teta-wot

`teta-wot` is a Rust framework for W3C Web of Things servers: Things with properties, actions and events, served over HTTP. Applications depend on the `teta-wot` crate (`teta_wot::` in code); the other crates in `crates/` sit behind it.

- **The guide** is in [`docs/guide/`](docs/guide/src/SUMMARY.md) (mdBook).
- **The examples** are in [`examples/`](examples/README.md); each one teaches one thing.
- **Releases** follow [`docs/releasing.md`](docs/releasing.md).
- **What changed for users** is recorded in [`CHANGELOG.md`](CHANGELOG.md).

## Delivering a change

- **One branch and one pull request per change.** The owner approves every pull request before it is merged.
  - Never merge without that approval.
  - Never push to `main`, and never create a tag, without the owner's word.
- **Phases driven by OScope.** Work planned in [OScope](https://github.com/lucaswewa/OScope) uses the branch `u/uNN-short-name` and the pull-request title `UNN: <title>`.
  - The spec is in OScope's `docs/milestone-1/phases.md`.
  - The phase's notes, with its design decisions, go into OScope (`docs/milestone-1/notes/UNN-short-name.md`), not here.
  - Here go the code, its tests, an example of the feature, the CHANGELOG entry and the guide pages.
- **Every user-facing change** gets an entry under `## [Unreleased]` in `CHANGELOG.md`, saying what changed for users. A breaking change says how to adapt.
- **Breaking changes are avoided where possible.** These surfaces are stable within a minor version:
  - the `teta-wot` crate's API;
  - the wire profiles;
  - configuration files and the command line;
  - Thing Description `id`s;
  - settings files;
  - problem types;
  - the minimum Rust version.

## Identifiers that must not change

They look like leftovers of the old `wot` name, but they are part of the wire format or of users' setups:

- **The namespace URL that Thing Description `id`s are derived from** (`https://github.com/lucaswewa/wot#td-id`, in `teta-wot-http`). Changing it changes every TD `id`.
- **`PROBLEM_TYPES_URL`,** the base of every problem `type` (`https://github.com/lucaswewa/teta-wot/blob/main/docs/problems.md`, in `teta-wot-http`). Each type there has a heading in `docs/problems.md`, which a test checks.
- **The log target `wot::things`,** which users filter on.
- **The `wot` wire profile's name.**

Changing any of them is a breaking change, made only on purpose, with migration notes.

## Rules

- **Platforms.**
  - Windows and Linux, on x86_64. CI runs every job on `windows-latest` and `ubuntu-24.04`.
  - Platform-specific code lives in small `cfg` blocks, each with an equivalent elsewhere or a clear message.
- **Line endings.** The repository stores LF.
- **Unsafe code** is forbidden in the workspace (`unsafe_code = "forbid"`).
- **Dependencies:**
  - Their licences must be MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 or CC0. Anything else needs the owner's decision.
  - Use versions at least two weeks old.
- **Tests:**
  - Wait for a condition, never a fixed time.
  - Give each test its own folder and port: port 0, or a temporary directory.

## Checks before opening a pull request

These match CI (`.github/workflows/ci.yml`):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo test --locked -p teta-wot-http -p teta-wot --lib
for example in examples/*/; do cargo run --quiet --locked --manifest-path "$example/Cargo.toml"; done
mdbook build docs/guide
```

**On Windows, check Linux too, under WSL** (Ubuntu 24.04, with `build-essential` and rustup installed). Run the same commands from the Windows checkout, building into a Linux-side folder, so the Windows and Linux builds never share `target/`:

```sh
cd /mnt/c/repos/teta-wot
CARGO_TARGET_DIR=~/target/teta-wot cargo test --workspace --all-features --locked
```
