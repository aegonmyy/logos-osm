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

Proof generation is off-screen in this capture (`RUST_LOG=warn` suppresses
risc0's info-level logs), but it is what the run did: the identical test in
dev mode (`RISC0_DEV_MODE=1`, no proving) completes in ~532 s; this capture
took **1013.99 s** — the difference is Groth16 proof generation and
verification per transaction. The narrated video (prize deliverable) shows
the proof-generation output on screen; that recording is done by the
submitter outside this repo.
