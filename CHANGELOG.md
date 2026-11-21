# Changelog

## v1.6.0 (2026-11-20) — in-app wallet + remote nodes + data freshness

**In-app wallet (the headline):**
- "Setup wallet" button beside the REGISTRAR field opens a modal that explains what a
  wallet is for (it holds the account that signs region registrations on-chain — without
  one, the app can build registration transactions but can't submit them). Two paths:
  **Generate** (fresh keypair + primary account + a recovery mnemonic to write down) or
  **Import** (paste a recovery phrase). On success, "Use this wallet" fills the registrar
  field automatically.
- **"Sign all on-chain"** button in the post-fetch card: opens the wallet at
  `~/.logos-osm/wallet/`, signs + submits + polls for inclusion — as a background job
  with live task-panel status. No more copy-pasting tx JSON to a CLI.

**Remote Logos Storage nodes:**
- Connect tab gains a **probe** button (stores + fetches a tiny blob through the
  configured node; reports reachable/unreachable + round-trip latency) so a dead node
  surfaces *before* a multi-GB host op, not minutes into it — plus localhost/custom
  quick-fill presets.

**CLI:**
- `osm update <region> --apply` — when Geofabrik has published a newer snapshot,
  re-hosts it (download → verify MD5 → store → new CID) and prints the registration
  command.
- `osm reanchor [--regions <csv>]` — checks every hosted region for staleness and
  re-hosts the ones behind. Run periodically to keep registry entries verifiable
  against current Geofabrik MD5s (they republish daily).

**Regions tab:** text search + sort by name/size/registered — finding one region no
longer means scrolling all 72.

**Wallet tooling:** `fund_account_from_genesis` now tries both genesis accounts and
adapts the amount (account A was drained to ~0.9T, so 1T transfers silently never
landed).

## v1.5.0 (2026-11-14) — onboarding fixes

- Account fields accept 32-byte hex **or** the base58 form LEZ wallets display — paste
  what the wallet shows, no more "invalid hex."
- The post-fetch "Finish on-chain" card treats registration as optional (only for
  mirroring) and can be dismissed; fetched bytes are usable without registering.
- Version display fixed: the UI showed 0.1.0/1.0.0 because the crate + plugin versions
  had drifted from the release metadata. All version sources now bump together.

## Earlier releases

v1.0.0–v1.4.0 shipped the Basecamp app, background jobs with live progress,
dead-module detection, `register --sign`, the demo-polish rounds, and the consumer
example. See the release tags: https://github.com/aegonmyy/logos-modules-release-base/releases
