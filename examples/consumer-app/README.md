# Consumer example: resolve a region to its CID

A minimal Basecamp module that depends on the OSM SDK and nothing else.

It is the shape most apps that want map data will take. A viewer, a router, a
field-mapping tool: each needs to turn a region into the content address of
the map file the registry vouches for it. None of them needs to host
anything, register anything, or run storage of its own.

## What it does

Two calls, both into the `osm` core module over RemoteObjects:

| Call | Purpose |
|---|---|
| `invokeOpJson("regions", "{}")` | enumerate the predefined region set |
| `invokeOpJson("lookup", "{\"region\":\"germany\"}")` | resolve a region to the CID, source URL, checksum and version recorded for it on-chain |

Everything else is the SDK's business. There is no storage node here, no
registration transaction, and no wallet.

## The dependency

Declared in `metadata.json`:

```json
"dependencies": ["osm"]
```

The `osm` module is what carries the SDK. This example never links the Rust
crate directly; it goes through the module API, which is the same interface
any other Basecamp module sees.

## Building

The module builds with `logos-module-builder`, the same as the distribution
app:

```sh
nix build .#osm-consumer-lgx            # from the repository root
```

Load the resulting bundle in Basecamp with the `osm` module present. The view
lists the region set and resolves whichever region you pick.

## Why the API stops where it does

The SDK exposes the full lifecycle — host, register, fetch, import — and this
example uses two of those calls. That is deliberate. A consumer that only
reads does not need the write half, and a module that pulls in hosting also
pulls in a storage node and a funded account. Keeping the read path separate
is what lets a small app embed the SDK without taking on an operator's
burden.
