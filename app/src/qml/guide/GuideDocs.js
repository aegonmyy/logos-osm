// The bundled guide documents, rendered by the Guide tab.
//
// The markdown lives here as string constants (a .pragma library is read by
// the QML engine's type loader, the same channel that loads this file's
// neighbors — it works inside the Basecamp sandbox). Runtime file reads were
// the other candidate: QML's XMLHttpRequest is disabled for local files by
// default in Qt 6, and the sandboxed engine's network stack denies every
// request, so loading guide/*.md off disk had no working path. See STATE.md
// (v1.3.4) for the verification.
//
// Formatting note: code samples use 4-space indented blocks so the file
// needs no escaped backticks.

.pragma library

const docs = {
    "how-it-works": {
        title: "How the distribution works",
        body: `# How the distribution works

OpenStreetMap publishes the whole planet in one enormous file. What a map
app usually wants is one country, or one state, at a time. This module
moves those regional files between three cooperating pieces:

- **Hosts** download a region from Geofabrik (the long-standing mirror of
  OpenStreetMap extracts), check the bytes against the published MD5
  checksum, and store them in Logos Storage — a blob store where a file's
  content hash is its address (the CID).
- **The registry** is a small program on the LEZ chain. It records who
  hosted what: one entry per region holding the CID, the checksum, and a
  version number. Reading it needs no wallet, only a network connection.
- **Consumers** pick a region, read its registry entry, fetch the bytes
  from storage, and verify the checksum again on arrival before importing.

## The trust triangle

Each leg covers the others:

1. **Geofabrik is the ground truth.** It publishes both the region file
   and its MD5. Bytes altered anywhere along the way produce a different
   checksum and fail the check.
2. **The chain is the permanent record.** A registration cannot be edited
   or quietly deleted afterwards. The entry says: this CID, this
   checksum, this version, filed by this registrar.
3. **The MD5 check is the delivery verification.** The consumer recomputes
   the checksum of what actually arrived and compares it with the
   registered one. Matching bytes prove the delivery is the thing that
   was registered.

## What a registrar is

A registrar is the public account ID that files registrations — an
address, 64 hex characters. Anyone can be one; filing is permissionless
and the registry keeps every distinct registrar a region gathers
(adoption likes three or more). The registrar signs the registration
transaction with their wallet; this app builds the transaction and hands
it over for signing. It never sees a private key.

## Why the wallet signs and the app never sees keys

The rule that keeps the system safe: the app assembles *what* should be
registered, and the wallet decides *whether* to sign it. Your keys stay
in the wallet app, in whatever storage it keeps them in. A bug in this
module can produce a wrong transaction; it cannot produce a signature.`

},
    "hosting": {
        title: "Hosting regions",
        body: `# Hosting regions

The Host tab turns your machine into a source of map data for the
network. Three steps: connect, pick, host.

## Connect

The four fields say where the pieces live:

- **Storage URL** — your Logos Storage node. The default points at a
  local one (http://127.0.0.1:8080). Hosted blobs land here, and this is
  what your node serves to others.
- **Geofabrik URL** — where the region files download from. Leave the
  default unless you run a mirror.
- **Cache dir** — where downloaded files stay on disk, so re-hosting and
  verifying skip the network.
- **Registrar** — your account ID (64 hex characters), used when you ask
  for a registration transaction. Find it in your wallet app under
  account details.

The dot next to STORAGE goes green when your storage node answers the
open call, red when it does not.

## Pick regions

"Select regions" opens the full region set with a search box and each
region's current download size. Selected regions pin as chips with an
× to remove. The set is frozen at 72 regions — countries plus the US
states.

## The three host actions

Each button hosts regions one at a time, so the tasks panel shows
per-region progress:

- **Host selected** — exactly the pinned regions.
- **Host all except selected** — the rest of the set.
- **Host all** — every region your node has not hosted yet.

Hosting one region means: download from Geofabrik, verify the bytes
against Geofabrik's published MD5, store them in Logos Storage, and
record the snapshot in the local catalog.

## What "hosted" means for your node

After a region hosts, your storage node holds that region's bytes under
their CID. Anyone who knows the CID (from the registry, or from you) can
fetch them from your node. Your cache dir keeps the original file so a
re-verify or re-store never re-downloads.`

},
    "fetching": {
        title: "Fetching maps",
        body: `# Fetching maps

The Consumer tab is the reader's side: get a region's map data into your
local catalog, verified.

## Pick a region

"Select region" opens the same region list as the Host tab — one choice,
not a basket. Your pick pins as a chip showing the region and its
download size.

## CID and MD5 auto-fill

The moment you pick a region, the app reads its registry entry from the
chain and fills in the two values a fetch needs:

- **CID** — the storage address of the region's bytes.
- **MD5** — the registered checksum.

The caption under the fields says where things stand: "from registry ✓"
when both filled, "not on the registry yet — nothing to fetch" when the
region has no registration, and "registry unreachable — enter manually"
when the chain read failed (the fields stay editable so you can paste
values from a friend or from \`osm lookup\` output).

## Fetch + import

The button downloads the bytes from Logos Storage, checks them against
the registered MD5, and imports the region into your local catalog. A
checksum mismatch stops the import — you never get unverified data.

## Import a local PBF

Already have the file on disk (from Geofabrik directly, or a copy)?
Pick the region and point at the file. "Use cached file" appears when
that region's hosted copy already sits in your cache dir — one click
fills the path. **Import (validate + catalog)** checks the file's
structure and catalogs it. **Verify + store + register tx** additionally
checks it against Geofabrik's published MD5, stores it in Logos Storage,
and prints the registration transaction for your wallet to sign.

## Update check

Pick a region and check whether Geofabrik has published a newer snapshot
than the one in your catalog: "up to date", "update available", or "not
imported" (with the currently published version, so you can fetch it).`

},
    "register-onchain": {
        title: "Registering regions on-chain (CLI)",
        body: `# Registering regions on-chain (CLI)

The app builds registration transactions; the CLI is the full workflow,
including the setup steps that happen once per deployment. Every command
prints a transaction as JSON — your wallet signs and submits it.

## Prerequisites

- The \`osm\` binary built from this repository (cargo build -p
  logos-osm-cli).
- A wallet that can sign and submit LEZ transactions.
- A Logos Storage node reachable at a URL (\`--storage-url\`).

## A funded account

Every chain write costs fees, and the testnet has no faucet you can
call. Accounts with tokens exist from the network's genesis — the
testnet publishes those genesis accounts and their keys, which makes
the published genesis account the dev faucet: send yourself tokens from
it, exactly like the LEZ tutorial's "fund the account you just created
from a genesis-funded devnet account" step.

## Deploying the program

The registry is a guest program. Deploying it is a sequence of
fee-bearing transactions, so the payer must be the funded account.
The deployment returns the program's new address (base58) — that
address is how everything else finds the program:

    osm --program-account <base58-from-the-deploy> ...

(Or set OSM_PROGRAM_ACCOUNT.) The bytecode's image id
(c272ec3c…98e79) is only its build fingerprint; the account address is
the live handle.

## Init, once

Bind the registry's owner (a provenance label; registration itself
stays open to anyone):

    osm init --owner <64-hex>

Prints the Init transaction JSON for the wallet.

## Host regions

Registration files what you actually stored, so host first:

    osm host germany --registrar <64-hex>
    osm host-bulk --regions germany,france,denmark

Each host downloads, verifies against Geofabrik's MD5, stores in Logos
Storage, and catalogs the snapshot locally.

## Register

One region (from the local catalog) or up to 24 per transaction:

    osm register germany --registrar <64-hex>
    osm register-bulk --regions germany,france --registrar <64-hex>

## Verify

Read back what the chain now records — needs no wallet:

    osm lookup --region germany
    osm lookup --parent us
    osm lookup --cid <cid>

Each prints the region's entry: every mirror with its registrar, CID,
MD5, version, and timestamp. When your region shows three or more
distinct registrars, the adoption bar is met.`
    }
};

// The doc list order the Guide tab renders.
const order = ["how-it-works", "hosting", "fetching", "register-onchain"];

function doc(id) {
    return docs[id] || null
}

function list() {
    return order.map(id => ({ id: id, title: docs[id].title }))
}
