# logos-osm — build state (LP-0018 / PR #71: OpenStreetMap integration)

> Working doc for whoever picks this up (agent or human). Update per milestone.
> Last updated: **2026-08-28** (by Claude — handoff refresh; see the next
> section). Earlier: 2026-08-22 ALL GREEN: public testnet lifecycle + all 4
> CI jobs on main. Only user-only steps remain.

## 🤝 STATE AT HANDOFF (2026-08-28) — read this first

**Everything needed is on `main`, verified against the remote:**

- Local `main` `a8840f9` == remote `main` (`aegonmyy/logos-osm`), working
  tree clean. CI **all 4 jobs GREEN** at the tip (`a8840f9ad`, plus the prior
  tip `1599b61bf` — both `completed/success`).
- Tip = demo-evidence commit (asciinema cast). Last functional state: CI
  4/4 on run 32576280128; adversarial review passed with all 6 Low findings
  fixed (see milestone below).
- Contest scene: the LP-0018 prize PR (`logos-co/lambda-prize#75`, weboko)
  is **still unmerged**; a third-party submission in this same lane
  (mart1n-xyz, #71, updated 2026-08-27) is active. Our solution PR is
  **not opened** (user-only, when #75 merges).

**If you are a successor agent, the rules that keep this repo shippable:**

1. **NEVER commit `tests/tests/adoption.rs`.** It is an operator tool kept
   out of the tree on purpose via `.git/info/exclude` (that file is local —
   a fresh clone won't have it; on this VPS it's already excluded). Its ops
   docs and ledger live in the private repo `aegonmyy/osm-adoption`
   (checkout at `~/osm-adoption-campaign/`; read its `STATE.md` first).
2. `~/logos-agent` is read-only (another agent owns it). `~/logos-vault` is
   a sibling hedge (read for patterns, don't mix histories).
3. Before any push: `cargo fmt --all -- --check` AND
   `cargo clippy --workspace --exclude osm-guest-builder --all-targets` AND
   compile every `--test` target locally — CI enforces all three and this
   machine's default `cargo test` invocation does NOT build test targets.
4. Push flow (scrub after every use):
   `TOKEN='…'; git push "https://$TOKEN@github.com/aegonmyy/logos-osm.git" HEAD:main`
   then verify `git remote -v | grep -c ghp_` = 0. Token is user-rotating.
5. Never add Claude as co-author on commits.
6. The committed artifact `methods/osm-host/osm_registry.bin` is the
   authority for the deployed program id `77ecdf2f…`; do not rebuild/re-pin
   casually (guest builds are not byte-reproducible — see honesty ledger).

**What remains is user-only:** narrated-video voiceover over the committed
cast, filing GitHub issues, opening the solution PR (and, on the campaign
side, whatever the user's redesign decides — see the private repo's STATE.md).
Agent-side work is complete unless the prize scene moves (competitor
activity, testnet reset, or #75 merging and surfacing new requirements).

## ⚙️ CI infrastructure change (2026-08-28) — self-hosted runners

**What broke:** at 2026-08-28T02:00Z all 4 CI jobs on `main` failed
instantly with no logs. The check-run annotation read: "The job was not
started because recent account payments have failed or your spending limit
needs to be increased." This is a GitHub **account-level billing gate** on
hosted runners for private repos, not a code problem. Every job had
`runner_id: 0` (never picked up). Prior green run: 32595305722 (2026-08-22).
Identical failure hit the sibling `logos-vault` at the same minute; both
were fixed the same way.

**Fix:** self-hosted runners on the logos-box VPS are free and not gated by
hosted-runner billing. Two dedicated runners were registered (one per hedge
repo) and CI switched off hosted runners:

- Runner install: `/home/ubuntu/actions-runner-osm`, GitHub Actions Runner
  v2.337.0, name `logos-box-osm`. (Sibling: `logos-box-vault`.)
- systemd service: `actions.runner.aegonmyy-logos-osm.logos-box-osm.service`.
- The runner exports its PATH from a `.path` file:
  `/home/ubuntu/actions-runner-osm/.path` includes `~/.cargo/bin`,
  `~/.risc0/bin` (cargo-risczero 3.0.5 / r0vm 3.0.5 / rust 1.97.0) and
  `/nix/var/nix/profiles/default/bin` (nix). This is what makes the risc0
  toolchain and nix visible to the CI jobs. If a runner is ever re-registered,
  re-create that `.path`.

**Workflow changes (`.github/workflows/ci.yml`, commit `012a5cb`):**
- All 4 jobs now `runs-on: [self-hosted, Linux, X64]`.
- `CARGO_BUILD_JOBS: "2"` in the env block (the box is shared; cap build
  parallelism to keep load sane).
- Hosted-only steps are guarded with `if: env.RUNNER_IS_GITHUB_HOSTED ==
  'true'` (set on GitHub-hosted runners, unset on self-hosted): disk
  reclamation (self-hosted skips and returns 0), `Install system deps` (apt),
  `Install risc0 toolchain` (the box already has it).
- `Install nix` (DeterminateSystems/nix-installer-action) replaced with an
  availability check that errors if nix is missing.
- Off-chain Codex step now **reuses a live codex on :8080** when present
  (`vault-codex` is a shared box resource) instead of always starting a new
  container, which would collide on the port.
- Digest-pinned images and the Codex `--entrypoint /usr/local/bin/storage`
  override are preserved.

**How to run CI locally (same as the runner):** the runner jobs use the box's
installed toolchains; the box is the logos-agent build box, so
`cargo-risczero`/`r0vm`/`nix`/`protobuf-compiler`/`libpcsclite-dev` are all
present. Runner service control: `sudo systemctl {start,stop,restart}
actions.runner.aegonmyy-logos-osm.logos-box-osm.service`.

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

## 🎬 Milestone — DEMO VIDEO CAPTURE, asciinema detached (2026-08-22)

Spec: "A recorded video demo … must show terminal output (including proof
generation) to confirm `RISC0_DEV_MODE=0` was active". Same process as the
vault hedge (per user: asciinema, zoomed out, no text overlays, detached
from the agent session, voiceover later):

- `scripts/demo.sh` log filter upgraded to
  `RUST_LOG='warn,risc0_zkvm=info,indexer_core=off'` + `RISC0_INFO=1` —
  surfaces the risc0 per-session proof summary on screen (two non-obvious
  gates: EnvFilter target is the crate name `risc0_zkvm` NOT `risc0`, and
  `Session::log()` early-returns without `RISC0_INFO` set — risc0-zkvm
  3.0.5) and silences the standalone indexer follower (benign parking;
  sequencer is the authority — see vault ISSUES_TO_FILE.md #9, same stack).
- `scripts/record-demo.sh` (new): records demo.sh under `asciinema rec`
  inside a **detached tmux session created `-x 200 -y 50`** (launch:
  `setsid nohup bash scripts/record-demo.sh`). Sizing lesson from the vault:
  asciinema allocates its pty at ITS terminal's size — a post-hoc `stty`
  records at 80×24; the tmux pane size is inherited correctly.
- Cast: `docs/demo-evidence/osm-demo-dev0-20260822T195048Z.cast` (200×50,
  started 19:50:48Z, detached). The demo-evidence README previously said
  the recording is "done by the submitter outside this repo" — now the cast
  is committed in-repo; only the voiceover remains user-only.

## User-only (never do these)

- Record + upload narrated video — voiceover over the committed cast
  (`asciinema play docs/demo-evidence/osm-demo-dev0-20260822T195048Z.cast`
  + screen recorder, or `agg` to convert; must show proof gen =
  RISC0_DEV_MODE=0).
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

## 2026-08-28 manager note (hedge CI) — UPDATED 04:20Z
- ci.yml commit `012a5cb` ("run on self-hosted runners (logos-box) instead of
  hosted") is committed on main, working tree clean, NOT yet pushed. YAML
  validated locally.
- Vault run 33137476969 reached **COMPLETED SUCCESS** (live-e2e + Build+unit
  on the `-j2` cap). The vault hedge is green.
- **OSM pushed 2026-08-28 05:14Z** — agent run 33141449968 went GREEN
  (completed success on `4e6e3c5`, the cargo-churn fix), so the hold lifted.
  OSM run **33144124163** on `012a5cb` is in flight (CU cycle profile started
  on `logos-box-osm`; 4 jobs total). Verify it goes fully green.
- **NEW root cause discovered 04:10Z (affects all three repos):** the three
  self-hosted runners share ONE home dir (`/home/ubuntu`). Host-level
  toolchain installs — `dtolnay/rust-toolchain` and `rzup install rust` (vault
  ci.yml lines 99-124, osm ci.yml lines 112-137, both on the live-e2e jobs) —
  churn the rustup shims in `/home/ubuntu/.cargo/bin`. Run concurrently with a
  job mid-`cargo`, the shims break (observed twice: agent e2e "cargo: command
  not found" at different steps). It even removed the `rustup` binary itself;
  I reinstalled it via `sh.rustup.rs -y --no-modify-path` + `rustup default
  1.94.0` (04:18Z, cargo 1.94.0 + rustc 1.94.0 verified). 
- **Durable fix pattern** (applied to logos-agent e2e job, commit `4e6e3c5`):
  prepend the real toolchain bin to PATH in the job
  (`echo "$HOME/.rustup/toolchains/1.94.0-x86_64-unknown-linux-gnu/bin" >>
  "$GITHUB_PATH"`) so cargo/rustc resolve to a real binary that rustup churn
  does not touch. **Recommended for vault/osm live-e2e jobs too** (they run
  rzup on the host): either the same PATH pin to their toolchain, or isolate
  with per-runner `RUSTUP_HOME`/`CARGO_HOME`. Not yet done — flagged for the
  next agent that touches those workflows.
