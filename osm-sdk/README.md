# logos-osm — the OSM registry SDK

The reusable crate behind the OSM distribution system. It covers two
different jobs, and most integrations only want one of them.

- **Consuming**: turn a region into the CID and metadata the registry
  records for it on-chain. This is the path a Basecamp module that wants map
  data takes. It needs no wallet, no storage node, and no registration.
- **Hosting**: download a region from Geofabrik, verify it, store it in
  Logos Storage, and build the on-chain registration. This is the
  distribution app's path.

## Consuming: resolve a region to its CID

```rust
use logos_osm::registry::resolve_region;

// The registry program's account, as returned by its deployment.
// Under the v0.3 program model this is the address the deployer chose,
// not the image id of the bytecode.
let program_account = "…".parse()?;

let entry = resolve_region(
    "https://testnet.lez.logos.co",
    &program_account,
    "germany",
).await?;

match entry {
    Some(e) => {
        let latest = e.latest_mirror().expect("a registered region has a mirror");
        println!("germany  cid={}  v{}", latest.cid, latest.version);
        println!("          from {}", latest.source_url);
        println!("          md5  {}", hex::encode(latest.checksum));
        println!("          {} registrar(s)", e.registrars().len());
    }
    None => println!("nobody has registered germany yet"),
}
```

`None` is an ordinary answer: it means the region is in the predefined set
but no one has mirrored it. The entry carries everything a reader needs to
fetch the bytes and check them independently — the CID to fetch from Logos
Storage, the checksum to verify against, and the canonical Geofabrik URL the
snapshot came from.

To enumerate instead of naming one region:

```rust
use logos_osm::registry::{all_regions, children_of};

for r in all_regions() {
    println!("{}  {}", r.path, r.name);
}
for child in children_of("us") {          // the closed set is a tree
    println!("{}", child.path);
}
```

From a Basecamp module the same call is available over the module API, with
no Rust dependency:

```qml
// C++ backend: modules().osm.invokeOpJson(op, argsJson)
// QML result: { region, parent, level, mirrors, registrars, latest: { cid, … } }
```

See `examples/consumer-app/` for a working module that does exactly this.

## Hosting: publish a region

```rust
use logos_osm::registry::build_register_region;
use logos_osm::storage::CodexStorage;
use logos_osm::OsmClient;

let client = OsmClient::new(CodexStorage::new("http://127.0.0.1:8080"), "osm-cache");

// Download, verify against Geofabrik's published MD5, store in Logos Storage.
let snapshot = client.host_region("germany").await?;
client.catalog_record_hosted(&snapshot)?;

// The registration a wallet submits. The SDK builds, the wallet signs.
let built = build_register_region(&program_account, &registrar, &snapshot.registration(None));
```

`host_regions_bulk` does the same for many regions with per-region opt-out,
and `build_register_regions_batch` builds the batch transaction.

## The integrity model

Map data is public, so nothing here encrypts. What it does is make the bytes
checkable. Every hop that moves data recomputes a streaming MD5 and compares
it against the checksum Geofabrik publishes for that snapshot, and the
registry stores that checksum next to the storage CID. A reader who fetches
a snapshot from Logos Storage verifies the same checksum a Geofabrik user
would, without a round trip to Geofabrik, and a host cannot substitute
different bytes without the check failing.

## Region set

The registry operates over a **frozen, non-overlapping** region set (the
prize's appendix A.4): 72 entries, 48 countries plus 24 subregions, where the
four largest countries are decomposed so no piece of geography is hosted or
counted twice. `regions::REGIONS` is the source of truth, and the guest
program embeds the same list and rejects anything outside it, so the chain
enforces the partition as well.

## Modules

- `regions` — the frozen set and the Geofabrik URL layout
- `geofabrik` — index and `.md5` fetch/parse, streaming PBF download
- `verify` — streaming MD5 over a downloaded file
- `storage` — Logos Storage (Codex) streaming put/get
- `registry` — the LEZ program client: PDA derivation, transaction building,
  on-chain reads, and entry decoding
- `osm` — the `OsmClient` lifecycle facade
- `retry` — exponential-backoff retry for transient transport failures
- `ffi` — the C ABI the Logos Core module dlopens

## Licence

MIT OR Apache-2.0.
