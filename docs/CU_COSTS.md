# Compute-unit (CU) costs — OSM registry LEZ program (LP-0018 / PR #71)

Measured by `tests/tests/cycle_profile.rs` through the **risc0 local
executor** (`ExecutorImpl::from_elf` over the exact committed guest
artifact), real per-instruction guest cycle counts. Re-run in CI
(`cycle-profile` job):

```sh
cargo test --release -p osm-integration-tests --test cycle_profile -- --ignored --nocapture
```

## Measurements

Program: `c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79`
(guest as of this document; see "changelog" below). Cycle counts are
deterministic for a given input + guest image.

Under the v0.3 program model one instruction costs **two** guest
invocations: a `Plan` call that sees the instruction and the account
metadata, and one `Apply` call per shard the plan writes. Both are reported
per instruction, then summed.

| instruction | plan user/total | apply user/total | total cycles | accounts |
|---|---|---|---|---|
| `Init` | 12,746 / 65,536 | 8,191 / 65,536 | 131,072 | 2 |
| `RegisterRegion` (fresh region) | 23,029 / 131,072 | 22,161 / 131,072 | 262,144 | 3 |
| `RegisterRegion` (append, 2 mirrors) | 23,029 / 131,072 | 29,470 / 131,072 | 262,144 | 3 |
| `RegisterRegionsBatch` ×10 | 129,711 / 262,144 | 145,629 / 720,896 | 983,040 | 12 |
| `RegisterRegionsBatch` ×24 (cap) | 293,203 / 524,288 | 343,585 / 1,703,936 | 2,228,224 | 26 |

(total = user + system/overhead cycles as reported by the executor, summed
over the plan and every apply the instruction emits.)

## Single vs batch — the amortization

- **Single region** (one `RegisterRegion`): **262,144 total cycles**/region.
- **Batch of 24** (one `RegisterRegionsBatch`, the cap): **2,228,224 total
  cycles** ÷ 24 = **≈92,843 total cycles/region** — a **2.8×** reduction
  versus scalar registration, because the fixed per-transaction cost
  (instruction decode, registry read+write, signer handling, and the plan
  call itself) is paid once instead of per region.

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
- The cycle profile runs as a CI job (manual trigger), so a guest change that
  shifts CU costs is caught by re-running it rather than in production.
- The v0.3 numbers are roughly double the v0.2 ones for the same work, which
  is the plan/apply split showing up directly: every instruction now pays for
  two guest invocations instead of one. The batch instruction amortises that
  fixed cost across regions, so bulk hosting still costs far less per region
  than registering one at a time.

## Changelog

- 2026-08-22: first measurement (guest with permissionless registration,
  append-only mirrors).
- 2026-08-22: guest rebuilt with the registrar-claim fix (rule-7; see
  STATE.md postmortem) → program id `77ecdf2f…c1c43f0`; table above
  re-measured on the committed artifact (all counts within 0.3% of the
  pre-fix guest — the fix's claim check is noise-level in cycles).
- 2026-09-30: ported to the LEZ v0.3 program model and re-measured. The
  program now runs `plan` plus one `apply` per emitted effect, so the table
  reports both; the registrar-claim pattern is gone with the old post-state
  model. Guest rebuilt → image id
  `c272ec3c…a98e79`. The per-region amortisation survives the change
  (92,843 total cycles/region at the batch cap).
