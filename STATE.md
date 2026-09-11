# logos-osm — build state (LP-0018 / PR #71: OpenStreetMap integration)

> Working doc for whoever picks this up (agent or human). Update per milestone.
> Last updated: 2026-08-22 (by Claude during the "iterate until done" run).

## Mission

Build a complete LP-0018 PR #71 submission (the **OpenStreetMap integration
variant** of LP-0018) in `~/logos-osm`, regardless of which LP-0018 PR merges.
This is hedge #2 of 4 in the FCFS hedge plan (vault #75 = done, this = OSM
#71). Fresh dir, original code, LEZ stack pinned **v0.2.4**. `~/logos-agent`
is **read-only** (another agent uses it); `~/logos-vault` is a pattern
reference (and its Codex container is reused here).

## Status: functionally complete; verification + ship-out in flight

| Area | State |
|---|---|
| Closed region set (72 regions, osm-core) | ✅ done, tested |
| SDK (geofabrik, verify, storage, registry, osm facade, retry, ffi) | ✅ done, 40+ lib tests green |
| LEZ guest (methods/osm) + committed artifact + host embed | ✅ done, guest unit tests green |
| CLI (all 12 subcommands incl. local-import full workflow) | ✅ done |
| FFI C ABI + Qt module + QML app + flake + module.json | ✅ written; **smoke on VPS pending** |
| Hermetic off-chain lifecycle test | ✅ GREEN (CI-runnable, loopback only) |
| Off-chain lifecycle on real Codex (vault-codex container) | ✅ GREEN |
| Live sequencer lifecycle (osm_registry_live) | 🔁 dev run FAILED at batch (rule-7 `NonDefaultAccountWithDefaultOwner`); **fix applied to guest** (registrar claimed via `new_claimed_if_default(Claim::Authorized)`), awaiting guest rebuild + re-run |
| CU cycle profile | ✅ DONE + GREEN (numbers in docs/CU_COSTS.md; re-run after artifact re-pin) |
| CI workflow (.github/workflows/ci.yml, 4 jobs) | ✅ written; unit job now `--lib --bins` (guest bin tests included); **not yet pushed/run** |
| README, docs/DESIGN.md, submission/LP-0018.md, scripts/demo.sh | ✅ written |
| docs/CU_COSTS.md, docs/PERFORMANCE.md | ⏳ awaiting measurements |
| lgx build + smoke_lgx.sh on VPS | ⏳ pending |
| GitHub repo + push + CI green + testnet deploy | ⏳ pending (task #17) |

## Key facts (don't re-derive)

- **Program ID (pinned, committed artifact):**
  `77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0`
  — `methods/osm-host/osm_registry.bin` (471,820 bytes), id pinned in its
  build.rs; `tools/guest-builder` is the only rebuild path (needs risc0
  toolchain; excluded from CI host builds via `--exclude osm-guest-builder`).
- **PDA seeds:** registry `b"/OSM/REGISTRY/V1/SEED/0000000000"`; region
  `SHA-256("osm-region:" || path)`. MAX_BATCH=24, MAX_MIRRORS=32.
- **Workspace crates:** `logos-osm` (sdk, cdylib `liblogos_osm.so`),
  `logos-osm-cli` (bin `osm`), `osm-core` (shared types + set), `methods/osm`
  (guest), `osm-registry` (host embed), `osm-integration-tests`,
  `osm-guest-builder`. LEZ deps pinned tag v0.2.4 in workspace Cargo.toml.
- **Version signal:** `X-Derived-From` header on the `.md5` (primary) or dated
  filename in the `.md5` body; `YYYYMMDD`. Geofabrik also 302-redirects
  `-latest` → dated file (observed 2026-08-22; reqwest follows by default).
- **Geofabrik mirror():** SDK `Geofabrik::new(base)` swaps the host in
  canonical URLs → fixture server in tests, real Geofabrik in prod. No URL
  plumbing elsewhere.
- **Codex container on this VPS:** `vault-codex` (port 8080), digest
  `sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb`,
  entrypoint override required (`--entrypoint /usr/local/bin/storage`). Reused
  for OSM real-Codex tests + demo.
- **Region sizes (measured 2026-08-22):** greece 323 MB, kenya 333 MB,
  denmark 470 MB, belgium 660 MB, japan 2.4 GB. Smallest sample = greece.
- **Wallet boundary:** SDK builds txs (JSON: accounts, identity kinds,
  instruction words), wallet submits. FFI never signs.
- **Two SDK bugs found + fixed by hermetic test:** (1) geofabrik base
  override ignored in absolute URLs → mirror(); (2) `import_local` clobbered
  hosted catalog provenance → md5-matched merge.

## Commands (verified working)

```sh
cd ~/logos-osm
cargo test --workspace --lib --bins --exclude osm-guest-builder    # unit (incl. guest bin tests)
cargo test -p osm-integration-tests --test offchain_live    # hermetic e2e
# real Codex (vault-codex must be up):
docker start vault-codex  # if stopped
OSM_CODEX_URL=http://127.0.0.1:8080 cargo test -p osm-integration-tests --test offchain_live -- --ignored
# live sequencer (Docker stack auto-brings-up; needs risc0 toolchain):
RISC0_DEV_MODE=1 cargo test -p osm-integration-tests --test osm_registry_live -- --ignored --nocapture --test-threads=1
# CU profile:
cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored --nocapture
# bundles + smoke:
nix build .#osm-lgx .#osm-app-lgx && ./scripts/build-ffi.sh && ./scripts/smoke_lgx.sh
# demo (real proofs): ./scripts/demo.sh
```

## Remaining work (in order)

1. ✅ **Guest artifact rebuilt + re-pinned** (2026-08-22): new id
   `77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0`
   (artifact 471,820 bytes) — swept through all 9 files incl. build.rs words
   + testnet EXPECTED_PROGRAM_ID_HEX; 0 stale refs remain.
2. ✅ **`osm_registry_live` GREEN both modes (2026-08-22)** — dev 531.76 s
   and `RISC0_DEV_MODE=0` **1013.99 s with real Groth16 proofs** (evidence:
   `docs/demo-evidence/osm_registry_live_dev0.log` + README). Full lifecycle
   incl. the previously-failing batch step: deploy → Init → 3 registrars ×
   germany (append-only) → RegisterRegionsBatch → readback asserts
   (region_count=4, registration_count=6). Rule-7 fix confirmed on-chain.
3. ✅ cycle_profile re-run on the new artifact (2026-08-22): GREEN, numbers
   within 0.3% of pre-fix (Init 96,142; fresh 161,127; append 222,820;
   ×10 766,758; ×24 1,704,499; 87,381 total/region) — CU_COSTS.md updated.
4. ✅ PERFORMANCE.md store column filled (2026-08-22): CLI `osm host greece`
   339,431,546 B in 219.8 s wall, peak RSS 32 MB, CID `zDvZRwzm…dPKa8j4H`.
5. ✅ **lgx + smoke GREEN (2026-08-22)**: `nix build .#osm-lgx .#osm-app-lgx`
   both bundles built; `smoke_lgx.sh` full-chain OK (plugin loads, ABI 0.5.0,
   Rust core dispatches, filtered regions query works). One real bug found +
   fixed **in the smoke script itself**: the args-array element must be a
   JSON *string* of the args object — Python eats one level of `\"`, so the
   wire bytes need `\\"`; malformed outer JSON → nlohmann throws → NULL
   return. (The plugin glue is correct — verified by disassembly: parse,
   per-element string type-check, then `OsmImpl::invokeOpJson`.)
   `build-ffi.sh` (glibc-matched cdylib) in flight.
6. fmt + clippy clean, run testnet test (`RISC0_DEV_MODE=0 cargo test …
   --test osm_registry_testnet -- --ignored`) → record ids in README/
   submission; commit everything.
7. Task #17: create private GitHub repo **logos-osm**, push via the token
   flow (TOKEN env var, push, then `git remote set-url origin` back to the
   clean URL — **always scrub the token**; user should rotate it: it was
   pasted in plaintext in a prior session), iterate CI green.
8. Update memory files (`lp18-project.md`) + this STATE.md.

### Rule-7 postmortem (2026-08-22)

Dev live run failed exactly at step 7 (batch, registrar_c's **2nd** tx):
`NonDefaultAccountWithDefaultOwner { DDYvJq… }`. Mechanism: a signer's first
tx writes their still-default account fine; the sequencer then advances
their nonce → account non-default, program_owner still default → every
**subsequent** tx by that signer fails unless the program claims the
account. Fix (guest): registrar post-state is now
`AccountPostState::new_claimed_if_default(acc, Claim::Authorized)` in both
Register instructions — claim on first touch, plain write after. Same
lesson as the vault's owner-at-Init, generalized to permissionless
registrars. Unit test `registrar_claimed_on_first_touch_only` pins it.
**Fix validated 2026-08-22**: guest bin tests 11/11 green (incl. the rule-7
test, run by name to confirm), artifact rebuilt + re-pinned, cycle profile
re-measured. Live dev-mode re-run in flight at the time of this note.

## User-only (never do these)

- Record + upload narrated video (must show proof gen = RISC0_DEV_MODE=0).
- File GitHub issues for Logos tech problems.
- Open the solution PR.

## Honesty ledger

- The spec asks for an IDL "using the SPEL framework": we hand-rolled on
  `lee_core::program` (framework pins incompatible nssa_core v0.2.0-rc3 vs
  stack v0.2.4) — documented in submission + DESIGN.
- `source_url` from the A.2 schema is *derived* from the embedded table, not
  stored per-entry — documented in submission.
- Update check compares Geofabrik-published vs **local catalog** version (the
  client's view of what it registered); the on-chain registry readback is via
  the wallet/RPC (osm_registry_live proves on-chain state) — documented.
- Testnet deploy NOT yet done (unlike vault). Don't claim it until step 7.
