# Deploying on testnet 0.3: the funding requirement

**Status: blocked on an external dependency.** Deploying the registry on
testnet 0.3 needs a payer account that already holds tokens. A freshly
created account holds nothing, and the testnet does not fund one.

## What was tried

`tests/tests/osm_registry_testnet.rs` was run against
`https://testnet.lez.logos.co` on 2026-09-30. It reached the chain (synced to
block 642), created a wallet and a payer account, and started the deploy. The
deploy failed:

```
Error: failed to upload segment 4
Caused by: Sequencer client error: ErrorObject {
    code: InvalidParams, message: "Incorrect fee", data: None }
```

The payer's on-chain balance was then read directly:

```
getAccountBalance(GpxtG8UBKfbHPqkSJ3CDeRP8zHi6cJUhDSEgAuHgJ8gR) -> 0
```

So the failure is funds, not a malformed transaction. A deploy is a sequence
of fee-bearing transactions (the v0.3 flow creates a header account plus one
segment account per 100 KiB of bytecode, then deploys), and a zero-balance
account cannot pay them. The error surfaces at the fourth segment because
that is where the charge first bites.

## Why this is not something the code can fix

The v0.3 deploy helper states the constraint directly:

> `payer` must be an existing, funded account. A freshly-claimed account
> can't pay for its own claim.

And the LEZ tutorial docs describe funding as a transfer *from an account
that already holds tokens*:

> Fund the account you just created from an account of yours that already
> holds tokens (for example a genesis-funded devnet account).

There is no faucet to call. The usual routes were checked and do not apply:

- no faucet endpoint under `testnet.lez.logos.co` or its subdomains;
- the wallet CLI has no fund/faucet subcommand;
- the token program's `mint` requires authorization over the token
  definition, so an unauthorised account cannot mint itself funds, and
  creating a definition is itself fee-bearing.

The first funded account on a testnet comes from genesis, and its keys are
held by whoever ran the network.

## What is needed

One of:

1. **A genesis-funded testnet account.** The LEZ/Logos team holds these.
   Asking in the builder channel for a funded account on testnet 0.3 is the
   direct route, and it is the same account that would later register the 25
   regions, so it is worth asking once rather than per-task.
2. **A documented funding mechanism** for testnet 0.3 that the docs above
   do not mention. If one exists, this document should be replaced by it.

## What is already in place for when funding arrives

- The deploy path is ported to the v0.3 flow and compiles.
- The test now checks the payer's balance before deploying and fails with
  this file's explanation rather than a fee error, so the next person to run
  it is told what to do.
- Everything downstream of the deploy (Init, registration, batch
  registration, and the `lookup` read path) is implemented and tested
  against a standalone sequencer.

## Note on the earlier deployment

A registry was deployed on the pre-0.3 testnet in August 2026, and that run
created its accounts the same way. Either that testnet funded new accounts,
or the deployment used an account provisioned outside the test. Either way,
the behaviour of testnet 0.3 as observed above is that new accounts start
empty.
