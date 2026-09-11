# logos-osm — build state (LP-0018 / PR #71: OpenStreetMap integration)

> Working doc for whoever picks this up (agent or human). Update per milestone.
> Last updated: 2026-08-22, 14:3x (by Claude — ALL GREEN: public testnet lifecycle + all 4 CI jobs on main). Only user-only steps remain.

## Mission

Build a complete LP-0018 PR #71 submission (the **OpenStreetMap integration
variant** of LP-0018) in `~/logos-osm`, regardless of which LP-0018 PR merges.
This is hedge #2 of 4 in the FCFS hedge plan (vault #75 = done, this = OSM
#71). Fresh dir, original code, LEZ stack pinned **v0.2.4**. `~/logos-agent`
is **read-only** (another agent uses it); `~/logos-vault` is a pattern
reference (and its Codex container is reused here).

## Status: ✅ COMPLETE — public testnet GREEN, all 4 CI jobs GREEN on main

| Area | State |
|---|---|
| Closed region set (72 regions, osm-core) | ✅ done, tested |
| SDK (geofabrik, verify, storage, registry, osm facade, retry, ffi) | ✅ done, 40+ lib tests green |
| LEZ guest (methods/osm) + committed artifact + host embed | ✅ done, guest unit tests green |
| CLI (all 11 subcommands incl. local-import full workflow) | ✅ done |
| FFI C ABI + Qt module + QML app + flake + module.json | ✅ GREEN (lgx built, smoke_lgx full-chain OK) |
| Hermetic off-chain lifecycle test | ✅ GREEN (CI-runnable, loopback only) |
| Off-chain lifecycle on real Codex (vault-codex container) | ✅ GREEN |
| Live sequencer lifecycle (osm_registry_live) | ✅ GREEN dev (531.76 s) + RISC0_DEV_MODE=0 (1013.99 s, real Groth16) |
| **Public LEZ testnet lifecycle (osm_registry_testnet)** | ✅ **GREEN (2026-08-22)** — see milestone below |
| CU cycle profile | ✅ DONE + GREEN (numbers in docs/CU_COSTS.md) |
| CI workflow (4 jobs) | ✅ **all 4 jobs GREEN** on f4807b9 (run 32576280128): Build+unit, Live LEZ e2e, CU profile, .lgx bundles |
| README, docs/DESIGN.md, submission/LP-0018.md, scripts/demo.sh | ✅ written; submission testnet/CI placeholders filled |
| docs/CU_COSTS.md, docs/PERFORMANCE.md | ✅ measured + filled |
| lgx build + smoke_lgx.sh on VPS | ✅ GREEN |
| GitHub repo + push + CI green + testnet deploy | ✅ **ALL DONE (2026-08-22)** — repo live, testnet GREEN, CI 4/4 GREEN (run 32576280128 on f4807b9) |

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
6. ✅ fmt + clippy clean (41m29 s full clippy); testnet test run GREEN
   (milestone below); all ids recorded in README/submission/STATE.
7. ✅ Repo **logos-osm** created + pushed via the token flow (token scrubbed
   from remote + verified zero refs). Commits: 9683754 → af91d30 → 3ddd00d
   → f4807b9 (no co-author). CI: run 32543830322 green on the first push
   except the two test-target compile errors this machine never compiled —
   fixed in f4807b9; decision run 32576280128 in flight (CU + lgx jobs
   already GREEN).
8. ✅ Memory `lp18-project.md` updated per milestone; this STATE.md updated.

## ✅ Milestone — PUBLIC TESTNET LIFECYCLE GREEN (2026-08-22)

`RISC0_DEV_MODE=0 cargo test -p osm-integration-tests --test
osm_registry_testnet -- --ignored --nocapture` against
`https://testnet.lez.logos.co`: **1 passed, 0 failed, 771.99 s, exit 0.**

| Step | Evidence |
|---|---|
| Deploy | tx `0xea6ac729f45586af54ed1a73c8008211f25d850db50723fe820f2723d7021f51` |
| Program | `77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0` (= committed artifact id) |
| Init | tx `0x2ab1afc37b68ecc66c075b916677866c6efae6cb0e87f3399587d56f5ff14bed` |
| Registry PDA | `fdcd6be67c17d9164eef31d75aa76aaafef33e14929afc9c0750414219d64ac7` |
| RegisterRegion(germany) | tx `0x4934796a05abfdaf6e186724bf722117f48aa05231c9fdb6c6501a87e5679c06` |
| germany PDA | `bb1f5e545c4fbbe6d35b01616516565b87417f074ca110467ee9b9685c8506b8` |
| RegisterRegionsBatch | tx `0x28e8c4d39387a36eef82a4b768732ee9203298d030e1ac115082d2762cb937e7` |
| Readback | `3 regions, 3 registrations` (germany, france, us/california), blocks ~18830→18843 |

Full log: `docs/demo-evidence/osm_registry_public_testnet.log`. The rule-7
fix (registrar claim-on-first-touch) is now proven on the **public** testnet,
not just the standalone sequencer.

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

## ✅ Milestone — ADVERSARIAL REVIEW (independent agent, 2026-08-22)

A fresh execution-empowered verifier agent evaluated this repo against the
submission checklist. Full report:
`docs/adversarial-review-2026-08-22-osm.md`. **Verdict: sound-with-findings.**

It independently reproduced, all exit 0: 65 unit tests (SDK 40 + core 11 +
registry host 3 + guest bin 11), the hermetic and real-Codex off-chain e2e,
the CU cycle profile **exactly** matching `docs/CU_COSTS.md`, fmt clean,
`nix build` both bundles + `smoke_lgx.sh` ("SMOKE OK", Rust-core
round-trip), a live Geofabrik resolve of all 72 regions, the live sequencer
lifecycle in dev mode (399 s, incl. the rule-7 batch step), and the greece
`.md5`/`X-Derived-From` against `docs/PERFORMANCE.md`. It also confirmed
the program-id pin (build.rs words ↔ committed artifact) and — secondarily —
that all 5 vault adversarial fixes are present in `~/logos-vault`.

Findings, all Low, all addressed same day:

| # | Finding | Fix |
|---|---|---|
| L1 | workspace-wide clippy NOT green (guest: dead code, digit grouping, manual RangeInclusive); CI clippy scoped to host crates | guest source cleaned + CI clippy widened to `--workspace --exclude osm-guest-builder` (verified locally, exit 0; guest bin tests still 11/11) |
| L2 | STATE.md said "12 subcommands" | corrected to 11 (verified against `enum Cmd`) |
| L6 | submission said retry.rs has "5 unit tests" (actual 4) | corrected to 4 |
| L4 | testnet log doesn't echo `RISC0_DEV_MODE=0` | honesty-ledger note added (timing + server-side proof verification corroborate) |
| NIV | DESIGN.md implied block-scoped `allow(unsafe)` | wording corrected to module-level, file-scoped |
| L3 | update-check vs local catalog (already disclosed) | no action — disclosed |
| L5 | `osm-perf/` 324 MB leftover (gitignored) | no action |

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
- ~~Testnet deploy NOT yet done~~ **DONE 2026-08-22** — evidence above; the
  public testnet was slow (~13 min for the lifecycle: deploy/Init/3 txs each
  waiting on inclusion), but every tx landed and the readback verified.
- The captured testnet log
  (`docs/demo-evidence/osm_registry_public_testnet.log`) does not echo the
  `RISC0_DEV_MODE` env var itself — "real proofs" for that run is
  builder-attested, corroborated by timing (~96 s/proving tx, consistent
  with the standalone dev0-vs-dev delta) and the public sequencer verifying
  proofs server-side. Future captures should prepend `env | grep RISC0`.
- **Guest source vs deployed artifact (2026-08-22, post-deploy clippy
  cleanup):** the deployed/tested bytes remain the **committed artifact**
  `methods/osm-host/osm_registry.bin` (id pinned in build.rs; unchanged).
  The guest *source* has since had clippy-only edits (dead-code `#[cfg(test)]`,
  date-bound consts, `RangeInclusive::contains`) — semantics-identical, but a
  rebuild from current source would not be byte-identical (guest builds are
  not byte-reproducible anyway; the artifact is the authority, per the
  program-id pinning test). No redeploy needed or performed.
