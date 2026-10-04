# Introduction

`teta-wot` makes laboratory instruments, and the software around them, available over HTTP as [W3C Web of Things](https://www.w3.org/WoT/) *Things*.

Its key features and aims are:

- **Things describe units of hardware or software.** A Thing is a Rust struct, marked with `#[derive(Thing)]`.
- **Properties and actions** are fields and methods, marked with attributes, and are added to the HTTP API and its documentation from those attributes alone.
- **Types describe the data.** The types of properties, action inputs and outputs are Rust types, whose JSON Schemas become the Thing Description's data schemas and the OpenAPI document's.
- **The lifecycle and concurrency suit hardware:**
  - each Thing is created, started and stopped once;
  - blocking driver code runs on threads of its own;
  - long actions can be cancelled;
  - an optional global lock keeps actions from overlapping.
- **The vocabulary and concepts are the W3C Web of Things'**: every Thing has a [Thing Description](wot_core_concepts.md), and a stricter wire profile follows the W3C WoT Profile.

And some of its own:

- **More of the Web of Things:**
  - events;
  - observation described in Thing Descriptions;
  - the W3C WoT Profile, as a [wire profile](wire_profiles.md);
  - [discovery](discovery.md) and [security](security.md).
- **A Windows service.** A server can [run as a Windows service](windows_service.md) and stop gracefully with it.
- **One binary.** A server is a single Windows executable, with no Python environment to install.

## Where to start

- [Quickstart](quickstart.md): a counter Thing, served in a few minutes.
- [Tutorial](tutorial/index.md): installing, running a server from a configuration file, and writing a Thing.
- The [examples](examples.md): each one teaches one thing, from a hand-built Thing Description to a simulated microscope.

## Platform and licence

`teta-wot` supports **Windows x86_64** only, and is built and tested there. It has no licence yet, so it isn't published to crates.io: use it from its Git repository ([Installing](tutorial/installing.md)).
