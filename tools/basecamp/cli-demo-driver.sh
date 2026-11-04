#!/usr/bin/env bash
# cli-demo-driver.sh — the scripted CLI demo session. Types each command
# slowly (readable), pauses, runs it for real, pauses on the output.
# Executed INSIDE `asciinema rec` so the cast captures everything.
# Per-action timestamps are appended to /tmp/basecamp/cli-timing.log
# as "epoch <phase> <command>" lines; offsets are computed afterwards.
LOG=/tmp/basecamp/cli-timing.log
: > "$LOG"
STAMP() { date +%s.%3N >> "$LOG"; echo "$1" >> "$LOG"; }

O=/workspaces/workspace/logos-osm/target/release/osm
CACHE=/home/codespace/osm-anchor/cache
REG=bb6ebd746d972e151603f26b80675b4077cd3ec869aadd0bfb35766d6307bc27
PROG=D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt

cmd() {
  STAMP "start: $1"
  printf '\r$ '
  local s="$1"
  for (( i=0; i<${#s}; i++ )); do
    printf '%s' "${s:$i:1}"; sleep 0.055
  done
  printf '\n'
  sleep 1.2
  eval "$1" 2>&1
  STAMP "end:   $1"
  sleep 4.5
  printf '\n'
}

clear
printf 'OpenStreetMap on Logos — the osm CLI (LP-0018)\n'
printf 'registry on LEZ testnet + Logos Storage, driven headlessly\n\n'
sleep 3.5

STAMP "phase: regions"
cmd "$O regions | head -14"
printf '... (the frozen set is 72 regions: 48 countries + 24 subregions)\n\n'
sleep 3.5

STAMP "phase: discover"
cmd "$O discover | head -12"
printf "... (every region resolves against Geofabrik's live index)\n\n" 
sleep 3.5

STAMP "phase: host"
cmd "$O --cache $CACHE host --registrar $REG hungary | head -30"
printf "... (fetch -> verify Geofabrik MD5 -> store to Logos Storage -> CID)\n" 
printf '... (then the RegisterRegion transaction is printed for the wallet)\n\n'
sleep 4

STAMP "phase: lookup"
cmd "$O --program-account $PROG lookup --region hungary"
printf '... (live read from the on-chain registry: version, CID, MD5, registrar)\n\n'
sleep 4

STAMP "phase: catalog"
cmd "$O --cache $CACHE catalog | head -12"
printf '... (the local hosted catalog: region -> CID -> MD5)\n\n'
sleep 4

printf 'Full pipeline, headless. The SDK underneath is what Basecamp apps\n'
printf 'and other modules consume — see docs/sdk.md and the consumer example.\n'
sleep 4
STAMP "phase: done"
