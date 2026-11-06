# logos-osm — build state (LP-0018 / PR #71: OpenStreetMap integration)

> Working doc for whoever picks this up (agent or human). Update per milestone.
> **Last updated: 2026-11-06** — shipped as v1.3.10: hover affordances + human status lines, CLI `register --sign`, wallet create/fund examples, post-announcement TODO roadmap.
> The section below is current; everything after the historical marker
> describes the pre-port state and is kept for context only, including rules
> that no longer apply.

## 🤝 FOLLOW-UP (2026-11-06) — hover "shine light" affordances + human status lines

Two UX gaps, one design: **hover explains the control, the status line
explains the outcome.**

**1. Hover explanations everywhere.** `SButton`/`PButton` gained a `tip`
string property → ToolTip on `hovered` (300 ms delay, suppressed when
empty). Every button in the app now sets it — the three Fetch actions,
re-check registry (both), List regions (both), the four Consumer pickers,
Restore + import, use cached file, Import (validate + catalog), Verify +
store + register tx, Check for newer snapshot, Local catalog. The 7
`FieldCell`s all carry `info:` (ⓟ glyph → tooltip): storage/geofabrik/cache
(registrar already had one), CID, published MD5, PBF FILE. The `info` hook
existed since v1.3.5 but only 1 of 7 fields used it.

**2. Deterministic results → fixed human text.** The Consumer ops return
small fixed shapes, so `taskSummary()` now pairs each to a sentence
instead of "ok": fetch → `region · restored ✓ · 51.4 MB · md5 matches
on-chain ✓`; import → `imported ✓ · … · md5 …` or (workflow
"verified+stored") `verified + stored ✓ · cid … · tx ready → sign with
wallet`; update → `up to date — your file matches Geofabrik's latest
(v20261106)` / `update available — v20261106 → v20261120` / `not in your
catalog yet — latest published v…`. Successful fetch/import/update no
longer raw-dump JSON to the output console (the status line says it
better); failures still dump in full.

**Verified:** `qmlformat` parses clean, `qmllint` 0 errors (only pre-
existing layout warnings), app reloaded on :99 with no QML errors,
storage node API healthy. Balance 547/547 braces, 802/802 parens.
Visual hover verification pending in VNC (user hovers).

**Reapplied after an accidental `git checkout` wipe** (the working tree
was reset to 85403a2 mid-task; all edits were re-applied from history and
balance-checked). The wallet-link Text+MouseArea fix was in that wipe too
and has been **reapplied** (Text `walletLink` + `walletMA` MouseArea +
`cInfoHi` color token) — the RichText `<a>` form is gone again.

## 🤝 STATE AT HANDOFF (2026-11-05, post-1.3.4) — long ops report back

The job work landed in the working tree (1.3.4's release shipped WITHOUT
it) had one live bug that defeated the whole feature: the result channel
silently dropped the running→done transition, so a host completed
server-side (bytes stored, catalog updated, core idle) while the task
stayed "running" for 20+ minutes and every op queued behind it starved.

### Root cause (proven live via a wiretap of the app↔core socket)

The QML→core transport delivers results as the `lastResult` **property**,
and the generated `OsmAppSimpleSource::setLastResult` short-circuits on
`QtPrivate::equalStrings` — a byte-identical value emits **no**
`lastResultChanged`, so the QML `onLastResultChanged` handler never runs.
While a job is `running`, `job_status` returned the identical JSON every
poll, so after the 2nd poll the reply stopped reaching `onJobStatus`,
`jobStatusInFlight` stuck `true`, and `pollJobs()` early-returned on every
tick forever. The job finished at +6 s (catalog written) but the UI had
already stopped asking — 2 polls total, then dead. Separately,
`OsmAppPlugin::invokeCore` passes a 20 000 ms `Timeout` to
`invokeRemoteMethod`, so any *sync* op whose reply takes >20 s is dropped
outright (the original "task runs forever" symptom); the job path sidesteps
this by returning a job id instantly, but it inherited the property-dedup
deadlock on its polls.

### The fix (Rust + QML)

- **`osm-sdk/src/ffi.rs` — `op_job_status` now embeds `elapsed` (seconds
  since `started_at`) in every reply.** While a job runs the reply now
  differs poll-to-poll, so the property sync always emits and the poll
  loop never stalls. The done/error payloads already differed (they carry
  the full result), so only the running phase needed this. (+ clippy
  `clone_on_copy` on `registrar` in the host job closure — `Option<AccountId>`
  is `Copy`.)
- **`app/src/qml/Main.qml` — `pollJobs()` re-arms on a deadline, not on the
  result arriving.** A poll is "in flight" until its result lands OR 3 ticks
  pass (a generous drop timeout — `job_status` is sub-millisecond
  server-side); either way the next tick fires again. So a dropped/deduped
  frame can no longer permanently stall the loop. `clearTasks()` resets the
  deadline too. This is defense in depth on top of the `elapsed` fix; either
  alone would do, both together tolerate any transport hiccup.
- **`app/src/qml/Main.qml` — the region picker popup now opens downward
  when there's no room above it, with its height clamped to the window.**
  The popup is 400 px tall and hung at `y: -(height+8)` above its button;
  on a 1280×800 screen the Host §02 button sits ~315 px down, so the popup's
  filter field (and first rows) were clipped above the screen top — the
  filter was unreachable and typing never landed. `place()` measures the
  button's absolute position at `onOpened` and flips below / shrinks as
  needed. All four pickers (Host + the three Consumer pickers) wire
  `anchorItem` to their button.

### Live verification (real Basecamp, VNC :99, wiretap on the socket)

Hosted `china/jiangsu` (80 MB) end-to-end, three times, against the
storage module's live libp2p node:

- **Host → done with summary.** Polls fired every second (67 of them),
  `elapsed` 0→35, state flipped `running`→`done` at +28–35 s with the full
  result (`bytes`, `cid`). The task row resolved to
  `china/jiangsu · zDvZRwzkzF… · 80 MB`. (Before the fix: 2 polls, then
  dead; task "running" 21+ min.)
- **Starvation test.** Fired `lookup_bulk` (72 regions) *during* a running
  host: it delivered (`count:72, registered_count:25`) while `job_status`
  polls kept flowing concurrently — the dispatcher is free because the host
  runs as a job. (Before: the host blocked the dispatcher and the queued
  lookup stuck — the user's "registry check stuck on the PC while a host
  ran".)
- **Clear mid-host.** Clicked Clear while a host polled: the task list
  emptied (chip → `idle`), job polling stopped (0 `job_status` frames
  afterward), and the host still completed server-side (catalog updated
  at +12 s) — its bytes land regardless; Clear only stops the UI tracking.

`cargo test -p logos-osm --lib`: 42 passed, 1 ignored. `cargo clippy`:
0 warnings. Shipped as **v1.3.5** (commit 3c4851c; all three metadata
files bumped; the windows vendor DLL rebuilt to match. The linux vendor
`liblogos_osm.so` in `module/vendor/x86_64-linux/` still needs the same
rebuild+commit when someone next builds on linux).

---

## 🤝 FOLLOW-UP (2026-11-05, post-1.3.5) — module-crash hang + hardening

A user-hosted `czech-republic` (951 MB) sat "running" 5+ minutes after
1.3.5. Investigation: **the osm module host process had segfaulted**
(signal 11 at 18:03, per basecamp.log) — the wire shows the host dispatch
forwarded, **no ack ever returned**, then `module_state_changed: osm →
error / module exited without being asked to`. The job/poll code was
fine; the process doing the work was dead.

- **Not reproducible.** Three attempts: (1) re-ran the exact sequence
  in-app — jiangsu host running at elapsed ~15, czech fired on top, core
  dumps armed — no crash; czech went running→done in 77 s (951,801,724
  bytes, `cid zDvZRwzmC75…`, catalog 28→29). (2)(3) ctypes with the same
  `.so`, single and two concurrent hosts (jiangsu + czech overlapping) —
  no crash. The segfault correlates with a degraded-libp2p window in the
  storage module right before it (`Stream Closed!`, `Missed multiple
  heartbeats`, `No key for a QueryResponse`); likely a transient
  native-level fault, not a deterministic bug in the job paths. Note
  `catch_unwind` cannot help against SIGSEGV.
- **The real gap it exposed:** a dead module answers every `invokeOp`
  with `""` (the RemoteObjects call simply never returns), so
  `lastResult` stays empty, `onJobStatus` never runs, and the row would
  poll a corpse forever — exactly the phantom "running" task the user saw.
- **Hardening (QML only, on top of 1.3.5):** consecutive-miss detection
  in the existing poll loop. Each deadline re-arm with no valid
  `job_status` reply bumps `jobStatusMisses`; at 5 (~15 s of silence)
  `failRunningJobs()` flips every still-running task to
  `error: "module not responding — it may have crashed; restart
  Basecamp"`, halts the host loop, and stops polling. A valid reply
  resets the streak; `clearTasks()` resets the counter.
- **Verified live:** fired a jiangsu host, `kill -9`'d the module host
  mid-job — within ~15–20 s every running row flipped to the error
  message and `job_status` polling stopped (rows stayed; "Clear | N
  tracked" in the panel header).

---

## 🤝 FOLLOW-UP (2026-11-05) — live byte progress + one-job-at-a-time

Two more UX gaps closed on top of the v1.3.5 + crash-hardening work:

### 1. Live progress during a host/fetch

A running host showed only an elapsed timer — no indication how far the
download/upload had gotten. Now the task row ticks
`downloading 45 / 80 MB · 56%` → `storing 72 / 80 MB · 90%` → final summary.

- **`osm-sdk/src/progress.rs` (new):** `JobProgress`, an `Arc` over lock-free
  atomics (`phase: AtomicU8`, `done`/`total: AtomicU64`). `Phase` is
  `Starting | Downloading | Storing`; a free `phase_label(u8)` maps the raw
  byte back to a label (so the JSON carries a stable int + a friendly string).
- **Streaming clients carry an optional progress cell** (absent by default —
  CLI/tests unchanged): `GeofabrikClient::with_progress` ticks `done` per
  chunk during the PBF download; `CodexStorage::with_progress` overrides the
  trait's no-op default and ticks `done` as the `ReaderStream` feeds reqwest's
  upload body (the slow phase for GB-scale files). The fetch path
  (`get_to_file`) reports download progress from storage the same way.
  `OsmClient::with_progress` fans the one cell out to both the Geofabrik and
  storage sides; `AnyStorage`/`MemoryStorage` keep the trait's no-op default
  (memory buffers, nothing to report).
- **FFI:** `op_host`/`op_fetch` job branches create the cell, pass a clone
  into the closure (`client(&st).with_progress(p)`), and store a clone on the
  `JobState`. `op_job_status` snapshots it into the running reply as
  `progress: {phase, phase_label, done, total}` (omitted when the job isn't
  streaming or is done — the final result already carries bytes/cid).
- **QML:** `onJobStatus`, on a `running` reply with `progress`, updates the
  row's summary via `jobProgressText(p)` (`"{phase} {done} / {total} · N%"`,
  `total` shows `—` when the transfer had no `Content-Length`). The state
  stays `running` so the elapsed timer keeps going; the summary is replaced
  by the final `taskSummary` on completion.

### 2. One long job at a time (no duplicate concurrent hosts)

A second host fired while one ran (double-click racing the button's
`enabled` binding, or a stray click) started a second download against the
same Geofabrik/storage node — wasteful, and the concurrent-job window is
where the one-off module segfault earlier landed. Now defended at two layers:

- **QML:** a `longJobBusy` binding (`Object.keys(jobPolls).length > 0`)
  disables the three Host buttons **and** the Fetch button while any host or
  fetch job is running (on top of the existing `hostLoopActive` loop guard).
  A click on a disabled button is a no-op.
- **Rust (last line of defense):** `op_host`/`op_fetch` job mode call
  `running_long_job()`; if any host **or** fetch job is already `running`,
  the call is refused with `{ok:false, error:"a long job is already running
  (job-N); wait for it to finish"}`. The host loop never trips this —
  `hostNext` fires the next region only after the previous job_status says
  `done`, by which point the registry has that job `done` (not `running`).

### Verification (real Basecamp, VNC :99)

- **Progress:** hosted `china/jiangsu` (80 MB); `job_status` polls showed
  `downloading 3.8→58.9 MB / 80 MB` ticking every second, then `storing
  72.3 MB / 80 MB · 90%`, then `done` at elapsed ~16 s with the full
  `bytes`/`cid`. The task row rendered `downloading … MB / 80 MB · N%` live
  and resolved to `jiangsu · zDvZRwzkzF… · 80 MB`.
- **Dedupe:** fired a host, then rapid duplicate clicks on "Host selected"
  while it ran — no second job created (button disabled). Standalone ctypes
  repro of a concurrent second host: refused with `"a long job is already
  running (job-0); wait for it to finish"`. (Earlier "duplicate" runs that
  appeared to create multiple jobs were an artifact of the OCR-driven click
  tool's latency — every second click landed after the previous host had
  finished, i.e. sequential re-fires, not concurrency.)

`cargo test -p logos-osm --lib`: 42 passed, 1 ignored. `cargo clippy`: 0
warnings. New file `osm-sdk/src/progress.rs`; edits in `geofabrik.rs`,
`storage.rs`, `osm.rs`, `ffi.rs`, `lib.rs`, `app/src/qml/Main.qml`.

---

## 🤝 FOLLOW-UP (2026-11-05) — Host→Fetch reframe + on-chain bridge

### 1. Naming: "Host" → "Fetch", Consumer "Fetch" → "Restore"

The "Host" operation never put a region on the chain — it downloads from
Geofabrik and stores bytes on your device + your storage node. On-chain
availability requires a wallet-signed registration transaction (CLI wallet
only). So the UI now names the operation for what it does:

- Tab `Host / Registrar` → `Fetch / Registrar`; §02 head
  `Host (download → verify MD5 → Logos Storage)` →
  `Fetch (Geofabrik → verify MD5 → your storage node)`.
- Buttons `Host selected / Host all except selected / Host all` →
  `Fetch selected / Fetch all except selected / Fetch all`.
- Consumer §01 `Fetch a registered snapshot…` → `Restore a registered snapshot…`;
  button `Fetch + import` → `Restore + import`.
- Task labels: `host china/jiangsu` → `fetch china/jiangsu`; the loop's
  group task `host N regions` → `fetch N regions`; progress
  `hosting 1/3` → `fetching 1/3`; done `N hosted` → `N fetched`.
- Guide docs retitled: "Hosting regions" → "Fetching regions to your node"
  (reframed: "this is not yet on Logos"); "Fetching maps" →
  "Restoring registered snapshots"; register-onchain doc's "Host regions"
  step → "Fetch regions".
- `metadata.json` description reworded accordingly.

**Op names unchanged** (`host`, `fetch`, `register`, `host-run`): the CLI,
`.rep`, FFI, and state file are untouched — this is UI labels + docs only.
Zero behavior change.

### 2. Status clarity: on device vs registered

After a fetch resolves, the per-region task row joins the local facts with
the on-chain status: `china/jiangsu · zDvZRwzkzF… · 80 MB · not registered`
(or `registered ✓` when the registry confirms). The Regions tab badges
(already driven by `lookup_bulk`) stay the single source of truth for
"is it on Logos"; the row just surfaces both facts so the gap is visible.

A fetch run now also auto-rechecks the on-chain registry for the regions it
fetched (one batched `lookup_bulk` after the catalog refresh), and the
group task row finalizes to `1 fetched · 80 MB · 0/1 registered — finish
with the CLI wallet` (or `…/K registered ✓`).

### 3. CLI wallet bridge: don't assume a wallet exists

A "Finish on-chain" card appears under the Fetch buttons whenever a fetch
run left regions unregistered. It states plainly that the chain learns
about a region only via a wallet-signed registration tx, then offers, per
region, a **selectable copy-paste command**:
`osm register china/jiangsu --registrar <64-hex>` (the registrar hex
auto-fills from the Connect field when present; `<64-hex>` placeholder
otherwise — the app builds the tx, the wallet signs; the app never holds
keys). It links out to the official Logos wallet install guide:
`https://docs.logos.co/lez/get-started/run-lez-wallet-via-cli` (CLI) and
`…/run-lez-wallet-app-initiate-native-token-transfers` (Basecamp app), and
has a **re-check registry** button that fires `lookup_bulk` for the pending
regions (read-only, no keys) so the chips refresh after the user runs the
CLI — closing the loop without the app ever holding a key.

### Verification (real Basecamp, VNC — confirmed by user)

Rename rendered live (`Fetch / Registrar` tab, `Fetch (Geofabrik → verify
MD5 → your storage node)` head, the three Fetch buttons, Consumer
`Restore…`). A `china/jiangsu` fetch (an unregistered region) produced the
task row `…· 80 MB · not registered`, the group row
`1 fetched · 80 MB · 0/1 registered — finish with the CLI wallet`, and the
Finish-on-chain card with `osm register china/jiangsu --registrar <64-hex>`
+ the wallet link + re-check button.

No Rust touched this round (labels/docs/UI only); `cargo`/`clippy` baseline
unchanged (42/1/0). Edits: `app/src/qml/Main.qml`,
`app/src/qml/guide/GuideDocs.js`, `app/metadata.json`. QML braces 520/520,
parens 700/700.

### Aside: web_search in this harness

`web_search` is unavailable here — it needs a `DEEPSEEK_API_KEY` (or a
DeepSeek account sign-in), and this session routes chat through the
`surplus-claude` gateway (`glm-5.2`), so neither credential exists. The
surplus gateway accepts the `web_search_20250305` tool but runs it
client-side (returns `tool_use`, no `web_search_tool_result` blocks), so it
cannot substitute. **`web_fetch` works keyless** and was used to find the
Logos docs (`docs.logos.co/lez`). Fix for `web_search`: add a real
DeepSeek API key to `~/.dsh/.env` as `DEEPSEEK_API_KEY` (or the GUI Web
Models page) — deferred, not blocking the OSM work.

---

## 🤝 FOLLOW-UP (2026-11-06) — storing-error readability + local demo guide

### The "storing" failure (no storage node) made readable

On a PC running only Basecamp (no Logos Storage node), a fetch downloads
fine (file in the cache dir) then fails at the store step — PUT to
`storage_url` (default `http://127.0.0.1:8080`) gets connection-refused.
The old error was an opaque one-liner that the task panel truncated
(`host china/henan: storing china/henan in Logos Storage: POST
/api/storage/v1/data: error sending request for url (…)`), unreadable
because the panel scrolls vertically only.

Two fixes:

- **Rust** (`osm-sdk/src/storage.rs`, `put_file`): a connection failure
  (`reqwest` `is_connect()`) now surfaces as a clear, actionable message
  instead of the raw "error sending request for url":
  `Logos Storage node at <url> did not respond — is a storage node running
  there? (the file downloaded fine; this step uploads it to the storage
  node)`. Timeouts and other send errors keep their original wording.
- **QML** (`Main.qml`, task panel row): the summary `Text` now **wraps**
  (up to 3 lines) instead of eliding off the right edge, and the row
  height grows to fit; the full text is also in the row's tooltip. So a
  long error reads in full without horizontal scrolling.

Reproduced and verified via ctypes against a dead storage port: the job
errors with the new message above. `cargo test -p logos-osm --lib`:
42 passed, 1 ignored; `cargo clippy`: 0 warnings.

### Local demo setup guide

New `docs/LOCAL_DEMO_SETUP.md` — how to run the full fetch → store →
register → verify round-trip on a personal PC for demo (the heavy 60 GB
region set stays on the codespace). Covers: the Logos Storage node (runs
as an app inside Basecamp, no separate binary), the LEZ wallet CLI
(`cargo install --path lez/wallet` from the LEZ repo, `wallet
change-network testnet`), the `osm` CLI (`cargo build -p logos-osm-cli`
from this repo), and a disk estimate (≈ 3–4 GB for a one-region demo).
Links the official Logos docs throughout.

---

## 🤝 FOLLOW-UP (2026-11-06) — explicit edit/save for every persistent field

### The "does it actually save?" question

The Connect fields (Storage URL, Geofabrik URL, Cache Dir, Registrar) all
feed the `open` op, which persists them to the state file. But two gaps
made that invisible (and one was a real bug):

1. **No affordance** — the fields were always-editable TextFields with no
   save button; editing one did nothing until an app restart, and there was
   no confirmation it took. The user couldn't tell a change persisted.
2. **Boot-wipe** — `openArgs()` always passed the fields' *placeholder
   default* text at startup, so `open` loaded the saved state and then
   *overwrote* it with the defaults. A customized Storage URL or Geofabrik
   URL was silently lost on the next restart. (Cache Dir escaped this only
   because it was guarded to skip the `osm-cache` placeholder.)
3. **Registrar never persisted** — `AppState` had no `registrar_hex` field,
   so the account id you pasted was gone after a restart.

### The fix

**Rust** (`osm-sdk/src/ffi.rs`):
- `AppState` gains `registrar_hex: String` (`#[serde(default,
  skip_serializing_if = "String::is_empty")]` — old state files still load).
- `op_open` validates + persists `registrar_hex` when given (a malformed id
  is rejected with `registrar_hex: …`, same as `program_account_hex`), and
  the open summary now returns `registrar_hex` so the app can re-seed the
  field.
- The summary already returned `storage_url`/`geofabrik_url`/`cache_dir`;
  the app just wasn't reading them.

**QML** (`app/src/qml/Main.qml`):
- `openArgs(withConfig)`: the **boot** call passes `false` — no user config,
  so the saved state loads untouched; a **save** passes `true` with the
  edited values, guarded so empty strings never overwrite a saved value
  (and the cache-dir placeholder `osm-cache` still pins nothing).
- `onOpResult("open")` now **seeds all four fields** from the result — the
  persistence round-trip is visible: whatever `open` loaded (and, for a
  save, just wrote) is displayed. A saved edit comes back; at boot the saved
  values fill the fields.
- New `PersistField` component: each persistent field is a **read-only
  value display** with a ✎ button at its right; clicking ✎ makes it
  editable and shows ✓ (save) + ✕ (revert). ✓ re-fires `open` (one reconnect
  persists the whole form), the result seeds the fields back, and a
  `✓ saved` flash appears beside each. MonoField gained a flatter
  `readOnly` style; `GlyphButton` is a plain Item+Rectangle+MouseArea (not
  a Control) so its 26×26 box lays out and clicks reliably. Applied to all
  four Connect fields uniformly.
- A user-save keeps the Connect section open (so the `✓ saved` flash is
  visible); the auto-open still folds it away.

### Verification (real Basecamp, VNC — confirmed by user)

✎ renders beside all four fields; clicking ✎ → editable + ✓/✕; clicking ✓
reconnects (storage dot blinks) + `✓ saved` flash (user-confirmed). Boot-
seeding verified: after a restart, Cache Dir shows the saved absolute path
(`/home/codespace/osm-anchor/cache`, not `osm-cache`) and Registrar shows
the saved hex — proving the boot-wipe is fixed and persistence is real.
`cargo test -p logos-osm --lib`: 42 passed, 1 ignored; `cargo clippy`: 0
warnings. QML braces 543/543, parens 766/766.

### Wallet-install link made actually clickable

The "How to install the LEZ wallet" link in the Finish-on-chain card was a
RichText `<a>` whose hit-box wasn't registering clicks. Replaced it with a
styled `Text` + `MouseArea` (underlined info-blue, hand cursor, the whole
label is the hit target) that calls `Qt.openUrlExternally` to the LEZ
wallet CLI docs. New `cInfoHi` color for the link hover state. User-confirmed
it now opens the browser.

---

## 🤝 FOLLOW-UP (2026-11-06) — `osm register --sign`: build + sign + submit in one step

The CLI's `register` / `register-bulk` previously only **built** the
registration tx and printed it as JSON, leaving the user to copy-paste that
JSON into the wallet CLI to sign + submit — a clunky two-tool handoff.
Since it's all command-line tooling on the user's own machine, the wallet's
key store is right there to open.

**Default is still build-only** (submitting to the chain is a delicate,
irreversible action — the safe path stays the default). A new `--sign` flag
opts into the full flow:

```sh
osm register china/henan --registrar <64-hex>        # build only (default): prints tx JSON
osm register china/henan --registrar <64-hex> --sign # build + open wallet + sign + submit + await
```

`--sign` opens the LEZ wallet at `~/.logos-osm/wallet/` (the one
`create_wallet` writes — override with `--wallet-dir` / `OSM_WALLET_DIR`),
calls `WalletCore::new_update_chain` (loads the existing key store via
`Storage::from_path`, same as the wallet CLI itself — the CLI never sees
key material beyond opening that store), `send_pub_tx` + `poll_transaction`
+ `sync_to_latest_block`, with a transient-retry loop for the testnet's
502/timeout/dropped-connection flakiness. No wallet present → a clear
error pointing at `create_wallet`. The same flag applies to `register-bulk`.

The "app never holds keys" rule was about the **Basecamp GUI app** (shared,
persistent, you don't want keys there) — not the personal CLI, which reads
the wallet store exactly as the wallet CLI does.

`cli/src/main.rs`: new `submit_register` helper + inline `retry!` macro
(closures can't return borrowed `wallet` futures, so the retry is a macro
matching the integration-test pattern); `Register`/`RegisterBulk` gain
`--sign`/`--wallet-dir`/`--wallet-password`. `cargo build -p logos-osm-cli`:
clean; `cargo clippy -p logos-osm-cli`: 0; SDK tests unchanged (42/1).
Verified: build-only default prints tx JSON (unchanged); `--sign` with no
wallet errors cleanly; `--help` documents the flags. `docs/LOCAL_DEMO_SETUP.md`
updated to the one-step `--sign` flow.

---

## 🤝 STATE AT HANDOFF (2026-11-04, v1.3.4) — UX overhaul

Five changes, one goal: the app explains itself and the registry answers
in bulk. 1894 → 2496 lines of QML, one new core op.

1. **The LEZ transactions section (Host §03) is gone from the app.**
   Registration/init tx builders moved out of the UI — the wallet+CLI
   flow owns them now (the new Guide tab's "Registering regions
   on-chain" doc walks the CLI). QML-only: the `register` /
   `register_bulk` / `init` FFI ops are untouched (the CLI depends on
   them). The Host tab is Connect + Host + console; `txSelectedRegions`,
   `toggleTxRegion`, the §03 picker, and the OWNER field all deleted
   with no dangling references (identifier-checked). The REGISTRAR
   field stays in Connect — Consumer §02's "Verify + store + register
   tx" still reads it.
2. **Connect-section UX.** Every `SectionHead` is now a toggle: a
   `Section` component wraps header + body, chevron rotates ▸→▾, body
   height animates 180 ms. Connect auto-collapses when the startup
   auto-open succeeds (a failed startup open leaves it expanded —
   verified with a fresh load in the headless harness); manual toggles
   are plain property state. Fields got captions (visible while
   empty/default), the REGISTRAR got live validation (64-hex → green
   border + "✓ 64 hex characters"; wrong → amber + "needs 64 hex
   characters") via a new `MonoField.validation` property, an ⓘ tooltip
   (what a registrar is, why the app never sees keys), and the STORAGE
   label got a reachability dot driven by every open result (green/red).
   The CORE chip renders `✓ v1.3.4` (parsed from the version JSON) with
   the raw JSON in its tooltip.
3. **Regions tab: registered-first with badges (new core op
   `lookup_bulk`).** Modeled on `sizes`: optional comma `regions`
   subset ("all"/absent = the frozen 72), 8 reads in flight
   (`registry::resolve_regions_bulk`, the jsonrpsee client cloned per
   job — it is cheap to clone), per-region errors recorded in an
   `error` field instead of failing the op, results
   `{statuses, count, registered_count, error_count}` tagged with
   `"op": "lookup_bulk"` by `dispatch`. Statuses MERGE into a new
   `AppState.region_statuses` (BTreeMap<region, Value>, serde default +
   skip-if-empty) exactly like the sizes cache, and `open` returns them
   so the tab renders from the state file. QML: opening the tab
   auto-fires `lookup_bulk` when the session has no statuses yet;
   "Re-check" re-fires; rows show `◆ registered · v<latest>` in the
   accent (muted `—` otherwise, "read failed" + tooltip with the error
   when the chain read failed); registered sort first (stable by path
   within groups); an All/Registered/Not registered filter row
   re-derives the model. Click-a-row → full `lookup` → console is
   unchanged. 2 new Rust tests (registry bulk shape + FFI
   shape/cache/merge; 42 pass now).
4. **Consumer tab: per-section pickers + auto-population.** Each of the
   three sections got its OWN single-select picker — one shared
   `RegionPickPopup` component (search + sizes list, the same popup the
   Host picker uses, with a `singleSelect` mode that closes on pick);
   the free-text REGION fields are gone; the pinned chip shows
   `region · size` with × to clear. Picking a fetch region fires
   `lookup`, routed in `onOpResult` and correlated by region (the core
   tags every result `"lookup"`, so a pending flag + region match
   guards against consuming a Regions-tab row-click result — verified:
   a mismatched result is ignored): ok fills CID + MD5 with a "from
   registry ✓" caption; "is not registered on chain" → "not on the
   registry yet — nothing to fetch"; any other error → amber "registry
   unreachable — enter manually" (fields stay editable). §02 import got
   "use cached file" (fills the PBF path from the local catalog
   record's `path` — `catalogRecords` now stores the full records; a
   finished host run refires `catalog` so fresh regions appear). MD5
   validates live (32 hex), CID gets a presence check.
5. **Guide tab (4th tab): bundled markdown, rendered in-app.** Doc list
   (220 px, left) + `Text { textFormat: Text.MarkdownText }` in a
   ScrollView. **Rendering path: STRING CONSTANTS, not file reads.**
   Two independent blockers, both verified: (a) Qt 6.10 QML
   `XMLHttpRequest` refuses file: URLs by default ("Using GET on a
   local file is disabled by default. Set QML_XHR_ALLOW_FILE_READ to
   1" — reproduced locally with PySide6 6.10.2), and (b) Basecamp's
   sandbox (logos-app `app/restricted/DenyAllNetworkAccessManager.cpp`,
   wired per ui_qml view in `QmlSandbox.cpp`) returns a failing reply
   for EVERY engine network request, file: included — so even with the
   env var set, the packaged app cannot XHR a sibling .md. The docs
   therefore live as markdown template-literals in
   `app/src/qml/guide/GuideDocs.js` (a `.pragma library` inside the QML
   dir, loaded through the type loader — the same channel that loads
   fonts, which the sandbox allows for local files under the module's
   qml root). Four docs: how-it-works (roles, trust triangle, what a
   registrar is, why the wallet signs), hosting (Host tab walkthrough),
   fetching (Consumer tab walkthrough), register-onchain (CLI:
   prerequisites, the published-genesis-account faucet, deploy,
   `osm init`, hosting, `osm register`/`register-bulk` ≤ 24 per tx,
   `osm lookup`).

Verification for this release (beyond cargo): the whole QML was loaded
headlessly (PySide6, offscreen) against a mock `osm_app` replica and
driven through the full startup chain, a `lookup_bulk` merge, all three
fetch-fill outcomes, a mis-correlated lookup, all four guide docs, both
open outcomes, and the region model's registered-first sort and
All/Registered/Not filter counts. qmllint shows no new warning
categories vs the pristine file (the two `missing-property` hits are
the pre-existing ConsoleScrollBar `parent.pressed/hovered` pattern;
`unqualified` grows with the file, which is that file's style).

Long ops used to be invisible: a result only landed in the bottom output
console, and `host_bulk` gave no feedback until every region finished. Four
changes, one goal — you can now see what the app is doing:

1. **Ongoing-tasks panel.** Every `root.call()` registers a task
   (`{op, label, startedAt, state, summary}`); `onOpResult` resolves the
   OLDEST running task matching the routed op (r.op first, pendingOp tag
   normalized back to the core name for pre-tagging cores). A chip below
   the header's CORE/REGIONS row ("TASKS ◐ 1 running · 2 done") opens a
   popup listing them — glyph, label, ticking mm:ss elapsed for running
   rows (ONE shared 1 Hz Timer bumps `taskTick`, which the elapsed
   bindings depend on), and a one-line summary (host: region + first 10
   CID chars + bytes; sizes: "N regions · X GB"; fetch: region + CID from
   the call args; others "ok"; errors keep the message, elided). Last ~8
   completed tasks are kept, oldest pruned.
2. **Real host progress — client-side, no core change.** The three Host
   §02 buttons compute their region list exactly as their labels do
   (selected / all-except-selected / all-except-hosted) and fire the
   single-region `host` op ONE AT A TIME through a small driver
   (`startHostRun` / `hostNext` / `onHostResult`): each result ticks a
   grouped task ("hosting 3/10 · bulgaria ✓ · 674 MB so far") and fires
   the next; a region error stops the loop and marks the group error with
   region + message. Strictly sequential like the startup chain — the
   RemoteObjects transport drops ops fired while another is in flight
   (which is also why the buttons disable while a run is active). The
   server-side `host_bulk` FFI op is untouched (the CLI uses it); the
   loop IS the progress channel — the C++ glue only surfaces a result
   when the op returns, so mid-op emissions can never reach QML.
3. **LEZ transactions picker.** Section 03's free-text REGION field is
   replaced by the same selection method as §02: own selection state
   (`txSelectedRegions` + generalized `toggleIn`), ×-chips, "Select
   regions ▾" button + popup. "Register tx" passes the single selected
   region (disabled unless exactly one); "Batch tx" passes all selected
   joined by commas; `registrar_hex` wiring unchanged.
4. **Sizes cache MERGE fix (Rust, op_sizes).** The `sizes` op used to
   REPLACE `AppState.region_sizes` with whatever the fetch returned — a
   subset call (`regions: "denmark"`) shrank the cache to one entry and
   the picker lost every other region's size on the next `open`. It now
   MERGES: only the fetched entries update, the rest survive, with the
   "only save if anything changed" guard kept (insert returns the old
   value; changed iff absent or different). Verified via ctypes against
   the real state file: open → sizes{denmark} → open keeps both the
   pre-existing entries and denmark; a fresh-region fetch (germany)
   writes the new entry while the old ones survive.

## 🤝 STATE AT HANDOFF (2026-11-04, v1.3.2) — sizes load faster

Three changes, one goal: the region picker's sizes show as early as
possible on startup.

1. **Sizes are cached in the state file.** The `sizes` op persists its
   result into `AppState.region_sizes` (`osm-sdk/src/ffi.rs`, a
   `BTreeMap<String, u64>` with `serde(default)` so old state files load
   fine) and `open` returns it in its result summary. The QML seeds the
   picker from that cache the moment `open` completes, so after the
   first-ever fetch the sizes are instant on every startup; the live
   `sizes` refresh then updates them in the background.
2. **The startup chain reordered: sizes before catalog.** The chain is
   open (seeds cached sizes) → discover → regions → sizes → catalog —
   sizes no longer waits behind the slow LEZ catalog RPC (5–15 s on
   testnet). The `sizes` arm fires catalog unconditionally (ok or
   error), so a failed sizes op never strands the chain; a sizes op
   dropped in transport is still recovered by the REGIONS chip, same as
   v1.3.1. Serialization (each op fired from the previous result) is
   unchanged — the RemoteObjects transport drops ops fired while
   another is in flight.
3. **24-way HEAD concurrency.** `geofabrik.sizes()` runs its HEAD
   requests `.buffer_unordered(24)` (was 8), so the live refresh of all
   72 regions lands proportionally sooner.

## 🤝 STATE AT HANDOFF (2026-11-04, v1.3.1) — QML routing fixes, verified live

v1.3.0 shipped with the picker QML broken in three ways, all found by
driving the real v1.3.0 .lgx inside Basecamp (Install Local Package →
search → launch):

1. **Op-name mismatch:** the core tags results with ITS op names
   (`"op": "regions"`, `"op": "catalog"`), but the QML arms matched the
   local dispatch tags (`"regions-list"`, `"catalog-list"`). With r.op
   authoritative, those arms never ran — the picker list and the
   hosted-set stayed empty. Arms now accept both names.
2. **Catalog result shape:** the `catalog` op returns the records as a
   BARE LIST under `result`, not `{catalog: [...]}` — the arm read
   `r.result.catalog` (undefined on an array) and always took the
   empty fallback. Handles both shapes now.
3. **Startup sizes fetch dropped:** firing `sizes` from the discover
   result while the slow catalog RPC was still in flight silently
   dropped the op in the RemoteObjects transport (no result ever
   arrived; core idle). The startup chain is now serialized — open →
   discover → regions → catalog → sizes, each fired from the previous
   result — and sizes fires with a drained queue. Clicking the REGIONS
   chip re-runs the whole chain (verified as the manual recovery).
   Plus a null guard on `ListView.view.count` in the Regions tab
   delegate (a TypeError every frame otherwise).

Live verification (Basecamp, both packages at 1.3.0/1.3.1, real
catalog with 25 hosted regions): REGIONS 72/72; sizes auto-load (~7s,
60.4 GB total); picker lists all regions with per-region sizes;
selecting germany+france pins ×-chips and shows "Host selected
(2 · 10.0 GB)", "Host all except selected (70 · 50.4 GB)", "Host all
(47 new · 51.7 GB)" — every number checks out by arithmetic.

## 🤝 STATE AT HANDOFF (2026-11-04, v1.3.0) — region picker + op-tagged results

Two changes ship in 1.3.0:

1. **`sizes` op + the region picker UI.** `geofabrik.sizes()` HEADs each
   region PBF (8-way concurrent, follows the `-latest`→dated redirect) and
   the FFI exposes it as `sizes` (optional comma `regions` subset or all).
   The app's Host §02 became a picker: "Select regions ▾" opens a popup
   with search + per-region sizes; selected regions pin as ×-chips; three
   actions — host selected, host all except selected, and host-all-except-
   what-you-have (opt_out from the catalog) — each with cumulative size
   estimates.
2. **Results are op-tagged.** The dynamic-replica result channel is ONE
   `lastResult` property: with several ops in flight (the startup chain
   fires open→discover→regions→catalog→sizes), results were attributed by
   a single `pendingOp` slot and misrouted — the REGIONS chip hung on
   "checking…" forever. Now every FFI result embeds `"op": <name>`
   (`dispatch` wraps `dispatch_op` in osm-sdk/src/ffi.rs) and the QML
   routes by `r.op`. Pre-tagging cores fall back to `pendingOp`.

QML fixes in the same version: output box is a real ScrollView (the bare
`ScrollBar.vertical` on a TextArea silently didn't attach — the box never
scrolled); `ToolTip.visible: hovered` → `containsMouse` (two ReferenceErrors
per render); "Host all except selected" shows its region count.

**Windows build gotchas that cost hours (all now encoded in
scripts/build-windows-cdylib.sh):** the rapidsnark tarball extracts NESTED
and must be flattened into `$RSN_DIR/lib`; the circuits `.lib→lib*.a`
renames must run OUTSIDE the download branch (a re-extracted tarball
restores `.lib` names); gcc temp files go to TMPDIR — point it at the big
disk, not a small tmpfs (a 7.6G tmpfs quota produced "error writing to
/tmp/cc*.s: Disk quota exceeded" AND silently zero-length static archives
that only fail later as "memory map must have a non-zero length").

## 🤝 STATE AT HANDOFF (2026-11-04) — Windows core built and vendored

`logos_osm.dll` (43 MB, x86_64-pc-windows-gnu, cross-compiled from Linux)
is built and vendored at `module/vendor/x86_64-windows/`; the flake's
postInstall copies it into the Windows `.lgx` next to the plugin, whose
$ORIGIN RPATH resolves it. The full build recipe — nine dependency walls,
each documented — is `scripts/build-windows-cdylib.sh`, mirrored by the
`Build Windows cdylib` CI workflow. Walls: rapidsnark (vendored the
windows-support rev of the crate), circuits statics (prebuilt artifact +
.lib→lib*.a), risc0-zkvm (unix-only prover compiled out via vendored
patch), rocksdb (posix-thread mingw), md5-asm (feature-gated off
windows), mcfgthread (official runtime + link-order resolvers
-lssp -lkernel32 -lntdll). Version 1.2.0 ships it.

## 🤝 STATE AT HANDOFF (2026-11-03) — packaging fix + app async fix

Two changes landed on `main` today (v1.1.0):

1. **`.lgx` now bundles the Rust core** (module/metadata.json: the vendored
   `module/vendor/<nix-system>/liblogos_osm.so` is declared under
   `nix.external_libraries` and staged into the module lib dir via the
   top-level `include` list). The plugin's RPATH is `$ORIGIN`, so the
   bundled lib resolves at runtime with no env vars. Before this, a fresh
   catalog install loaded the module but every op failed closed with
   "cannot load liblogos_osm.so" — evaluator-fatal, found 2026-10-31 in
   the live demo environment. Only linux-amd64 is vendored today (the
   only cdylib this box can build); other platforms keep the
   `LOGOS_OSM_FFI_PATH` env override and fail closed gracefully.
   Multi-platform vendoring needs CI-built cdylibs (macOS runner for the
   dylib; windows cross) — follow-up.
2. **The Basecamp app's QML uses async dispatch** (app/src/qml/Main.qml):
   `logos.module()` returns a Qt RemoteObjects DYNAMIC REPLICA — a QML
   method call on it returns a default value immediately; results only
   arrive via property sync (`lastResult`). The old `call()` read the
   return value, so every op showed `{"ok":false,"error":""}` while the
   op actually executed. Now ops are fire-and-tag and results route from
   `lastResult` changes; errors surface the same way. Open args also use
   absolute state paths (a module host's CWD can be a deleted dir).
   Verified live end-to-end (chain lookup through the GUI) in the demo
   environment on 2026-11-01.

Release flow for the catalog (UPDATED 2026-11-05): the release repo is now
**`ameenme/logos-modules-release-base`** (public; mirrored from aegonmyy
after the aegonmyy account hit the GitHub Actions billing gate — HTTP 422
"Actions disabled" on dispatch, in-flight runs cancelled; the burn is the
darwin-arm64 macOS legs at 10x billing plus private-repo minutes). Bump the
`submodules/logos-osm` pointer there, then dispatch
`release-logos-osm.yml` / `release-logos-osm-app.yml` with the ameenme
token. logos-repo.json's indexUrl points at the ameenme repo; the catalog
publishes unsigned (signing_mode none — no Actions secrets configured).
The aegonmyy mirror still serves releases through v1.3.3; aegonmyy git
pushes work fine, only its Actions are gated.

## 🤝 STATE AT HANDOFF (2026-09-30) — read this first

**Where things are:** `main`, at the tip of `aegonmyy/logos-osm`. The v0.3
port was merged here from the `lez-0.3` branch, along with the testnet
funding blocker from `blocked-testnet-funding`; both branches are now
redundant. The solution PR is **opened** (`logos-co/lambda-prize#169`, a
draft).

**The registry is ported to the LEZ v0.3 program model.** v0.3 replaced the
post-state API with `plan`/`apply`: a program emits one effect per shard it
writes, and each effect is applied separately against that shard's
pre-data. Consequences that matter:

- The guest is `run_program(plan, apply)`. `plan` validates what account
  metadata can establish and emits effects; `apply` enforces the
  state-dependent preconditions and returns new shard contents.
- **The registrar-claim pattern is gone.** v0.2 needed
  `Claim::Authorized` on first touch because a program owned whole accounts;
  v0.3 writes only the program's own shard, so there is nothing to claim.
  The rule-7 machinery below is history, not current behaviour.
- **A deployed program lives at an account the deployer chose**, not at a
  fixed function of its bytecode. PDAs and transaction targets key off that
  account (`--program-account` / `OSM_PROGRAM_ACCOUNT`), and
  `DOCUMENTED_IMAGE_ID_HEX` is only the bytecode identity used to verify the
  committed artifact. Getting this wrong is invisible until the accounts
  turn out not to exist.
- Instructions are **borsh** (they were risc0-serde `u32` words).
- Account reads are `account.data.shard(program_account)` — 0.3 stores a
  per-program shard map, not one blob per account.
- Toolchain is **1.98.1** (`rust-toolchain.toml`); 1.94 cannot compile the
  0.3 sequencer crates, which use if-let guards.

**Program image id:** `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79`
(guest artifact rebuilt 2026-09-30). Re-pin it in `methods/osm-host/build.rs`
(both the hex comment and the `[u32; 8]`), `osm-sdk/src/ffi.rs`, the README,
this file, `docs/DESIGN.md`, `docs/CU_COSTS.md`, both submission files, and
`tests/tests/osm_registry_testnet.rs`.

**What works right now, verified on this machine:**

- 66 tests pass across the workspace, 0 failures.
- The guest state machine's 10 host tests pass against the new model.
- `osm lookup` reads the registry from the live testnet over the sequencer's
  JSON-RPC (no wallet). Verified against `https://testnet.lez.logos.co`.
- The catalog publishes **both** modules on all four platforms: `osm` (the
  SDK core module) and `osm_app` (the distribution app). The app had never
  been built before 2026-09-30.
- CU costs re-measured for the plan/apply split (`docs/CU_COSTS.md`).

**What is blocked, and on what:**

1. **Deploying on testnet 0.3 needs a funded payer account**, which the
   testnet does not hand out. Read `docs/TESTNET_FUNDING.md` first: it has
   the observed failure, everything that was ruled out, and the one ask
   (a genesis-funded account, which is also what the 25 region
   registrations will need).
2. **The 25 regions** depend on (1).
3. **The 5 independent modules** need five real contributors; nothing in
   this repo can satisfy that.
4. **The app has never been *run* in Basecamp**, only built. Loading it
   needs the Basecamp runtime, which needs nix.
5. **macOS has never been exercised.** The catalog builds `darwin-arm64`.

**CI is `workflow_dispatch` only.** The suite's real-proof job runs the
whole lifecycle at `RISC0_DEV_MODE=0` and takes hours, so it is run
deliberately rather than on every push. The jobs now target hosted runners
and install Rust 1.98.1; before that they targeted the self-hosted box that
died with the VPS, and the hosted-runner guard read an env var GitHub does
not set.

**If you are a successor agent:** commit and push to `main`. Verify before
claiming: the README previously asserted a live testnet deployment that the
v0.3 chain reset had already erased, and the submission claimed macOS met on
the strength of a build.

---

## 🗄️ Historical: STATE AT HANDOFF (2026-08-28)



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
  `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79`
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
   `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79`
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
| Program | `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79` (= committed artifact id) |
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
