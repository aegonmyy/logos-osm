# OSM on Logos: decentralized OpenStreetMap distribution (LP-0018)

A pipeline that mirrors [OpenStreetMap extracts](https://download.geofabrik.de)
into **Logos Storage** and registers them on a **LEZ on-chain registry**, so
map data is served from the Logos network with verifiable integrity and
queryable provenance instead of from one host's HTTP server.

```
Geofabrik PBF -> MD5 verify -> stream to Logos Storage (CID)
                       |              |
                       |              v
                       |     LEZ registry program
                       |     (region -> mirrors, versions)
                       v              |
                published MD5         v
                recorded on-chain   consumer: fetch by CID,
                                   re-verify MD5, import, update-check
```

## What it does

| Lifecycle step | Where |
|---|---|
| Discover the closed region set against Geofabrik's live index | `osm-sdk` `geofabrik` + `osm` |
| Host: download, verify published MD5, store in Logos Storage | `osm-sdk` `osm::OsmClient::host_region` |
| Bulk hosting with per-region opt-out | `host_regions_bulk` |
| Register on LEZ (single + batch, permissionless, append-only mirrors) | `methods/osm` guest + `osm-sdk` `registry` |
| Query by region, parent, or CID | registry accounts + `cli` |
| Fetch from storage by CID, re-verify MD5, import locally | `fetch_snapshot` + `import_local` |
| Update check against Geofabrik's published version | `update_check` |

## Repository layout

```
osm-sdk/            the reusable SDK crate (logos-osm): regions, geofabrik,
                    verify, storage, registry, osm facade, ffi (C ABI)
cli/                `osm` command-line: the whole lifecycle from a terminal
methods/osm-core/   shared types + the frozen region table (host+guest)
methods/osm/        the LEZ RISC0 guest program (osm_registry)
methods/osm-host/   host-side embed of the committed guest ELF + program id
tools/guest-builder the only rebuild path for the guest artifact
module/             Logos Core module plugin (dlopens the SDK cdylib)
app/                Basecamp QML app (registrar + consumer tabs)
examples/           consumer-app: the minimal embedding example (SDK module
                    only, resolves a region to its CID, hosts nothing)
idl/                osm_registry.idl.json (SpelIdl schema)
tests/              integration tests: live sequencer lifecycle, off-chain
                    e2e (hermetic + real Codex), CU cycle profile
scripts/            build-ffi.sh, package-basecamp.sh, smoke_lgx.sh
docs/               cli-reference.md, CU_COSTS.md
```

## The region set

72 regions (48 countries + 24 subregions) forming a non-overlapping
partition of the world. Countries Geofabrik itself decomposes are represented
by their parts under a top-level parent: `us` has 8 states, `india` has 6
zones, `china` has 6 provinces, `russia` has 4 districts. The table is frozen
in `methods/osm-core/src/set.rs`, re-exported as `logos_osm::regions`, and
embedded in the guest program. The chain itself rejects any region outside
the set, and `parent`/`level` on-chain come from the table, never from the
client's claim.

```console
$ osm regions --parent us
us/california  north-america  https://download.geofabrik.de/north-america/us/california-latest.osm.pbf
us/texas       north-america  https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf
...
```

## Integrity model

Map data is public, nothing is encrypted. Integrity and provenance are the
product.

- Geofabrik's published MD5 is downloaded and the PBF is verified against it
  before storing. The checksum is recorded on-chain next to the storage CID.
  A consumer that fetches the snapshot re-verifies the same MD5. A mismatch
  is a hard error, never a silent pass (tamper test included).
- Versions are Geofabrik's snapshot dates, recovered from the
  `X-Derived-From` response header or the dated filename inside the `.md5`
  body, normalized to `YYYYMMDD`.
- The registry is append-only. Each registration appends a mirror (registrar,
  CID, checksum, version, timestamp) to the region's on-chain entry. Nothing
  is overwritten. History is capped at 32 mirrors (oldest pruned).
- Permissionless hosting. Any signer may mirror any region in the set. The
  adoption criteria (distinct registrars, advancing versions) are read
  straight off the chain.

## The LEZ program

One RISC0 guest (`methods/osm/src/bin/osm_registry.rs`), pure `execute()`
dispatch (unit-tested on the host, wired to the RISC0 bridge in `main()`):

- `Init { owner }` - one-time registry initialization.
- `RegisterRegion { registration }` - append one mirror to one region PDA.
- `RegisterRegionsBatch { registrations }` - up to 24 regions per tx.

Accounts are PDAs derived from `(program_id, seed)`: one registry PDA
(`/OSM/REGISTRY/V1/SEED/0000000000`) plus one PDA per region
(`SHA-256("osm-region:" || path)`). The committed artifact is
`methods/osm-host/osm_registry.bin` with the program id pinned in its
`build.rs`:

```
c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79
```

Rebuild path: `tools/guest-builder` (needs the risc0 toolchain; CI embeds
the committed artifact instead).

### Deployed on the public LEZ testnet (v0.3)

The registry is live on Logos testnet v0.3 with **25 verified region entries
across 25 countries**. Each entry has its Geofabrik MD5 and Logos Storage CID
recorded on-chain, and each hosted blob can be fetched and hash-verified from
a publicly reachable storage node.

| Item | Value |
|---|---|
| Sequencer | `https://testnet.lez.logos.co` |
| Program account | [`D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt`](https://explorer.testnet.lez.logos.co/account/D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt) |
| Program account (hex) | `b2fa65027d9b2540d4ac2aa4e0a38040ac03604c2201930592c547f3d65ecb43` |
| Guest image ID | `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79` |
| Registrar (signer) | `4287ea6f05d6736f5ae9167a6f3dd46ef858aee54aba295cd547117e53e204b4` |
| Public storage node | `http://185.227.110.22` (read-only, writes blocked) |
| Hosted blobs | 29 PBF snapshots (42 total including test artifacts) |

#### Registered regions (25 entries, 25 countries)

24 country-level entries: Switzerland, Denmark, Portugal, Greece, Hungary,
Romania, Bulgaria, Belarus, Mexico, South Korea, Thailand, Vietnam,
Philippines, Pakistan, Bangladesh, Iran, Argentina, Colombia, Peru, Chile,
South Africa, Egypt, Kenya, Morocco, Ethiopia.

1 subregion: `china/henan` (3 mirrors, versions 20261106, 20261108 and
20261109. The latest is the current snapshot on the storage node.)

Read any entry directly from the chain:

```sh
osm --program-account D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt \
   --sequencer https://testnet.lez.logos.co \
   lookup --region china/henan
```

Fetch a hosted blob by its on-chain CID and verify the MD5:

```sh
curl -o henan.osm.pbf \
  http://185.227.110.22/api/storage/v1/data/zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3/network/stream
md5sum henan.osm.pbf   # should match the on-chain checksum
```

Testnet 0.4 deployment is pending the network launch (around 2026-11-30).

## Setup

This section walks through everything you need to host and register OSM
regions: the prerequisites, installing the CLI, running a storage node, and
your first host and register cycle. For the Basecamp app path, see
[Basecamp app + module](#basecamp-app--module) below.

### Prerequisites

| Requirement | What it is | Where to get it |
|---|---|---|
| **Rust** (1.98+) | Compiles the CLI and SDK | [rustup.rs](https://rustup.rs) |
| **Logos Storage (Codex)** | Stores PBF files, serves them by CID | [Storage docs](https://logos-storage-docs.netlify.app/tutorials/storage-module/) or the Docker command below |
| **LEZ wallet** | Signs and submits registration transactions on the LEZ chain | [Run an app docs](https://docs.logos.co/run-an-app) (wallet setup) or mine/fund on testnet |
| **Testnet LEZ** | The chain the registry is deployed on | `https://testnet.lez.logos.co` (public, read-only queries need no wallet) |

You do **not** need a wallet or testnet tokens to browse the registry or
fetch regions. You only need a funded wallet to **register** (host and write
to the chain).

### 1. Install the CLI

```sh
git clone https://github.com/aegonmyy/logos-osm.git
cd logos-osm
cargo build -p logos-osm-cli

# The binary is at target/release/osm (or target/debug/osm)
# Optionally symlink it:
sudo ln -sf "$(pwd)/target/release/osm" /usr/local/bin/osm
```

Full command reference: [`docs/cli-reference.md`](docs/cli-reference.md).

### 2. Run a Logos Storage node (Codex)

You need a storage node to host PBF files. The easiest way is Docker:

```sh
docker run -d --name osm-codex -p 8080:8080 -e NAT_IP_AUTO=false \
  --entrypoint /usr/local/bin/storage \
  codexstorage/nim-codex@sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb \
  --api-bindaddr=0.0.0.0 --api-port=8080
```

This starts a Codex storage node on port 8080. Verify it is running:

```sh
curl http://127.0.0.1:8080/api/storage/v1/data?list=true
# Should return {"content":[]} on a fresh node
```

For the full storage setup guide, see the
[Logos Storage module tutorial](https://logos-storage-docs.netlify.app/tutorials/storage-module/).

### 3. Set up your LEZ wallet

To register regions on-chain, you need a LEZ wallet with testnet tokens.
Follow the [Run an app guide](https://docs.logos.co/run-an-app) to create a
wallet and fund it. The CLI looks for the wallet at
`~/.logos-osm/wallet/` by default (containing `config.json` and a `storage/`
directory).

If you just want to browse the registry or fetch regions, skip this step.
Those operations are read-only and need no wallet.

### 4. Host and register your first region

Set your environment variables:

```sh
export OSM_STORAGE_URL=http://127.0.0.1:8080
export OSM_PROGRAM_ACCOUNT=D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt
export OSM_SEQUENCER=https://testnet.lez.logos.co
```

Browse the region set (no wallet needed):

```sh
osm regions                    # the 72-region closed set
osm regions --parent us        # US subregions
osm lookup --region germany    # read germany's on-chain entry
```

Host a region (downloads from Geofabrik, verifies MD5, stores in your Codex
node, prints the registration transaction):

```sh
osm host germany --registrar <your-account-hex32>
```

Register it on-chain (signs with your wallet and submits to the sequencer):

```sh
osm register germany --registrar <your-account-hex32> --sign
```

That is the full cycle. The region is now hosted in your storage node and
registered on the LEZ chain with its CID and checksum pinned. Anyone on the
network can look it up and fetch it.

To bulk host and register multiple regions at once:

```sh
osm host-bulk --regions germany,france,spain --registrar <your-account-hex32>
osm register-bulk --regions germany,france,spain --registrar <your-account-hex32> --sign
```

To fetch a region someone else has hosted (using the CID and checksum from
the on-chain registry):

```sh
osm lookup --region china/henan    # get the CID and checksum
osm fetch china/henan --cid <cid> --checksum <md5hex>
```

### 5. Or use the Basecamp app instead

If you prefer a GUI over the CLI, install the modules in
[Logos Basecamp](https://github.com/logos-co/logos-basecamp):

1. Add the catalog (Settings, Package Repositories):
   ```
   https://raw.githubusercontent.com/aegonmyy/logos-modules-release-base/main/logos-repo.json
   ```
2. Install `osm` (the SDK) and `osm_app` (the distribution app).
   If you only need to resolve regions to their CIDs without the full
   distribution app, install `osm` and `osm_consumer` instead.
3. Point the app at your Codex node and the testnet:
   ```
   Storage URL:  your local Codex node (e.g. http://127.0.0.1:8080)
   Sequencer:    https://testnet.lez.logos.co
   Program:      D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt
   ```

The app has three tabs: Host/Registrar (host and register regions), Regions
(browse the set and see on-chain status), and Consumer (fetch, import,
update check).

Demo video: https://youtu.be/mW0M1lPDjpo

## Basecamp app + module (technical)

`module/` is a Logos Core module plugin (universal interface) that dlopens
`liblogos_osm.so` and forwards ops over the FFI C ABI
(`logos_osm_invoke`). `app/` is the QML app with three tabs: **Host /
Registrar** (open, discover, regions, host, host-bulk, register,
register-bulk, init), **Regions** (the closed set with each region's
on-chain status via `lookup`), and **Consumer** (fetch, import, update,
catalog). It talks to the module over LogosAPIClient.

`examples/consumer-app/` is the minimal embedding example: a module that
depends on `osm` and does nothing but enumerate the set and resolve a region
to its CID. It is the shape most apps that want map data will take. It is
published to the catalog as `osm_consumer` v1.0.0, so you can install it in
Basecamp alongside `osm` without building from source.

### Build locally

```sh
nix build .#osm-lgx .#osm-app-lgx   # .lgx bundles (root flake)
nix build ./app#lgx-portable        # the app alone (app/flake.nix)
./scripts/build-ffi.sh              # liblogos_osm.so for the module
./scripts/smoke_lgx.sh              # full-chain smoke (plugin + Rust core)
```

## Tests and CI

```sh
cargo test --workspace --lib --bins --exclude osm-guest-builder
cargo test -p osm-integration-tests --test offchain_live
cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored
```

CI (`.github/workflows/ci.yml`) runs five jobs: unit tests and hermetic
off-chain lifecycle, live e2e with a standalone Docker sequencer and real
Codex node, the CU cycle profile, a macOS run that checks the cdylib loads
and dispatches, and the .lgx bundle build with a callable smoke test. See
`docs/CU_COSTS.md` for measured per-instruction costs.

## License

MIT OR Apache-2.0.
