# Compute-unit (CU) costs — OSM registry LEZ program (LP-0018 / PR #71)

Measured by `tests/tests/cycle_profile.rs` through the **risc0 local
executor** (`ExecutorImpl::from_elf` over the exact committed guest
artifact), real per-instruction guest cycle counts. Re-run in CI
(`cycle-profile` job):

```sh
cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored --nocapture
```

## Measurements

Program: `77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0`
(guest as of this document; see "changelog" below). Cycle counts are
deterministic for a given input + guest image.

| instruction | user cycles | total cycles | accounts |
|---|---|---|---|
| `Init` | 96,142 | 262,144 | 2 |
| `RegisterRegion` (fresh region) | 161,127 | 262,144 | 3 |
| `RegisterRegion` (append, 2 mirrors) | 222,820 | 524,288 | 3 |
| `RegisterRegionsBatch` ×10 | 766,758 | 1,048,576 | 12 |
| `RegisterRegionsBatch` ×24 (cap) | 1,704,499 | 2,097,152 | 26 |

(total = user + system/overhead cycles as reported by the executor.)

## Single vs batch — the amortization

- **Single region** (one `RegisterRegion`): **262,144 total cycles**/region.
- **Batch of 24** (one `RegisterRegionsBatch`, the cap): **2,097,152 total
  cycles** ÷ 24 = **≈87,381 total cycles/region** — a **3.0×** reduction
  versus scalar registration, because the fixed per-transaction cost
  (instruction decode, registry read+write, signer handling) is paid once
  instead of per region. User-cycle amortization is similar:
  161,127 → ≈71,021/region.

The batch instruction is the right shape for bulk hosting (the prize's bulk
workflow): hosting 24 regions costs about the same total compute as ~8
scalar registrations. Growth inside the batch is close to linear in region
count (≈67k user cycles/region marginal), and appending to an existing
region costs more than a fresh one (223k vs 161k user) because the region
account's mirror list must be decoded, appended, re-sorted, and re-encoded —
bounded by the 32-mirror cap.

## Notes

- LEZ's per-transaction compute budget may change during testnet; these are
  executor-measured guest cycles for the program as committed, the quantity
  the budget ultimately prices.
- `MAX_BATCH = 24` keeps the batch instruction inside one transaction's
  instruction-size and account-count limits (2 + N accounts; N=24 → 26).
- The cycle profile also runs as a CI job on every push, so a guest change
  that shifts CU costs visibly is caught in the diff of this document's
  regeneration command, not in production.

## Changelog

- 2026-08-22: first measurement (guest with permissionless registration,
  append-only mirrors).
- 2026-08-22: guest rebuilt with the registrar-claim fix (rule-7; see
  STATE.md postmortem) → program id `77ecdf2f…c1c43f0`; table above
  re-measured on the committed artifact (all counts within 0.3% of the
  pre-fix guest — the fix's claim check is noise-level in cycles).
