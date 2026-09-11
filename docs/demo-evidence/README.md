# Demo evidence

`osm_registry_live_dev0.log` — the full on-chain registry lifecycle
(`osm_registry_full_lifecycle_on_sequencer`) run with **`RISC0_DEV_MODE=0`**
(real Groth16 proving), captured 2026-08-22 on the build VPS:

```sh
RISC0_DEV_MODE=0 RUST_LOG=warn \
  cargo test -p osm-integration-tests --test osm_registry_live \
  -- --ignored --nocapture --test-threads=1
```

What the log shows:

- deployment of the committed guest artifact —
  `ProgramDeployment(ProgramDeploymentTransaction { message: Message { bytecode: <471820 bytes> } })`
  (the exact `methods/osm-host/osm_registry.bin` size);
- every subsequent transaction (`Init`, 3 × `RegisterRegion`,
  `RegisterRegionsBatch`) with `Transaction is included in block N` lines —
  inclusion means the sequencer verified each tx's real proof;
- final readback assertions passing (`ok` / `test result: ok`).

Proof generation is off-screen in this text capture (`RUST_LOG=warn`
suppresses risc0's info-level logs), but it is what the run did: the
identical test in dev mode (`RISC0_DEV_MODE=1`, no proving) completes in
~532 s; this capture took **1013.99 s** — the difference is Groth16 proof
generation and verification per transaction.

`osm-demo-dev0-20260822T195048Z.cast` — the **full demo** (`scripts/demo.sh`,
on-chain + off-chain legs + unit tests) captured as an asciinema cast
(200×50 terminal, `RISC0_DEV_MODE=0`, run 2026-08-22). Unlike the text log
above, the demo now runs with `RUST_LOG='warn,risc0_zkvm=info,
indexer_core=off'` + `RISC0_INFO=1`, so the risc0 per-session proof summary
(segments / user / total cycles, ecalls, syscalls) is **on screen** for
every prove — the explicit proof-generation evidence the spec's video
requirement asks for (the wall-clock delta above is the same evidence for
the text log). This cast is the footage for the narrated video; the
voiceover is added by the submitter on replay (`asciinema play` + screen
recorder, or convert with `agg`).
