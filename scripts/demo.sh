#!/usr/bin/env bash
#
# demo.sh — reproducible end-to-end demo of OSM distribution on Logos.
#
# Runs the OSM registry lifecycle against a real local LEZ stack (brought up
# automatically by test_fixtures::TestContext via Docker) with REAL proofs:
# RISC0_DEV_MODE=0. The proof-generation output on screen is the evidence
# that dev mode is off.
#
# The demo proves, on a standalone sequencer:
#   - deploy of the OSM registry guest ELF + Init (owner claimed)
#   - germany registered by 3 distinct registrars with advancing versions
#     (20260801 -> 20260815 -> 20260821), append-only mirror history
#   - RegisterRegionsBatch { france, us/california, kenya }
#   - on-chain counters readback: region_count=4, registration_count=6
# Plus the off-chain lifecycle against a real Codex storage node:
#   - discover (all 72 regions resolve against the index)
#   - host germany: download -> MD5 verify -> store -> CID readback
#   - fetch from storage -> re-verify -> import -> update check -> tamper reject
#
# Prerequisites: Docker running; the RISC0 toolchain (r0vm 3.0.5).
# A Codex storage node on :8080 is started automatically if not reachable
# (digest-pinned; see README "Logos Storage node").
#
# Log filtering: RUST_LOG=warn keeps the output readable; `risc0_zkvm=info`
# additionally surfaces the per-session proof-execution summary (segments /
# total cycles) — the on-screen evidence of real proving the spec's video
# requirement asks for (the target is the crate name `risc0_zkvm`, underscore
# — EnvFilter's `risc0=info` does NOT match it — and risc0's Session::log()
# early-returns unless RISC0_INFO is set); `indexer_core=off` silences the
# standalone stack's indexer follower, which cannot re-verify
# privacy-preserving proofs of freshly deployed programs (no guest ELF) and
# parks at the first private block, logging one ERROR per block after. The
# sequencer is the authority — it validates every proof before inclusion and
# the tests read state back from it. Presentation-only (see ISSUES_TO_FILE.md).
#
# Usage:
#   ./scripts/demo.sh            # full lifecycle (real proofs)
#   ./scripts/demo.sh dev        # fast: RISC0_DEV_MODE=1 (no proof gen)

set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CODEX_NAME=osm-demo-codex
CODEX_DIGEST=sha256:25d9409b591da37200896e68fd9e4c591566f1ad0a80c0633c6cd5011ae56edb

banner() {
  echo
  echo "=================================================================="
  echo "  $1"
  echo "=================================================================="
  echo
}

if [[ "${1:-}" == "dev" ]]; then
  export RISC0_DEV_MODE=1
  MODE_NOTE="fast (no proof generation)"
else
  export RISC0_DEV_MODE=0
  MODE_NOTE="REAL Groth16 proofs — each registration takes a few minutes"
fi

banner "OSM on Logos — end-to-end demo"
echo "RISC0_DEV_MODE = $RISC0_DEV_MODE   ($MODE_NOTE)"
echo
echo "Part 1 (on-chain):  deploy -> Init -> 3 registrars x germany (versions"
echo "                    advancing) -> batch{france,us/california,kenya}"
echo "Part 2 (off-chain): discover(72) -> host+MD5-verify -> store -> fetch"
echo "                    -> re-verify -> import -> update-check -> tamper"
echo "                    reject, against a REAL Codex storage node."

# --- Bring up Codex storage if nothing is listening on :8080. -------------
code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:8080/api/storage/v1/data" || true)
if [[ "$code" != "200" ]]; then
  banner "Starting Logos Storage (Codex) node"
  # Digest-pinned + entrypoint override: the image's default config is broken
  # upstream (CMD ["codex"], no such binary — the binary is
  # /usr/local/bin/storage), so a plain `docker run` always dies.
  docker rm -f "$CODEX_NAME" >/dev/null 2>&1 || true
  docker run -d --name "$CODEX_NAME" -p 8080:8080 -e NAT_IP_AUTO=false \
    --entrypoint /usr/local/bin/storage \
    "codexstorage/nim-codex@$CODEX_DIGEST" \
    --api-bindaddr=0.0.0.0 --api-port=8080 >/dev/null
  echo -n "waiting for the node"
  ready=""
  for _ in $(seq 1 60); do
    code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:8080/api/storage/v1/data" || true)
    if [[ "$code" == "200" ]]; then ready=1; echo " ready"; break; fi
    echo -n "."
    sleep 2
  done
  if [[ -z "$ready" ]]; then
    echo "codex never became ready:" >&2
    docker logs "$CODEX_NAME" | tail -20 >&2
    exit 1
  fi
else
  banner "Logos Storage (Codex) already reachable on :8080 — reusing it"
fi

DEMO_LOG_FILTER='warn,risc0_zkvm=info,indexer_core=off'
# risc0's per-session proof summary (segments / user / total cycles) is
# opt-in via RISC0_INFO — without it Session::log() returns before logging
# (risc0-zkvm 3.0.5 src/host/server/session.rs).
export RISC0_INFO=1

banner "Part 1: on-chain registry lifecycle  [RISC0_DEV_MODE=$RISC0_DEV_MODE]"
( cd "$REPO_DIR" && RUST_LOG="$DEMO_LOG_FILTER" cargo test -p osm-integration-tests \
    --test osm_registry_live -- --nocapture --ignored --test-threads=1 )

banner "Part 2: off-chain lifecycle on real Codex storage"
( cd "$REPO_DIR" && OSM_CODEX_URL=http://127.0.0.1:8080 cargo test \
    -p osm-integration-tests --test offchain_live -- --nocapture --ignored )

banner "SDK unit tests (regions, geofabrik, verify, storage, ffi)"
( cd "$REPO_DIR" && cargo test -p logos-osm --lib -- --nocapture )

banner "Demo complete — lifecycle verified with RISC0_DEV_MODE=$RISC0_DEV_MODE"
