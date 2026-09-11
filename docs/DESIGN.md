# Design notes — OSM on Logos (LP-0018 / PR #71)

These are the decisions that aren't obvious from the code, and why they're
shaped the way they are. Reader assumed: a reviewer or the next engineer
picking this up cold.

## 1. The closed region set is code, not config

`methods/osm-core/src/set.rs` freezes **72 regions** (48 countries + 24
subregions). The same table is:

- re-exported to SDK users as `logos_osm::regions`,
- **embedded in the RISC0 guest**, which rejects any path outside it, and
- the authority for `parent`/`level` on-chain — the client's claim is never
  trusted.

Why embed rather than validate client-side: the prize's core invariant is a
*non-overlapping partition* (appendix A.4). If enforcement lived in the
client, a hostile registrar could register overlapping regions and the chain
would faithfully record garbage. Embedding moves the invariant into the
verified program: the guest panics on `narnia` as easily as on `us` (the
decomposed countries are deliberately *not* in the set — `us`, `india`,
`china`, `russia` exist only as parents; their Geofabrik files would overlap
the subregions that *are* hosted).

Cost: adding a region means recompiling the guest. That's the point — the
partition should change as slowly as the program that enforces it.

### Decomposed countries

Geofabrik decomposes four large countries into sub-files. We model them as
parents with children in the set (`us/alaska` … `us/wisconsin`,
`india/south` …, `china/hubei` …, `russia/*` under a top-level `russia/`
parent). A test guards the invariant: none of `osm_core::set::DECOMPOSED`
resolves via `by_path`.

## 2. Version recovery from Geofabrik

Geofabrik does not publish a machine-readable version column. The `.md5`
endpoint is the only per-region metadata, and its body names the *undated*
`-latest` file for most regions. The snapshot date is recovered from either:

1. the **`X-Derived-From` response header** — `/{path}/{region}-{yymmdd}.osm.pbf.md5`
   names the dated upstream snapshot the `-latest` file was derived from; or
2. a **dated filename inside the `.md5` body** (`{hash}  {region}-{yymmdd}.osm.pbf`).

Both normalize to `YYYYMMDD` (`20_000_000 + yy*10_000 + mmdd`), which is the
`version` recorded on-chain. The guest asserts the value is a plausible date
(`19700101..=99991231`).

The header path is primary because it's present even when the body names
`-latest`. Both paths are exercised by the hermetic fixture server in
`tests/tests/offchain_live.rs`, which speaks the exact layout
(`index-v1-nogeom.json`, `<region>-latest.osm.pbf[.md5]`, `X-Derived-From`).

## 3. Integrity: MD5, deliberately

MD5 is cryptographically broken *as a collision-resistance primitive*. It is
the right choice here anyway, and the reason matters:

- **It's Geofabrik's published checksum.** Verifying against it proves we
  have *the bytes Geofabrik published* — substituting SHA-256 would be a
  self-attested hash of unknown provenance, strictly weaker for this purpose.
- **The adversary is corruption/mis-mirror, not Geofabrik forging collisions.**
  The threat is a storage node or mirror serving wrong bytes; any mismatch
  against the on-chain-recorded MD5 catches that.
- The checksum is recorded **on-chain next to the CID**, so a consumer
  fetching by CID re-verifies against a value the registrar committed to —
  the storage node can't serve different bytes without detection.

The registry also stores the storage **CID** (content-addressed), so the
snapshot is independently addressable; the MD5 is the Geofabrik-provenance
proof, the CID is the network pointer.

## 4. Registry program model

One guest program, two account families, all PDAs (LEZ has no account
attributes / program-derived niceties beyond this — see the LEZ facts):
- registry PDA — seed `/OSM/REGISTRY/V1/SEED/0000000000`, holds `RegistryState`
  (owner, region_count, registration_count);
- one region PDA per region — seed `SHA-256("osm-region:" || path)`, holds
  `RegionEntry` with the full append-only mirror history.

`RegisterRegion` appends `Mirror { registrar, cid, checksum, version,
timestamp, hosted }`. Mirror history is timestamp-ordered and capped at
`MAX_MIRRORS = 32` (oldest pruned) so a hot region's account can't grow
unboundedly. Region vs registration counts: registering an existing region
increments only `registration_count` — the adoption metrics (distinct
registrars per region, advancing versions) are computed from the region
entry's mirror list, not the global counters.

`RegisterRegionsBatch` (cap `MAX_BATCH = 24`) amortizes tx overhead for bulk
hosting: one registry-account write + one registrar nonce for N regions. The
cap keeps the instruction within tx-size and per-tx compute bounds; the CU
profile (`docs/CU_COSTS.md`) measures the amortization curve and shows where
it flattens.

**Permissionless by design.** Anyone can mirror any region in the set. No
owner approval step exists because the prize's adoption criteria are about
*multiple independent registrars* — gating on the owner would defeat the
metric. There is also no removal instruction: registrations are append-only;
an incorrect mirror is superseded (consumers take `latest_mirror()` by
timestamp), not erased. That's the honest version of "the chain is an
append-only ledger".

## 5. Off-chain client architecture

`osm-sdk` is transport-agnostic where it can be:

- `Storage` trait with two impls: `CodexStorage` (the real Logos Storage REST
  node, streaming put/get) and `MemoryStorage` (tests, offline demos). The
  CLI picks with `--memory-storage`; the FFI picks from its persisted state.
- `Geofabrik` client with a `mirror()` host-swap: pointing the base at a
  different host rewrites canonical URLs (`download.geofabrik.de` → the
  override) while leaving paths intact. This is what lets the *same* client
  code run against the hermetic fixture server in CI and the real Geofabrik
  in production — no URL plumbing through the call graph, no test-only code
  paths in the SDK.
- The wallet boundary: the SDK **builds** transactions (accounts +
  risc0-serde instruction words, serialized as JSON) and the wallet
  **submits**. The SDK holds no keys and never signs; the FFI exports exactly
  this shape. The full submit path is proven by
  `tests/tests/osm_registry_live.rs`, which drives the real standalone
  sequencer end-to-end (deploy → Init → germany by 3 registrars → batch →
  counters).

## 6. Committed guest artifact

Guest ELFs are not byte-reproducible across toolchain versions. The repo
therefore commits the built artifact (`methods/osm-host/osm_registry.bin`,
471,820 bytes) and pins the program id in `build.rs`:

```
77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0
```

CI embeds the committed artifact — no risc0 toolchain needed for the unit
job. `tools/guest-builder` (excluded from default workspace builds) is the
only rebuild path: run it, then re-pin the id if it differs. This keeps
"what's deployed" and "what's tested" the same bytes, which is exactly what
an evaluator checking `RISC0_DEV_MODE=0` proofs cares about.

## 7. FFI and the Qt module

`osm-sdk/src/ffi.rs` is a minimal C ABI (`logos_osm_version`,
`logos_osm_invoke(name, args_json)`, `logos_osm_free_string`) over a private
tokio runtime. All ops return `{"ok":true,"result":…}` / `{"ok":false,"error":…}`.
The module plugin (`module/`) dlopens the cdylib and forwards ops; the app
(`app/`) talks to the module over LogosAPIClient. Two-panel design mirrors
the two roles: **Registrar** hosts and registers; **Consumer** fetches,
verifies, imports, and checks updates — the same SDK facade both ways.

The SDK is `#![deny(unsafe_code)]` (not `forbid`) solely so `ffi.rs` — the
one boundary that must handle raw pointers — can scope `allow(unsafe)` with
a comment. Everything else stays checked.

## 8. Test strategy

| Layer | Test | What it proves |
|---|---|---|
| Guest logic | `methods/osm` `#[cfg(test)]` | closed-set rejection, PDA checks, signing checks, append-only, mirror cap, batch cap — as pure host tests over the same `execute()` |
| SDK units | `osm-sdk` lib tests (40) | region table, URL join, MD5 parse + version recovery (both signals), storage round-trips, catalog merge, mirror host-swap |
| Off-chain e2e | `offchain_live::hermetic_lifecycle_on_fixture` | discover(72) → host → verify → store → tx shape → fetch → import → update → tamper-reject, zero network beyond loopback — runs in CI on every push |
| Off-chain e2e (real) | `offchain_live::offchain_lifecycle_on_real_codex` | the same flow against a real Codex node (streaming put/get + CID readback over the wire) |
| On-chain e2e | `osm_registry_live` | real sequencer: deploy → Init → registrations by 3 distinct registrars (version advancement) → batch of 3 → counters |
| CU costs | `cycle_profile` | real guest cycles per instruction via the local executor (single vs batch amortization) |
| Bundles | `smoke_lgx.sh` | plugin loads, dispatches over the module ABI, fails closed without the Rust core, full chain with it |

The hermetic fixture server deserves a note: it is a real HTTP server
(tokio `TcpListener`) serving a Geofabrik-shaped index and files, including
the `X-Derived-From` header and a mutable published version for germany — so
the update-check path (local 20260821 → published 20260822) is exercised
without touching the real Geofabrik. Flakiness by network and GH-runner
egress policy is thereby zero; fidelity is preserved by speaking the exact
wire format.

## 9. What is deliberately NOT here

- **No decrypt/encrypt** — map data is public; integrity is the product (§3).
- **No owner-gated registration** — permissionless by design (§4).
- **No mirror removal / history rewrite** — append-only (§4).
- **No off-chain indexer** — every query the prize lists (by region, by
  parent, by CID) reads registry/region accounts directly.
- **No Geofabrik polling loop** — `update_check` is pull-based; a cron of
  `osm update --region …` covers the "monitor for new versions" criterion
  without a long-running service to operate.
