# TODO.md — post-announcement release roadmap

Staged features to build and ship AFTER the app is announced — one wave per
release, so the project shows a steady cadence of updates. Nothing here gets
implemented before the announcement.

Pre-announcement obligations (LP-0018 submission, durable re-host before the
VPS trial ends, adoption modules) are tracked separately in STATE.md — do not
let them slip behind this roadmap.

## Wave 1 — onboarding (remove the CLI and local-node hassle)

- [x] **Account format auto-conversion.** *(shipped v1.5.0)* — Every account field
      (`registrar_hex`, `owner_hex`, `program_account_hex`) parses hex only,
      but LEZ wallets display accounts in base58 (`AccountId` Display is
      plain base58, bitcoin alphabet, no checksum — `account.rs:300`), so
      users hit "invalid hex" on a copy-paste of exactly what their wallet
      shows. Fix once at the FFI layer: replace `account_from_hex`
      (`ffi.rs:463`) with a parser that accepts 64-char hex as-is, else
      falls back to base58 via `AccountId::from_str` (`account.rs:288`
      already implements it). Deterministic and bijective — same 32 bytes
      either way, and a wallet always derives the same account for a given
      index. Users then just paste their wallet account, in either format.
- [ ] **In-app wallet: create / import.** Port `tests/examples/create_wallet.rs`
      and `fund_wallet.rs` into the module so the app can create a wallet or
      import one from a mnemonic and sign registration txs in-app. Today the
      app builds unsigned txs (`tx_json`) and the user must take them to an
      external wallet — the biggest onboarding cliff for an ordinary
      developer.
- [ ] **Adaptive genesis funding.** `fund_account_from_genesis`
      (tests/src/lib.rs) hardcodes genesis account A, which is depleted
      (~0.9T atomic < the 1T transfer, so txs are accepted by the sequencer
      but never land). Fold in the both-accounts/adaptive-amount approach
      from `tests/examples/fund_wallet.rs`.
- [ ] **Remote Logos Storage node support.** The app assumes a local codex
      node (default `http://127.0.0.1:8080`); using a remote node today
      means hand-typing the URL in Connect. Make remote nodes first-class:
      - Node presets/picker in Connect (localhost, a public remote node,
        custom URL), persisted per install.
      - Connect-time health probe: store+fetch a tiny blob through the node
        and report reachable/unreachable + latency — surface "no Logos
        Storage node is running at the configured URL" BEFORE a multi-GB
        host op starts, not minutes into it.
      - WAN-tuned timeouts and retry for region uploads/downloads against
        remote nodes (local-loopback assumptions break over a VPS link);
        streamed byte progress already exists, keep it.
      - Optional: https/credentials once remote nodes serve them.
- [x] **Post-fetch "Finish on-chain" card is problematic.** *(shipped v1.5.0; in-app signing still pending the wallet item)* — After a
      Consumer-tab fetch, a banner appears (Main.qml ~2324, driven by
      `lastRunPending`) instructing: "Copy the command for each region, run
      it in the LEZ wallet CLI, then re-check." A consumer who just wanted
      the bytes gets pushed into a host-side registration workflow with
      leave-the-app CLI instructions — confusing, and the same CLI cliff as
      the rest of this wave. Fix: differentiate intent — for pure
      consumption, demote to a dismissible hint (or drop); for
      mirror-intent, replace the copy-paste CLI command with in-app signing
      once the Wave 1 in-app wallet lands.

## Wave 2 — data freshness

- [ ] **PBF version tracking + update (CLI and Basecamp).** Each hosted PBF
      has a version — the Geofabrik snapshot date, already recorded as
      `latest`/`version` (e.g. "20261029") in `osm-state.json` `region_statuses`
      and the catalog — but nothing tracks or advances it. Formalize:
      per-region local version vs current upstream snapshot date, surfaced in
      both interfaces; an update flow — fetch the newer snapshot → verify
      against Geofabrik's published MD5 → re-host → new CID → register the
      new version, keeping mirror history — runnable from BOTH the CLI and
      the Basecamp Host tab. Geofabrik republishes daily, so a fixed-date
      snapshot drifts off the published MD5 over time.
- [ ] **Re-anchor cadence.** A periodic (manual-trigger first) re-anchor for
      the hosted set so registry entries stay verifiable against current
      Geofabrik MD5s.

## Wave 3 — consumer experience

- [ ] **Regions tab display optimization.** The Regions tab (tab index 1)
      renders the 72-region catalog with a registered-first sort and an
      All/Registered/Not filter (`Main.qml` ~259-299), but no text search,
      no sort-by-size/name, and the filtered view recomputes on every
      interaction. Optimize: name search/filter, sort options (name, size,
      registered status), cache/memoize the filtered view, and tighten row
      density so scanning 72 entries is fast — finding one region today
      means scrolling the whole list.
- [ ] **Registry browser.** Read the on-chain registry (regions, mirrors,
      registrars, versions) and render it — makes the decentralized catalog
      legible without the CLI.
- [ ] **Map preview.** Render a fetched region's OSM data in-app (even a
      coarse tile render). The most demo-able consumer feature.
- [ ] **Fetch verification.** After a consumer fetch: verify MD5 against
      Geofabrik's published value and confirm the CID resolves to the same
      bytes — closes the trust loop visibly.

## Wave 4 — resilience

- [ ] **0.4 testnet reset runbook.** When 0.4 ships and the chain resets:
      re-deploy the program + re-register (hosting and CIDs survive —
      content-derived). Script it end to end from the anchor state.
- [ ] **Storage redundancy.** The single-VPS codex node is a hard dependency
      for fetches; mirror the region set on at least one independent node.
- [ ] **Hosted-region availability health check (flag dead registrations).**
      A region can be registered on-chain yet have a CID that no longer
      resolves to bytes — e.g. when the storage node that hosted it dies
      (the VPS codex trial ends 2026-11-24, taking every CID hosted only
      there with it). Add a health probe: for each registered region, attempt
      to resolve its CID on the configured storage node(s) and flag entries
      whose CID is unresolvable as "dead" (an unhealthy status in the
      Regions/Host tab, distinct from "not registered"). Surfaces
      availability gaps so dead mirrors get re-hosted before anyone notices
      a broken fetch — and pairs with the redundancy item above (redundancy
      prevents them, this surfaces what already slipped).

## Parking lot (candidates, not scheduled)

- Batch consumer fetch queue with per-region progress reuse across restarts
- Per-region mirror counts surfaced in the Host tab (adoption visibility)
