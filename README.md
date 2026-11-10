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

## Quickstart (CLI)

Full command reference with examples: [`docs/cli-reference.md`](docs/cli-reference.md).

```sh
cargo build -p logos-osm-cli

export OSM_STORAGE_URL=http://127.0.0.1:8080
export OSM_PROGRAM_ACCOUNT=D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt

osm regions                          # the closed set
osm discover                         # cross-check against Geofabrik's index
osm host germany --registrar <hex32> # download, verify MD5, store, print tx
osm register-bulk --regions germany,france --registrar <hex32>
osm fetch germany --cid <cid> --checksum <md5hex>
osm update germany

# read the registry straight from the chain (no wallet needed)
osm lookup --region germany
osm lookup --parent us
osm lookup --cid <cid>
```

Commands that build a registration transaction need
`--program-account <base58>` (or `OSM_PROGRAM_ACCOUNT`). Under the v0.3
program model a deployed program lives at an account the deployer chose, so
PDAs and transaction targets key off that account rather than off the
bytecode's image id.

Registration transactions are printed as JSON (the program account, the
shard selectors, the signing account, and borsh-encoded instruction bytes)
for the wallet to submit. The SDK builds, the wallet signs, and the SDK never
holds keys. Pass `--sign` to have the CLI open the wallet, sign, and submit
to the sequencer directly. The full on-chain submission is exercised
end-to-end by `tests/tests/osm_registry_live.rs` against a standalone
sequencer.

### Logos Storage node (Codex)

The upstream image's default entrypoint is broken (`CMD ["codex"]`, no such
binary). Run it digest-pinned with the entrypoint override:

```sh
docker run -d --name osm-codex -p 8080:8080 -e NAT_IP_AUTO=false \
  --entrypoint /usr/local/bin/storage \
  codexstorage/nim-codex@sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb \
  --api-bindaddr=0.0.0.0 --api-port=8080
```

## Basecamp app + module

`module/` is a Logos Core module plugin (universal interface) that dlopens
`liblogos_osm.so` and forwards ops over the FFI C ABI
(`logos_osm_invoke`). `app/` is the QML app with three tabs: **Host /
Registrar** (open, discover, regions, host, host-bulk, register,
register-bulk, init), **Regions** (the closed set with each region's
on-chain status via `lookup`), and **Consumer** (fetch, import, update,
catalog). It talks to the module over LogosAPIClient.

`examples/consumer-app/` is the minimal embedding example: a module that
depends on `osm` and does nothing but enumerate the set and resolve a region
to its CID.

### Install from the catalog

The modules are published, so Basecamp can install them without a local
build. Add this catalog to the module repo list:

```
https://raw.githubusercontent.com/aegonmyy/logos-modules-release-base/main/logos-repo.json
```

It serves `osm` (the SDK core module) and `osm_app` (the distribution app)
for darwin-arm64, linux-amd64, linux-arm64 and windows-x86_64.

Point the app at your Codex storage node and the live testnet registry:

```
Storage URL:  your local Codex node
Sequencer:    https://testnet.lez.logos.co
Program:      D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt
```

Demo video: https://youtu.be/mW0M1lPDjpo

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
