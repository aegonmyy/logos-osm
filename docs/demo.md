# OSM on Logos: a written walkthrough

OpenStreetMap is the free, community-built map of the world. It powers
routing in apps like Organic Maps and OsmAnd, search in Nominatim, and
tile rendering on openstreetmap.org. The data itself is open, but in
practice almost everyone downloads it from one host: Geofabrik. If
Geofabrik goes down or changes a file, every downstream app breaks.

This project puts that same map data on the Logos network. Each region's
checksum is pinned on-chain, so anyone can host a copy, verify it hasn't
been tampered with, and fetch it without trusting a single server.

## What you can build with this

The SDK resolves any region to its on-chain content ID. Any app that
needs map data can depend on it. Here are concrete examples, rooted in
things people already use today:

**Offline maps and navigation.** Organic Maps, OsmAnd, and Maps.me all
download OSM extracts to work offline. Today they fetch from Geofabrik
or their own mirrors. With the SDK, the same app can fetch a verified
region by its on-chain CID, and the user knows the bytes match what was
registered. No silent swap, no broken mirror.

**Local search and geocoding.** Nominatim is the search engine behind
openstreetmap.org. It reads OSM PBF files and indexes place names,
addresses, and POIs. A Logos-native geocoder would resolve a region to
its CID, fetch the PBF, build the index, and answer queries. The index
itself could be stored and served from Logos Storage too.

**Routing.** OSRM, GraphHopper, and Valhalla all consume OSM PBF data to
build routing graphs. A Logos-native routing app would fetch a verified
region, build the graph, and serve turn-by-turn directions. The graph
can be cached and re-verified against the on-chain checksum on each
update.

**A "what changed" viewer.** The registry records multiple versions of
each region (Geofabrik republishes snapshots). An app could fetch two
versions of the same region and show a diff: new roads, deleted POIs,
renamed neighborhoods. This is unique to the on-chain model because the
version history is append-only and publicly verifiable.

**Map rendering in Basecamp.** A QML canvas that resolves a region,
fetches the PBF, parses it, and draws the roads, rivers, and parks as
vector geometry. The SDK module is already published. A renderer module
on top of it would be a standalone Basecamp app that works with any
region in the set.

**Decentralized tile server.** Render map tiles from OSM data and serve
them from Logos Storage nodes. Multiple hosts can serve the same tiles
with the same content ID. A map website or app fetches tiles by CID,
verifying each one. No single tile server to DDoS or trust.

## CLI walkthrough

Everything below runs from the command line. No wallet needed to browse
or fetch. You only need a wallet to host and register. For the full list
of commands, flags, and options, see the
[CLI reference](cli-reference.md).

### Browse the region set

72 regions, fixed and non-overlapping:

```sh
osm regions
```

```
germany         Europe       https://download.geofabrik.de/europe/germany-latest.osm.pbf
france          Europe       https://download.geofabrik.de/europe/france-latest.osm.pbf
great-britain   Europe       https://download.geofabrik.de/europe/great-britain-latest.osm.pbf
italy           Europe       https://download.geofabrik.de/europe/italy-latest.osm.pbf
...
```

Countries that Geofabrik itself decomposes are listed under a parent:

```sh
osm regions --parent us
```

```
us/california    North America   https://download.geofabrik.de/north-america/us/california-latest.osm.pbf
us/texas         North America   https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf
us/florida       North America   https://download.geofabrik.de/north-america/us/florida-latest.osm.pbf
...
```

### Look up what is already on-chain

25 regions are registered on the Logos testnet right now. You can read
the registry directly:

```sh
osm lookup --region china/henan
```

```
china/henan   china   level=1   mirrors=3   registrars=1
  4287ea6f... v20261106  cid=zDvZRwzm5xvyV6sq6Sh6CzeDoYpQeofDwj5MDvcQREry6PrzGmhR  md5=4695a4256b1664bb2dab96a63f2f4377
  4287ea6f... v20261108  cid=zDvZRwzkweTn7BCCYxfsCTnpekiHTW9Nms5p8W1zqYM6Bodjcdzc  md5=0c7d0316022e4c64bb4d6c5fef3768fd
  4287ea6f... v20261109  cid=zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3  md5=0acc723baca10b2860d82e706f7fcd7d
```

Three snapshots, three different content IDs, three different checksums.
Each one is a complete copy of Henan province's map data at a different
point in time. The registry is append-only, so all three are visible.

### Host a region

Hosting downloads the PBF from Geofabrik, verifies the MD5 against
Geofabrik's published checksum, stores it in Logos Storage, and prints
the registration transaction. You need a Codex storage node running
locally (see the Setup section in the README). The `host` command has
flags for bulk hosting and opt-out, see the
[CLI reference](cli-reference.md) for details.

```sh
osm host germany --registrar <your-account-hex>
```

The output shows the download, the MD5 verification, the storage upload,
and the CID that Logos Storage assigned:

```
downloading germany from https://download.geofabrik.de/europe/germany-latest.osm.pbf
verified md5: 9a3f7b2c...
stored in Logos Storage: zDvZRw...
registration tx: {"region":"germany","cid":"zDvZRw...","md5":"9a3f7b2c...","version":"20261115"}
```

### Register it on-chain

The host step prints the transaction. Register signs it with your wallet
and submits it to the LEZ sequencer. The `register` and `register-bulk`
commands accept `--wallet-dir`, `--wallet-password`, and `--sign` flags,
see the [CLI reference](cli-reference.md) for the full set.

```sh
osm register germany --registrar <your-account-hex> --sign
```

Now the region is on-chain. Anyone on the network can look it up, see
your CID, and fetch the data from your storage node. The checksum is
pinned next to the CID, so a consumer can verify the bytes match.

### Fetch a region someone else hosted

You don't need to host a region to use its data. Look up the CID and
checksum on-chain, then fetch from Logos Storage. The `fetch` command
takes `--cid` and `--checksum` (both required, both come from the
on-chain lookup), see the [CLI reference](cli-reference.md) for the
full fetch and import workflow.

```sh
osm lookup --region china/henan
```

Take the latest CID and MD5 from the output, then:

```sh
osm fetch china/henan --cid zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3 --checksum 0acc723baca10b2860d82e706f7fcd7d
```

```
fetched china/henan (51592926 bytes, 1053 blobs, md5 verified)
```

The SDK fetched the data from Logos Storage by its content ID,
downloaded it across multiple blobs, and verified the MD5 against the
on-chain checksum. If a storage node served the wrong bytes, the fetch
would fail with a checksum mismatch. The data is now cached locally and
ready to use.

### The decentralization part

When someone else runs their own storage node and hosts the same region,
they register a new mirror entry with the same region path but a
different hosting account. The registry tracks all mirrors:

```
china/henan   mirrors=3   registrars=1
```

Right now all three mirrors are from one registrar. When a second
person hosts Henan under their own account, `registrars` goes to 2.
Every new mirror makes the data genuinely more decentralized. The CID
stays the same, so consumers don't need to change anything. They just
get more sources to fetch from.

This is the difference between "we moved the data to a new central
host" and "the data is actually distributed across independent nodes
that anyone can verify."

## Basecamp walkthrough

The same cycle works from the Basecamp app, no terminal needed. Screenshots
will go here showing each step.

<!-- TODO: screenshot — installing the osm and osm_app modules from the catalog -->

<!-- TODO: screenshot — the Regions tab showing the 72-region set with on-chain status -->

<!-- TODO: screenshot — the Host/Registrar tab hosting a region (download, verify, store) -->

<!-- TODO: screenshot — the registration transaction and on-chain confirmation -->

<!-- TODO: screenshot — the Consumer tab fetching a region by CID and verifying the MD5 -->

<!-- TODO: screenshot — the on-chain lookup showing mirrors and registrars -->
