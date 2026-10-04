# Discovery

A client that knows nothing about a server can find it, and its Things, as W3C WoT Discovery describes.

## The well-known URL and the directory

Every server is a read-only Thing Description Directory of its own Things:

| URL | What |
|---|---|
| `/.well-known/wot` | The directory's TD (`@type` `ThingDirectory`), as `application/td+json`, at the root whatever the API prefix |
| `{prefix}/directory/things` | Every served TD, sorted by `id`, as `application/ld+json`; `?offset=` and `?limit=` page it, and `?format=collection` wraps it |
| `{prefix}/directory/things/{id}` | One TD, by its `id` |

The directory's TD describes the other two. A generic consumer:

1. reads `/.well-known/wot`;
2. follows its `things` property's form;
3. reads the TDs it lists.

## DNS-SD

With the `mdns` feature, and `mdns: true` in the configuration (or `.mdns(true)`), the server advertises itself on the local network:

- as a `_wot._tcp` service with the `_directory` subtype, named after its server ID;
- with a TXT record saying where its TD is (`td=/.well-known/wot`), its type (`type=Directory`), and its scheme.

A browser for `_directory._sub._wot._tcp.local.` finds it: `dns-sd -B _wot._tcp` on Windows, or Python's `zeroconf`. The advertisement is withdrawn when the server stops. Multicast must be allowed on the network, and Windows may ask whether to let the server listen.

The [`discovery`](https://github.com/lucaswewa/teta-wot/tree/main/examples/discovery) example advertises two Things, finds them from Rust and from Python, and reads the directory.
