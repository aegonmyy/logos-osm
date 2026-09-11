# OSM on Logos — decentralized OpenStreetMap distribution (LP-0018 / PR #71)

A complete pipeline that mirrors [OpenStreetMap extracts](https://download.geofabrik.de)
into **Logos Storage** and registers them on a **LEZ on-chain registry**, so
map data is served from the Logos network with verifiable integrity and
queryable provenance — not from one host's HTTP server.

```
Geofabrik PBF ──download──► MD5 verify ──stream──► Logos Storage (CID)
                                 │                       │
                                 │                       ▼
                                 │            LEZ registry program
                                 │            (region → mirrors, versions)
                                 ▼                       │
                          published MD5                  ▼
                          recorded on-chain     consumer: fetch by CID,
                                               re-verify MD5, import, update-check
```

## What it does

| Lifecycle step | Where |
|---|---|
| Discover the closed region set against Geofabrik's live index | `osm-sdk` `geofabrik` + `osm` |
| Host: download → verify published MD5 → store in Logos Storage | `osm-sdk` `osm::OsmClient::host_region` |
| Bulk hosting with per-region opt-out | `host_regions_bulk` |
| Register on LEZ (single + batch, permissionless, append-only mirrors) | `methods/osm` guest + `osm-sdk` `registry` |
| Query by region / parent / CID | registry accounts + `cli` |
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
tests/              integration tests: live sequencer lifecycle, off-chain
                    e2e (hermetic + real Codex), CU cycle profile
scripts/            build-ffi.sh, package-basecamp.sh, smoke_lgx.sh, demo.sh
docs/               DESIGN.md, CU_COSTS.md, PERFORMANCE.md, demo evidence
submission/LP-0018.md
```

## The region set

**72 regions** (48 countries + 24 subregions) forming a non-overlapping
partition of the world (prize appendix A.4). Countries Geofabrik itself
decomposes are represented by their parts under a top-level parent
(`us` → 8 states, `india` → 6 zones, `china` → 6 provinces, `russia` → 4
districts). The table is frozen in `methods/osm-core/src/set.rs`, re-exported
as `logos_osm::regions`, and **embedded in the guest program** — the chain
itself rejects any region outside the set, and `parent`/`level` on-chain come
from the table, never from the client's claim.

```console
$ osm regions --parent us
us/alaska      north-america   https://download.geofabrik.de/north-america/us/alaska-latest.osm.pbf
us/california  north-america   https://download.geofabrik.de/north-america/us/california-latest.osm.pbf
...
```

## Integrity model

Map data is public — nothing is encrypted. Integrity and provenance are the
product:

- **Geofabrik's published MD5** is downloaded and the PBF is verified against
  it *before* storing; the checksum is recorded on-chain next to the storage
  CID. A consumer that fetches the snapshot re-verifies the same MD5 — a
  mismatch is a hard error, never a silent pass (tamper test included).
- **Versions** are Geofabrik's snapshot dates, recovered from the
  `X-Derived-From` response header (`<path>-<yymmdd>.osm.pbf.md5`) or the
  dated filename inside the `.md5` body, normalized to `YYYYMMDD`.
- **The registry is append-only**: each registration appends a mirror
  (registrar, CID, checksum, version, timestamp) to the region's on-chain
  entry; nothing is overwritten. History is capped at 32 mirrors (oldest
  pruned).
- **Permissionless hosting**: any signer may mirror any region in the set —
  the adoption criteria (≥3 distinct registrars, advancing versions) are read
  straight off the chain.

## The LEZ program

One RISC0 guest (`methods/osm/src/bin/osm_registry.rs`), pure `execute()`
dispatch (unit-tested on the host, wired to the RISC0 bridge in `main()`):

- `Init { owner }` — one-time registry initialization.
- `RegisterRegion { registration }` — append one mirror to one region PDA.
- `RegisterRegionsBatch { registrations }` — up to 24 regions per tx.

Accounts are PDAs derived from `(program_id, seed)`: one registry PDA
(`/OSM/REGISTRY/V1/SEED/0000000000`) + one PDA per region
(`SHA-256("osm-region:" || path)`). The committed artifact is
`methods/osm-host/osm_registry.bin` with the program id pinned in its
`build.rs`:

```
77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0
```

(Rebuild path: `tools/guest-builder` — needs the risc0 toolchain; CI embeds
the committed artifact instead.)

## Quickstart (CLI)

```sh
cargo build -p logos-osm-cli

# against a local Codex node (see below); --memory-storage works offline
export OSM_STORAGE_URL=http://127.0.0.1:8080

osm regions                          # the closed set
osm discover                         # cross-check against Geofabrik's index
osm host germany --registrar <hex32> # download, verify MD5, store, print tx
osm register-bulk --regions germany,france --registrar <hex32>
osm fetch germany --cid <cid> --checksum <md5hex>
osm update germany
```

Registration transactions are **printed as JSON** (accounts, identity kinds,
risc0-serde instruction words) for the wallet to submit — the SDK builds, the
wallet signs; the SDK never holds keys. The full on-chain submission is
exercised end-to-end by `tests/tests/osm_registry_live.rs` against a
standalone sequencer.

### Logos Storage node (Codex)

The upstream image's default entrypoint is broken (`CMD ["codex"]`, no such
binary); run it digest-pinned with the entrypoint override:

```sh
docker run -d --name osm-codex -p 8080:8080 -e NAT_IP_AUTO=false \
  --entrypoint /usr/local/bin/storage \
  codexstorage/nim-codex@sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb \
  --api-bindaddr=0.0.0.0 --api-port=8080
```

## Basecamp app + module

`module/` is a Logos Core module plugin (universal interface) that dlopens
`liblogos_osm.so` (`scripts/build-ffi.sh`) and forwards ops over the FFI C ABI
(`logos_osm_invoke`). `app/` is the QML app — a **Registrar** tab
(open/discover/regions/host/host-bulk/register/register-bulk/init) and a
**Consumer** tab (fetch/import/update/catalog) — talking to the module over
LogosAPIClient.

```sh
nix build .#osm-lgx .#osm-app-lgx   # .lgx bundles
./scripts/build-ffi.sh              # liblogos_osm.so for the module
./scripts/smoke_lgx.sh              # full-chain smoke (plugin + Rust core)
```

## Tests & CI

```sh
cargo test --workspace --lib --bins --exclude osm-guest-builder      # unit (incl. guest logic tests)
cargo test -p osm-integration-tests --test offchain_live      # hermetic e2e
cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored  # CU costs
```

CI (`.github/workflows/ci.yml`) runs four jobs: unit + hermetic off-chain
lifecycle; **live e2e** (standalone Docker sequencer, real registration
lifecycle + off-chain flow against a real Codex node); the CU cycle profile;
and the .lgx bundle build + callable smoke. See `docs/DESIGN.md` for the
architecture and `docs/CU_COSTS.md` for measured per-instruction costs.

## License

MIT OR Apache-2.0.
