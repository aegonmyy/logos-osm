# Adversarial review — logos-osm (LP-0018 / PR #71: OpenStreetMap integration)

Independent execution-empowered verifier. Date: 2026-08-22. Reviewer ran
everything itself; doc claims were checked against the code, the live
Geofabrik server, the running `vault-codex` container, and the public LEZ
testnet endpoint.

## Verdict

**sound-with-findings** — the submission's core claims hold up under
execution. All cheap-to-run evidence (unit, hermetic e2e, real-Codex e2e,
CU profile, nix `.lgx` build, `smoke_lgx.sh`, live Geofabrik 72-region
resolve) is green on my run and matches the docs' numbers. The committed
guest artifact id, PDAs, and tx hashes are internally consistent across
`submission/LP-0018.md`, `STATE.md`, `README.md`, and the captured logs.
The findings below are precision/scope issues, not functional defects.

## What I verified myself (tools + results)

| Check | Command | Result |
|---|---|---|
| Unit tests (workspace) | `CARGO_BUILD_JOBS=2 cargo test --workspace --lib --bins --exclude osm-guest-builder` | **exit 0** — logos_osm 40/40, osm_core 11/11, osm_registry 3/3, guest bin 11/11 (incl. `registrar_claimed_on_first_touch_only`) |
| Hermetic off-chain e2e | `cargo test -p osm-integration-tests --test offchain_live` | **exit 0** — `hermetic_lifecycle_on_fixture` ok; real-Codex variant `ignored` |
| Real-Codex off-chain e2e | `OSM_CODEX_URL=http://127.0.0.1:8080 cargo test -p osm-integration-tests --test offchain_live -- --ignored --nocapture` | **exit 0** — `offchain_lifecycle_on_real_codex` ok (against the running `vault-codex` container) |
| CU cycle profile (release) | `cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored --nocapture` | **exit 0** — Init 96,142/262,144; fresh 161,127/262,144; append 222,820/524,288; ×10 766,758/1,048,576; ×24 1,704,499/2,097,152 — **exact match** to `docs/CU_COSTS.md` |
| `cargo fmt --all -- --check` | as CI | **exit 0** |
| Clippy, CI scope (4 host crates) | `cargo clippy -j2 -p logos-osm -p logos-osm-cli -p osm-core -p osm-registry --all-targets -- -D warnings` | **exit 0** (matches CI; this is what CI runs) |
| Clippy, integration-tests crate | `cargo clippy -j2 -p osm-integration-tests --all-targets -- -D warnings` | **exit 0** |
| Clippy, workspace-wide (review command) | `cargo clippy -j2 --workspace --exclude osm-guest-builder --all-targets -- -D warnings` | **FAIL (exit 0 from the pipe, but 6 clippy errors)** on `osm-registry-guest` — see Finding L1 |
| `.lgx` bundle build | `nix build .#osm-lgx .#osm-app-lgx --print-out-paths` | **exit 0** — both store paths printed |
| `smoke_lgx.sh` full chain | `./scripts/smoke_lgx.sh` | **exit 0** — "SMOKE OK: module loads, ABI negotiates, Rust core dispatches"; ABI 0.5.0; methods enumerated; `liblogos_osm.so` round-trip |
| Live Geofabrik 72-region resolve | `cargo test -p logos-osm --lib geofabrik_live -- --ignored --nocapture` | **exit 0** — all 72 closed-set regions present in the live `index-v1-nogeom.json`; kenya `.md5` parsed, version `20260821` |
| Real Geofabrik greece `.md5` | `curl https://download.geofabrik.de/europe/greece-latest.osm.pbf.md5` | MD5 `d05c23af3b8d82af80f622adc8e9a7b7`, `X-Derived-From: europe/greece-260821.osm.pbf.md5` — **exact match** to `docs/PERFORMANCE.md`'s published checksum + version-recovery claim |
| Program-id pin consistency | `python3` decode of build.rs `[u32;8]` vs hex | **match** — `77ecdf2f…1c43f0` decodes to the exact words in `methods/osm-host/build.rs`; artifact on disk is 471,820 bytes (matches the `ProgramDeployment` log line and `STATE.md`) |
| Live sequencer test (dev mode) | `RISC0_DEV_MODE=1 cargo test -p osm-integration-tests --test osm_registry_live -- --ignored --nocapture --test-threads=1` | **exit 0** — `osm_registry_full_lifecycle_on_sequencer` ok, finished in **399.42 s** (dev mode; the doc's 531.76 s and dev0's 1013.99 s are this-machine/other-mode runs). The test drives deploy → Init → germany by **3 distinct registrars** (20260801 → 20260815 → 20260821, append-only) → `RegisterRegionsBatch{france, us/california, kenya}` → readback `region_count=4, registration_count=6` — the full lifecycle including the batch step that exercises the rule-7 fix |
| Public testnet explorer probe | `curl https://explorer.testnet.lez.logos.co/account/<registry-PDA-hex>` | returns 200 but the page body is `"ServerError":"Invalid account ID"` — the explorer rejects raw-hex account ids (same response for a bogus PDA), so I could **not** independently confirm the on-chain deployment via the explorer; the testnet log remains the primary evidence |

## Findings

### L1 (Low) — Workspace-wide clippy is NOT green; "clippy clean" is over-scoped

- **What:** `cargo clippy -j2 --workspace --exclude osm-guest-builder
  --all-targets -- -D warnings` fails on `osm-registry-guest` with 6
  errors (4 on the bin target, 3 on the test target — the test target
  shares 3 of the 4). The errors are:
  1. `function region_pda_id is never used` (`methods/osm/src/bin/osm_registry.rs:71`) — the helper is dead in the bin target (`register_one` inlines the PDA derivation at line 244).
  2. `digits grouped inconsistently by underscores` (×2) — the version-range assert at line 255: `1_970_010_1 && … 9_999_123_1`.
  3. `manual RangeInclusive::contains implementation` — same version assert.
- **Why it matters:** `STATE.md` says "fmt + clippy clean (41m29 s full
  clippy)" and `submission/LP-0018.md` lists "Build + unit tests (fmt,
  clippy, build-all-targets incl. every test target, …)". CI's clippy
  step is actually scoped to **four host crates** (`-p logos-osm -p
  logos-osm-cli -p osm-core -p osm-registry`) and excludes the guest
  crate and `osm-integration-tests`. A reviewer running the
  workspace-wide clippy gate gets failures. The guest compiles and its
  unit tests pass, so this is cosmetic — but the "clippy clean" claim is
  only true for the CI-scoped subset, not the workspace.
- **Fix:** (a) gate `region_pda_id` with `#[cfg(test)]` (only the test
  module calls it), or delete it and inline the derivation in tests; (b)
  regroup the version constants to `1_970_01_01`/`9_999_12_31` (or
  `19700101`/`99991231` without underscores); (c) replace the manual
  range check with `(1_970_01_01..=9_999_12_31).contains(&version)`. Then
  widen CI's clippy step to `--workspace --exclude osm-guest-builder` so
  the gate matches the claim.

### L2 (Low) — CLI subcommand count is off by one in STATE.md

- **What:** `STATE.md:22` says "CLI (all 12 subcommands incl. local-import
  full workflow)". The actual `enum Cmd` in `cli/src/main.rs` has **11**
  variants: `Regions, Discover, Host, HostBulk, Register, RegisterBulk,
  Init, Fetch, Import, Update, Catalog`. The FFI surface has 13 ops
  (`open, status, regions, discover, host, host_bulk, register,
  register_bulk, init, fetch, import, update, catalog`). "12" matches
  neither. `submission/LP-0018.md` does not make this claim — it lists
  the ops without a count — so the binding submission is unaffected.
- **Fix:** correct `STATE.md` to "11 subcommands" (or "13 FFI ops") —
  internal doc only.

### L3 (Low) — `update_check` compares against the local catalog, not the on-chain registry

- **What:** `osm-sdk/src/osm.rs:361` `update_check` compares Geofabrik's
  published version against the **local catalog** (`catalog.json`), not
  the on-chain registry. The submission's success-criterion text says
  "compares per-region on the region path (country vs subregion tracked
  independently)" — which is accurate (the catalog is keyed by region
  path) — but a reader could infer "compares against the registry". The
  on-chain readback is proven by `osm_registry_live`, not by
  `update_check`.
- **Why it matters:** this is **already disclosed** in the honesty
  ledger (`STATE.md:169`): "Update check compares Geofabrik-published vs
  **local catalog** version … documented." So it is not an overclaim —
  it is honest. Flagging only because the success-criterion wording is
  slightly looser than the implementation. No fix required; the
  disclosure is accurate.

### L4 (Low) — Testnet log does not echo `RISC0_DEV_MODE=0`

- **What:** `docs/demo-evidence/osm_registry_public_testnet.log` is a
  48-line capture that shows deploy → Init → RegisterRegion →
  RegisterRegionsBatch → readback, exit 0, 771.99 s. It does **not**
  print the `RISC0_DEV_MODE` env value, so "the testnet run used real
  Groth16 proofs" is builder-attested, not log-proven.
- **Corroborating evidence I checked:** the testnet lifecycle (post
  initial sync) took ~234 s for 3 proving txs (Init + RegisterRegion +
  RegisterRegionsBatch; the deploy tx carries bytecode, not a guest
  receipt). The standalone `RISC0_DEV_MODE=0` run
  (`osm_registry_live_dev0.log`) took 1013.99 s vs 531.76 s in dev mode
  — a ~480 s delta for 5 proving txs ≈ 96 s/proof. 3 proofs × ~96 s ≈
  288 s, in the same order of magnitude as the 234 s observed post-sync
  on the testnet. The timing is therefore consistent with real proofs
  (a dev-mode testnet run would have been much faster post-sync). Also,
  the public testnet sequencer verifies proofs server-side; dev-mode
  (fake) receipts would not be expected to be includable on a real
  network. I treat the claim as likely-true but not log-proven.
- **Fix:** re-run with `RUST_LOG=info` and prepend `echo RISC0_DEV_MODE=$RISC0_DEV_MODE` (or `env | grep RISC0`) to the captured log so the env is on-screen; or annotate the log header.

### L5 (Low) — `osm-perf/` leftover (324 MB) is gitignored but present

- **What:** `osm-perf/` (324 MB, the greece perf download) and
  `osm-state.json` exist on disk. Both are in `.gitignore` and `git
  ls-files` confirms they are **not** committed. No action needed;
  noting for completeness so a reviewer doesn't mistake the local
  working dir for the committed tree.

### L6 (Low) — Off-by-one counts in the submission: "5 unit tests" in `retry.rs` (actual: 4)

- **What:** `submission/LP-0018.md:167` claims "(`osm-sdk/src/retry.rs`,
  5 unit tests)". The module has **4** tests
  (`succeeds_on_second_attempt`, `fatal_returns_immediately`,
  `exhausts_after_max_attempts`, `status_classification`); my run
  confirms "4 passed". Unlike the STATE.md "12 subcommands" slip (L2),
  this one is in the **binding submission document**.
- **Fix:** change "5 unit tests" → "4 unit tests" in
  `submission/LP-0018.md` (Reliability → "Upload retries with backoff").

### NIV (non-issue) — `#![allow(unsafe_code)]` is module-wide in `ffi.rs`, not block-scoped

- `docs/DESIGN.md` §7 says the SDK is `#![deny(unsafe_code)]` "solely so
  `ffi.rs` … can scope `allow(unsafe)` with a comment". The actual
  `osm-sdk/src/ffi.rs:4` has `#![allow(unsafe_code)]` at **module**
  scope (inner attribute), so the whole ffi module opts out, not
  individual `unsafe` blocks. Effect is the same (only `ffi.rs` can use
  `unsafe`; the rest of the crate is `deny`), and `lib.rs:67` does set
  `#![deny(unsafe_code)]`. The three `unsafe` usages are the legitimate
  FFI boundary (`logos_osm_invoke`, `CString::from_raw`, the free fn).
  Not a defect; wording nit only.

## Honesty audit

| Doc claim | Held up? | Evidence |
|---|---|---|
| Program id `77ecdf2f…1c43f0`, artifact 471,820 bytes | ✅ | `methods/osm-host/osm_registry.bin` is 471,820 bytes; `build.rs` `[u32;8]` decodes to the hex; `osm_registry` lib unit test `program_id_is_stable_and_documented` passes |
| Rule-7 fix (`NonDefaultAccountWithDefaultOwner` → `new_claimed_if_default` claim-on-first-touch) is in the guest | ✅ | `methods/osm/src/bin/osm_registry.rs:211-213` `registrar_post_state` uses `new_claimed_if_default(... Claim::Authorized)`; unit test `registrar_claimed_on_first_touch_only` (line 519) passes; both `RegisterRegion` and `RegisterRegionsBatch` emit it |
| Testnet log exercises the batch step | ✅ | `osm_registry_public_testnet.log` line 42 shows `RegisterRegionsBatch` tx included in block 18843; `tests/tests/osm_registry_testnet.rs:247-258` builds and sends the batch and asserts both batch regions' entries |
| "3 regions / 3 registrations" on testnet | ✅ (accurate) | Final readback `3 regions, 3 registrations` — but note: the testnet test uses **one** registrar for all 3 registrations (germany + batch{france, us/california}). The "3 distinct registrars" bar is proven only by the **standalone** `osm_registry_live` (4 accounts: owner + 3 registrars), not by the testnet run. The submission does not claim 3 distinct registrars for the testnet run — it claims it for `osm_registry_live` (line 78). Consistent. |
| CU numbers in `CU_COSTS.md` | ✅ | My release-mode `cycle_profile` run reproduced all five rows exactly (Init 96,142/262,144; fresh 161,127/262,144; append 222,820/524,288; ×10 766,758/1,048,576; ×24 1,704,499/2,097,152; 87,381 total/region) |
| 3.0× batch amortization | ✅ | 262,144 ÷ 87,381 = 3.000; 161,127 ÷ 71,021 = 2.27 user-cycle amortization — matches the doc's "≈71,021/region" |
| MD5 verification / tamper-rejection | ✅ | `offchain_live.rs:345-354` tampers the checksum and asserts `fetch_snapshot` errors; hermetic test passes |
| "Geofabrik's published MD5" for greece = `d05c23af3b8d82af80f622adc8e9a7b7`, version `20260821` | ✅ | My `curl` of the live `.md5` returned exactly that hash and `X-Derived-From: europe/greece-260821.osm.pbf.md5` |
| "Live index must resolve all 72" | ✅ | `geofabrik_live_smoke` (ignored, network) passed: `available.len() == REGIONS.len()` (72) against the real `download.geofabrik.de` |
| 72-region non-overlapping partition (48 countries + 24 subregions) | ✅ (internally) | `set_is_non_overlapping`, `paths_are_unique`, `parents_are_consistent`, `decomposed_countries_are_not_themselves_in_the_set`, `expected_shape` (48/24/72/52) all pass. **Mapping to the prize's appendix A.4 is unverified** — see below |
| Hand-rolled IDL on `lee_core::program` (not `spel-framework`) | ✅ (accurately disclosed) | `methods/osm/src/bin/osm_registry.rs:30` and `osm-sdk/src/registry.rs:38` use `lee_core::program` directly; no `spel`/`spel-framework` references anywhere. The submission's honesty note (LP-0018.md:154-157) is precise |
| "Map data is public — nothing is encrypted" | ✅ | No encrypt/decrypt in the SDK; `DESIGN.md §9` explicitly lists "No decrypt/encrypt" |
| Permissionless registration | ✅ | Guest `RegisterRegion`/`RegisterRegionsBatch` check `registrar.is_authorized` (signing) but never check ownership/owner approval; no removal instruction |
| `.lgx` bundles build + `smoke_lgx.sh` passes | ✅ | I ran `nix build .#osm-lgx .#osm-app-lgx` (exit 0, both paths) and `./scripts/smoke_lgx.sh` (exit 0, "SMOKE OK") |
| CI green (run 32576280128, 4 jobs) | builder-attested | The repo is private during review; I did not use the GitHub token. I verified the **local equivalents** of every CI job except the live Docker-sequencer job (in flight at review time) |
| Recorded video demo | not done | `submission/LP-0018.md:232` correctly marks this `[ ]` as user-only |

## What I could NOT verify

- **Live sequencer test (`osm_registry_live`)** — **ran it myself** in dev
  mode: **exit 0, 399.42 s**, `1 passed`. The full lifecycle (deploy →
  Init → 3 registrars → batch → readback 4 regions / 6 registrations)
  executed against a standalone Docker LEZ stack the test harness
  brought up. The captured `osm_registry_live_dev0.log` (1013.99 s,
  `RISC0_DEV_MODE=0`) is internally consistent with the same test source
  and the deploy tx hash matches the testnet deploy hash
  (content-addressed deployment id — expected). I did not re-run the
  `RISC0_DEV_MODE=0` variant on this VPS (load was ~6.5 during the dev
  run); the dev0 capture plus the public-testnet capture stand as the
  real-proof evidence.
- **Public testnet deployment via the explorer** —
  `https://explorer.testnet.lez.logos.co/account/<hex-PDA>` returns 200
  but the page body is `{"Err":{"ServerError":"Invalid account ID"}}`,
  identical for the real registry PDA and a bogus `000…abc` PDA. The
  explorer does not accept raw-hex account ids (the wallet uses the
  `Public/…` form). The testnet RPC (`POST /v1/programs`) rejects
  unknown JSON-RPC methods. So I could not independently confirm the
  on-chain state via HTTP. The testnet log
  (`docs/demo-evidence/osm_registry_public_testnet.log`) is the primary
  evidence; it has realistic structure (block numbers 18830→18843, the
  537.44 s initial sync, per-tx inclusion lines, 771.99 s total, exit 0)
  and its program id, PDAs, and tx hashes are consistent across
  `submission/LP-0018.md`, `STATE.md`, and `README.md`. The deploy tx
  hash matching the standalone-sequencer deploy hash is expected
  (content-addressed deployment of the same 471,820-byte ELF) and is
  consistent rather than suspicious.
- **Mapping of the 72-region set to the actual LP-0018 prize appendix
  A.4** — there is no raw spec copy in the repo. The set's *internal*
  invariants (non-overlap, 48/24/72/52 shape, decomposed countries
  excluded) are unit-tested and pass, and the live Geofabrik index
  resolves all 72 paths the table records. But I cannot trace the
  table's *contents* to the binding prize text; "mapping unverified" in
  the spec-vs-code sense. (The submission asserts the set is "frozen by
  the prize document"; I take that on trust.)
- **PERFORMANCE.md full-region numbers beyond greece** — only greece is
  measured end-to-end; kenya/denmark have `.md5`-only timings. The
  greece checksum and version match the live server. The other region
  sizes (333 MB–2.4 GB) are stated as "measured 2026-08-22" in
  `STATE.md` but I did not re-download them; they are plausible and not
  load-bearing for any criterion.

## Secondary: vault fixes present? (`~/logos-vault`, commit `c3825dd`/`17aea1b`)

| Fix | Present? | Evidence |
|---|---|---|
| `package_cids` field on `Capability` (`vault-sdk/src/messaging.rs`) | ✅ | `messaging.rs:42` `pub package_cids: Vec<String>`; defaulted at 325/353 |
| `GrantSalt::derive` (`vault-sdk/src/grants.rs`) | ✅ | `grants.rs:149` `pub fn derive(vault_id: &[u8; 32], spec: &GrantSpec) -> Self` |
| `expiry != 0` asserts in the tx builders | ✅ | `grants.rs:270` and `:301` `assert!(g.expiry != 0, "grant expiry must be non-zero (mandatory)")` (the tx builders live in `vault-sdk/src/grants.rs`); `security.md` §2 documents "the tx builders assert `expiry != 0`" |
| `docs/security.md` §2.4/§2.5 rewritten | ✅ | `docs/security.md` §2 reflects the composite rotate+RevokeGrant (+ optional re-grant) model; commit `c3825dd` message explicitly records "F2 [Med] security.md §2.4 overclaim … §2.5 split out" |
| Two-tier smoke wording in `submission/LP-0018.md` | ✅ | `submission/LP-0018.md:285-286` "load-and-dispatch verified by `scripts/smoke_lgx.sh` in two tiers: the full-FFI smoke … ran on the build" |

All five claimed adversarial-review fixes are present in the vault source.
