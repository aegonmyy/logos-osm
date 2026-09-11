# Logos tech problems to file as GitHub issues (user-only step)

Ready-to-paste drafts for the spec's "GitHub issues filed for any problems
encountered with Logos technology" deliverable. File against the relevant
repos when the submission goes in, then link them from
`submission/LP-0018.md`. Items marked *(shared)* were also hit by our
LP-0018 vault build — one issue each is enough; link it from both
submissions.

---

## 1. Explorer rejects raw-hex account IDs, blocking PDA verification

**Repo:** logos-co (explorer / testnet explorer)

`https://explorer.testnet.lez.logos.co/account/<58-byte-hex-PDA>` returns
`200 {"ServerError":"Invalid account ID"}` — identically for a real,
on-chain PDA and for a deliberately bogus one, so an evaluator (or us)
cannot independently read PDA state over HTTP. Base58 rendering of the same
account (as the wallet prints it) is accepted. The hex form is what the
SDK/RPC natively produce (`AccountId` Display), so the friction is real.
**Ask:** accept raw-hex account ids (58-byte or 32-byte) in explorer
account URLs.

## 2. Recurring-signer rule: a signer's 2nd tx to a program fails with `NonDefaultAccountWithDefaultOwner` unless the program claims the account

**Repo:** logos-blockchain/logos-execution-zone (docs) *(shared)*

On the public testnet, a registrar's **first** transaction (writing their
still-default account) succeeds; the sequencer then advances their nonce →
the account is non-default but `program_owner` is still default → every
**subsequent** tx by that signer fails with
`NonDefaultAccountWithDefaultOwner` unless the program's post-state claims
the account (`AccountPostState::new_claimed_if_default(acc,
Claim::Authorized)`). Nothing in LEZ/SPEL docs mentions this; we found it
by testnet failure and fixed the guest. **Ask:** document the rule and the
claim pattern for permissionless multi-tx signers (LEZ's own
`authenticated_transfer` uses the same pattern for the owner-at-Init case).

## 3. spel-framework pins an incompatible `nssa_core`

**Repo:** logos-co/spel *(shared)*

`spel-framework` v0.3.0 pins `nssa_core = v0.2.0-rc3`, while the working
stack (LEZ v0.2.4, the live testnet) uses v0.2.4 — mixing them fails to
build. Result: guest programs are hand-rolled on `lee_core::program`
instead of the framework macro (both prizes' specs ask for "SPEL
framework" IDLs; the types/encoding are equivalent). **Ask:** cut a
spel-framework release against nssa_core v0.2.4.

## 4. nim-codex image's default entrypoint is broken

**Repo:** codexstorage/nim-codex *(shared)*

The published image's `CMD ["codex"]` references a binary that does not
exist — the storage binary is `/usr/local/bin/storage`. A plain
`docker run codexstorage/nim-codex` dies with `exec: codex: not found`.
Workaround (documented in our README): `--entrypoint
/usr/local/bin/storage`. There is also no readiness probe-friendly cheap
endpoint; we warm up by retrying a small PUT for ~90 s. **Ask:** fix the
image entrypoint; add a `/healthz`.

## 5. risc0 guest builds are not byte-reproducible → program-id drift

**Repo:** logos-blockchain/logos-execution-zone or risc0 tooling *(shared)*

Building the same guest source twice (different machines / toolchain
states) yields different ELF bytes → a different program id, breaking
"deployed = tested" and any docs that pin the id. We work around it by
committing the deployed artifact (`methods/osm-host/osm_registry.bin`,
471,820 B) and pinning the id in `build.rs` with a unit test asserting
bytes→id→docs consistency. **Ask:** documented reproducible-build recipe
(pinned rustc + cargo-risczero versions + `RUSTFLAGS`) for guest ELFs.
