# Local demo setup — Logos Storage node + LEZ wallet CLI

> For running the OSM app's full flow on **your own PC** (demo only). The
> heavy lifting (the full 60 GB region set, the indexed catalog) stays on
> the codespace; this guide sets up just enough locally to complete a
> fetch → store → register → verify round-trip for one small region.

## Why you need these

The OSM app's "Fetch" button does two things:

1. **Download** the region PBF from Geofabrik → your cache dir. *(works with
   nothing else running — you saw the file land on disk)*
2. **Store** it in a **Logos Storage node** (PUT to `storage_url`, default
   `http://127.0.0.1:8080`) → returns the CID. *(fails with "Logos Storage
   node at … did not respond" if no node is running — this is the error you
   hit)*

And to make a region **truly available on Logos** (the chain), a third step:

3. **Register** it on-chain — a transaction the **LEZ wallet** signs and
   submits. The app builds the tx; the wallet signs; the app never holds keys.

So: step 2 needs a Storage node; step 3 needs the wallet CLI. Both run
locally alongside Basecamp.

## Disk estimate (demo, one region)

| What | Size | Notes |
|---|---|---|
| Logos Storage app (in Basecamp) | ~50–150 MB | the module + its DB; grows with what you store |
| LEZ wallet CLI (build) | ~1.5–2.5 GB | Rust compile deps (one-time); the binary itself is ~30–60 MB |
| `osm` CLI (this repo) | ~1 GB | builds the SDK + CLI; one-time |
| One demo region (`china/henan`) | 51 MB | the PBF in your cache dir + a copy in storage |
| **Total for a minimal demo** | **≈ 3–4 GB** | the full 60 GB set stays on the codespace |

## 1. Logos Storage node — run it from Basecamp (no separate binary)

The Storage node is a **Logos module that runs as an app inside Basecamp** —
the same Basecamp you already have. You don't install a separate `storage`
binary or Docker image.

**Prerequisite:** Logos Basecamp **0.3.2 or later**, installed and running
([install guide](https://docs.logos.co/basecamp/install-logos-basecamp)).

**Steps** (full guide:
[Run a Storage Node from Basecamp](https://docs.logos.co/storage/get-started/run-storage-node-from-basecamp)):

1. In Basecamp, open the **Logos Storage** app (Applications → Storage tile,
   same way you open the OSM app).
2. The first run shows an **onboarding wizard** — accept the defaults. The
   node binds its API locally (the OSM app's default `storage_url`
   `http://127.0.0.1:8080` points here).
3. Once the node is up, the OSM app's **Connect** section STORAGE dot goes
   green (the `open` call reaches it).

That's it — no CLI, no Docker. The Storage app manages the node lifecycle
from Basecamp's app bar.

> If you ever prefer a standalone node (headless server), the same module
> runs from the CLI:
> [Run a Storage node from the CLI](https://docs.logos.co/storage/get-started/run-storage-node-from-cli).

## 2. LEZ wallet CLI — install + connect to testnet

The wallet signs LEZ transactions (the registration tx the OSM app builds).

**Prerequisite:** Rust + cargo ([rustup](https://rustup.rs)) and build deps:

```sh
# Ubuntu / Debian
sudo apt update
sudo apt install git curl build-essential clang libclang-dev pkg-config libssl-dev libpcsclite-dev
# macOS
xcode-select --install
brew install pkg-config openssl
```

**Install** (full guide:
[Run an LEZ wallet via the CLI](https://docs.logos.co/lez/get-started/run-lez-wallet-via-cli)):

```sh
git clone https://github.com/logos-blockchain/logos-execution-zone.git
cd logos-execution-zone
git checkout v0.3.0          # must match the testnet's programs
mv ~/.lee/wallet ~/.lee/wallet.old 2>/dev/null || true   # avoid conflicts
cargo install --path lez/wallet --force
wallet change-network testnet        # point at https://testnet.lez.logos.co
wallet check-health                  # → "✅ All looks good!"
```

Create your signer account and note its ID (this is the **registrar** the
OSM app's Connect field wants — 64 hex chars):

```sh
wallet account new public           # prints account_id Public/<…>
```

To **fund** it (every chain write costs fees; the testnet has no faucet):
bridge tokens from the Logos Blockchain via a channel deposit, or have
another testnet user send you native tokens. See
[Fund the sender account](https://docs.logos.co/lez/get-started/run-lez-wallet-via-cli#fund-the-sender-account).

## 3. The `osm` CLI — build from this repo

The `osm` binary (`logos-osm-cli` package) is what actually emits the
registration transaction JSON for the wallet to sign.

```sh
cd /path/to/logos-osm            # this repository
cargo build -p logos-osm-cli --release
# binary: target/release/osm
```

`osm --help` lists the subcommands; the ones that matter for the demo:

```sh
osm host china/henan --registrar <64-hex>          # download + verify + store
osm register china/henan --registrar <64-hex>      # build the register tx (prints JSON)
osm register china/henan --registrar <64-hex> --sign   # build + sign + submit in one step
osm lookup --region china/henan                    # read back the chain entry (no wallet)
```

> `register` (and `register-bulk`) **build only** by default — printing the
> transaction JSON is the safe, inspect-first path, since submitting to the
> chain is irreversible. Adding `--sign` opts into the full flow: the CLI
> opens your LEZ wallet (`~/.logos-osm/wallet/`, the one `create_wallet`
> wrote — override with `--wallet-dir`), signs, submits to the testnet
> sequencer, and awaits inclusion.

## 4. End-to-end demo round-trip

With the Storage app running in Basecamp, the wallet installed, and `osm`
built — pick **one small region** (`china/henan`, 51 MB) so the demo stays
light:

```sh
# 1. Fetch (download + store) — the OSM app's Fetch button does this in the
#    GUI; the CLI equivalent (needs storage_url + geofabrik + account env):
osm host china/henan --registrar <64-hex>

# 2. Register on-chain — one command builds, signs, and submits:
osm register china/henan --registrar <64-hex> --sign
#    (omit --sign to just print the tx JSON for inspection / another signer)

# 3. Verify — read back what the chain now records (no wallet needed):
osm lookup --region china/henan
```

Or drive the build from the **OSM app GUI** (Fetch button → "Finish
on-chain" card gives you the exact `osm register …` command), run it with
`--sign` in your terminal, then click **re-check registry** in the app to
see the badge flip to `registered ✓`.

## Troubleshooting

- **"Logos Storage node at http://127.0.0.1:8080 did not respond"** — the
  Storage app isn't running in Basecamp (or bound elsewhere). Open it; check
  the STORAGE dot in the OSM app's Connect section is green.
- **Wallet `check-health` fails** — wrong network (`wallet change-network
  testnet`) or the testnet sequencer is down.
- **Registration tx rejected (fees)** — your registrar account has no
  tokens; fund it first (see §2).
- **`cargo install --path lez/wallet` slow/huge** — that's the one-time LEZ
  stack compile (~2 GB of build deps); subsequent rebuilds are incremental.

## Reference docs

- Logos Storage intro: https://docs.logos.co/storage
- Run a Storage Node from Basecamp: https://docs.logos.co/storage/get-started/run-storage-node-from-basecamp
- Run a Storage node from the CLI: https://docs.logos.co/storage/get-started/run-storage-node-from-cli
- LEZ wallet via CLI: https://docs.logos.co/lez/get-started/run-lez-wallet-via-cli
- LEZ wallet app (Basecamp GUI alternative): https://docs.logos.co/lez/get-started/run-lez-wallet-app-initiate-native-token-transfers
- Install Logos Basecamp: https://docs.logos.co/basecamp/install-logos-basecamp
