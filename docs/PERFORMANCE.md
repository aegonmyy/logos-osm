# Performance measurements — OSM on Logos (LP-0018 / PR #71)

All measurements from this build's VPS on 2026-08-22, against the **real**
endpoints: `download.geofabrik.de` and a real Logos Storage (Codex) node
(digest-pinned container, loopback). CU costs are in
[`CU_COSTS.md`](CU_COSTS.md).

## Checksum-only verification (no PBF download)

The update check fetches **only** the `.md5` (~100 bytes) and reads the
`X-Derived-From` header for the version — no extract download:

| region | `.md5` fetch | version recovered |
|---|---|---|
| greece | 0.72 s (warm), 1.51 s (cold), 0.95 s (measured run) | `20260821` |
| denmark | 0.8 s | `20260821` |

## Full host workflow (download → MD5 verify → store)

The smallest closed-set regions are Greece (323.7 MiB) and Kenya (~333 MB); the
set's largest single file is Japan (2.4 GB). For scale reference the prize
text mentions Berlin ≈ 93 MB — Berlin is a Geofabrik *metro* extract,
outside the predefined closed set, so the closest comparable set entries are
the country files here.

**Greece (339,431,546 bytes = 323.7 MiB; live Content-Length agrees), raw
network pass (curl -L + md5sum), VPS → Geofabrik:**

| step | time | rate |
|---|---|---|
| `.md5` fetch (published checksum + version) | 0.95 s | — |
| PBF download (streaming) | 159.5 s | 2.0 MB/s |
| standalone MD5 pass over the file | 11.4 s | 28 MB/s |
| **verified MD5** `d05c23af3b8d82af80f622adc8e9a7b7` | = published ✓ | |

In the SDK the MD5 is computed **while the download streams** (hash chunks as
they land), so verify adds no wall-clock time after the last byte — the
11.4 s standalone pass is the cost only when re-verifying a file already on
disk (e.g. local import).

**End-to-end via the CLI (`osm host greece`) — the exact shipping workflow:
download → MD5 verify → store to a real Codex node, single command:**

| region | size | `.md5` fetch | download (MD5 hashed in-stream) | store → CID | total wall |
|---|---|---|---|---|---|
| greece | 339,431,546 B | incl. | incl. | incl. → `zDvZRwzm…dPKa8j4H` | **219.8 s (3:39.8)** |

Measured with `/usr/bin/time -v`: **peak RSS 32,560 KB ≈ 32 MB** for a
324 MiB region — the pipeline never buffers the file (download hashing,
verification, and the storage PUT all stream). CPU: 17.7 s user + 6.9 s
system (11% of one core — the workflow is network-bound, not CPU-bound).
The store step (streaming PUT to Codex, then CID readback) accounts for
roughly the last ≈60 s of the 219.8 s total given the raw-pass download
cost of ~160 s above.

The hosted snapshot verified before storing: version `20260821`, MD5
`d05c23af3b8d82af80f622adc8e9a7b7` = Geofabrik's published checksum.

## Notes

- Downloads are streaming with bounded memory (`verify` hashes as bytes
  arrive); a GB-scale region never sits in RAM.
- Storage put streams to the Codex REST node; the CID is content-derived, so
  re-putting identical bytes yields the same CID.
- The VPS pipe to Geofabrik measured 2.0 MB/s on this run (AWS region to
  Geofabrik's EU mirrors); consumers closer to a mirror see proportionally
  better times. The **verify** costs — what this repo controls — are
  bandwidth-independent: the checksum-only check is one round trip (<1 s),
  and a disk MD5 pass runs at disk speed.
- Real-Geofabrik end-to-end correctness is proven by the same numbers: the
  downloaded bytes' MD5 equals the published checksum for the version the
  `X-Derived-From` header names (`europe/greece-260821`).
