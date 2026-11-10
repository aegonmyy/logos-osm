# OSM CLI Reference

The `osm` command-line tool covers the full lifecycle of OpenStreetMap data
on Logos: discover regions, host them in Logos Storage, register on the LEZ
chain, fetch by CID, import locally, check for updates, and query the
on-chain registry.

## Global options

These go **before** the subcommand. They configure where the tool finds
Logos Storage, the LEZ sequencer, the Geofabrik index, and the deployed
registry program.

### `--storage-url <URL>`

Logos Storage (Codex) REST endpoint. This is where the tool uploads PBF
files during `host` / `host-bulk` / `import --registrar`, and where it
fetches them from during `fetch`. Point this at your own Codex node.

- Env: `OSM_STORAGE_URL`
- Default: `http://127.0.0.1:8080`

### `--memory-storage`

Use an in-memory storage backend instead of a real Codex node. Nothing
persists across runs. Useful for offline demos and testing the full workflow
without a storage node.

- Default: off (use real storage)

### `--cache <DIR>`

Directory for the download cache and the local catalog. Downloaded PBF files
are stored here, and the catalog (a JSON file tracking what you have hosted
or imported) lives in this directory.

- Env: `OSM_CACHE`
- Default: `osm-cache`

### `--geofabrik <URL>`

Geofabrik base URL. Override this to use a mirror or a local fixture instead
of the live Geofabrik downloads server. The tool appends region paths to
this base URL to construct download URLs.

- Env: `OSM_GEOFABRIK_URL`
- Default: `https://download.geofabrik.de`

### `--program-account <BASE58>`

The deployed registry program's account id (base58), as returned by the LEZ
deployment. Required for commands that build a registration transaction
(`host`, `host-bulk`, `register`, `register-bulk`, `import --registrar`,
`init`). Under the v0.3 program model, a deployed program lives at an
account the deployer chose, so PDAs and transaction targets key off that
account rather than off the bytecode's image id.

- Env: `OSM_PROGRAM_ACCOUNT`
- Default: none (must be set for tx-building commands)

### `--sequencer <URL>`

LEZ sequencer JSON-RPC endpoint. Used by `lookup` to read registry accounts
directly from the chain. Read-only: a query needs no wallet.

- Env: `OSM_SEQUENCER`
- Default: `https://testnet.lez.logos.co`

### `-h, --help`

Print the top-level help and exit.

### `-V, --version`

Print the CLI version and exit.

### Full invocation pattern

```sh
osm --storage-url http://127.0.0.1:8080 \
    --program-account D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt \
    --sequencer https://testnet.lez.logos.co \
    --cache /home/codespace/osm-anchor/cache \
    <subcommand> [options]
```

Or via environment variables:

```sh
export OSM_STORAGE_URL=http://127.0.0.1:8080
export OSM_PROGRAM_ACCOUNT=D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt
export OSM_SEQUENCER=https://testnet.lez.logos.co

osm regions
osm host germany --registrar <your-hex32>
```

---

## Subcommands

### `regions`

List the predefined, non-overlapping region set. The set is frozen at 72
regions (48 countries + 24 subregions) and is embedded in the SDK, so this
works without network access.

#### Options

| Option | Description |
|---|---|
| `--parent <PARENT>` | Only list subregions of this parent (e.g. `us`, `india`, `china`, `russia`) |
| `--level <LEVEL>` | Filter by level: `country` or `subregion` |
| `-h, --help` | Print help |

#### Examples

```sh
osm regions                    # all 72 regions
osm regions --parent us        # 8 US state subregions
osm regions --level country    # 48 country-level entries
osm regions --parent china     # 6 Chinese province subregions
```

---

### `discover`

Cross-check the closed set against Geofabrik's live index. Fetches the
Geofabrik JSON index over the network and verifies that every region path,
URL, and level in the frozen set still matches what Geofabrik publishes.
Use this to detect if Geofabrik has renamed or moved a region extract.

#### Options

| Option | Description |
|---|---|
| `-h, --help` | Print help |

#### Example

```sh
osm discover
# Output: 72 regions checked, 72 matched, 0 mismatched
```

---

### `host`

Host one region: download the PBF from Geofabrik, verify its MD5 against
Geofabrik's published checksum, store it in Logos Storage, add it to your
local catalog, and print the LEZ registration transaction as JSON. Does
**not** submit the transaction to the chain. The printed JSON contains the
program account, shard selectors, signer, and borsh-encoded instruction
bytes for the wallet to submit.

#### Arguments

| Argument | Description |
|---|---|
| `<REGION>` | Region path from the closed set (e.g. `germany`, `china/henan`, `us/california`) |

#### Options

| Option | Description |
|---|---|
| `--registrar <REGISTRAR>` | Registrar account id (32-byte hex). Sets the signer for the registration tx. The tx is printed for the wallet to submit. |
| `-h, --help` | Print help |

#### Example

```sh
osm host germany --registrar 4287ea6f05d6736f5ae9167a6f3dd46ef858aee54aba295cd547117e53e204b4

# Output:
# germany: 479034789 bytes, version 20261109, md5 a1b2c3...
# storage cid: zDvZRwzm...
# --- RegisterRegion transaction (submit via the wallet) ---
# { "image_id_hex": "...", "program_account_hex": "...", ... }
```

---

### `host-bulk`

Bulk host many regions in one invocation, with per-region opt-out. Downloads
and stores each region sequentially, then prints a single batch registration
transaction (one `RegisterRegionsBatch` instruction covering all hosted
regions). Use `all` as the regions list to attempt the entire 72-region set.

#### Options

| Option | Description |
|---|---|
| `--regions <REGIONS>` | Comma-separated region paths, or `all` for the full closed set |
| `--opt-out <OPT_OUT>` | Comma-separated region paths to skip (default: none) |
| `-h, --help` | Print help |

#### Examples

```sh
osm host-bulk --regions germany,france,spain --registrar <your-hex32>
osm host-bulk --regions all --opt-out russia/siberian-fed-district,australia --registrar <your-hex32>
```

---

### `register`

Build the LEZ registration transaction for a region you have already hosted
(it must be in your local catalog). By default this only **builds** the
transaction and prints it as JSON so you can inspect it before submitting.
Submitting to the chain is irreversible, so build-only is the safe default.

Pass `--sign` to also open your LEZ wallet, sign the transaction, and submit
it to the testnet sequencer, then await inclusion in a block.

#### Arguments

| Argument | Description |
|---|---|
| `<REGION>` | Region path that is already in your local catalog (hosted via `host` or `import`) |

#### Options

| Option | Description |
|---|---|
| `--registrar <REGISTRAR>` | Registrar account id (32-byte hex). The signer for the transaction. Required. |
| `--sign` | Sign with your LEZ wallet and submit the tx to the sequencer. Without this flag, the tx is only built and printed as JSON. |
| `--wallet-dir <WALLET_DIR>` | Wallet directory (contains `config.json` + `storage/`). Defaults to `~/.logos-osm/wallet/` (what `create_wallet` writes). Only used with `--sign`. Env: `OSM_WALLET_DIR` |
| `--wallet-password <WALLET_PASSWORD>` | Wallet password. Defaults to `testpw` (what `create_wallet` uses). Only used with `--sign`. Env: `OSM_WALLET_PASSWORD`. Default: `testpw` |
| `-h, --help` | Print help |

#### Examples

```sh
# Build only (inspect the tx before submitting):
osm register germany --registrar <your-hex32>

# Sign and submit to testnet:
osm register germany --registrar <your-hex32> --sign

# With explicit wallet location and password:
osm register germany --registrar <your-hex32> \
  --sign \
  --wallet-dir ~/.logos-osm/wallet \
  --wallet-password testpw
```

---

### `register-bulk`

Build a batch registration transaction for multiple regions at once. A
single `RegisterRegionsBatch` instruction covers all listed regions in one
on-chain transaction (up to 24 regions per batch). Same `--sign` opt-in as
`register`: build-only by default, sign and submit when `--sign` is passed.

#### Options

| Option | Description |
|---|---|
| `--regions <REGIONS>` | Comma-separated region paths (all must be in your local catalog) |
| `--registrar <REGISTRAR>` | Registrar account id (32-byte hex). Required. |
| `--sign` | Sign with your LEZ wallet and submit the batch tx to the sequencer. Without this flag, the tx is only built and printed as JSON. |
| `--wallet-dir <WALLET_DIR>` | Wallet directory. Defaults to `~/.logos-osm/wallet/`. Only used with `--sign`. Env: `OSM_WALLET_DIR` |
| `--wallet-password <WALLET_PASSWORD>` | Wallet password. Defaults to `testpw`. Only used with `--sign`. Env: `OSM_WALLET_PASSWORD`. Default: `testpw` |
| `-h, --help` | Print help |

#### Examples

```sh
osm register-bulk --regions germany,france,spain --registrar <your-hex32>
osm register-bulk --regions germany,france --registrar <your-hex32> --sign
```

---

### `init`

Build the registry `Init` transaction. This is a one-time operation at
deploy time that initializes the on-chain registry with an owner account.
Only run once after deploying the LEZ program. The owner is the account
that can administer the registry.

#### Options

| Option | Description |
|---|---|
| `--owner <OWNER>` | Owner account id (32-byte hex). The account that administers the registry. |
| `-h, --help` | Print help |

#### Example

```sh
osm init --owner 4287ea6f05d6736f5ae9167a6f3dd46ef858aee54aba295cd547117e53e204b4
```

---

### `fetch`

Fetch a registered snapshot from Logos Storage, verify the checksum, and
import it to your local catalog. If the region is not found in Logos Storage,
falls back to downloading directly from Geofabrik. You need the CID and
checksum from the on-chain registry (use `lookup` to get them).

#### Arguments

| Argument | Description |
|---|---|
| `<REGION>` | Region path (e.g. `germany`, `china/henan`) |

#### Options

| Option | Description |
|---|---|
| `--cid <CID>` | Logos Storage CID for the region's PBF snapshot. From the on-chain registry. Required. |
| `--checksum <CHECKSUM>` | Published MD5 (hex) for the region's PBF. From the on-chain registry. Required. The fetched bytes are hashed and compared against this. |
| `-h, --help` | Print help |

#### Example

```sh
osm fetch china/henan \
  --cid zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3 \
  --checksum 0acc723baca10b2860d82e706f7fcd7d
```

---

### `import`

Import a local PBF file. Without `--registrar`, it validates the file and
adds it to your local catalog (no storage upload, no registration). With
`--registrar`, it runs the full local-import workflow: verify the file
against Geofabrik's published MD5, store it in Logos Storage, add it to your
catalog, and print the LEZ registration transaction.

#### Arguments

| Argument | Description |
|---|---|
| `<REGION>` | Region path (e.g. `germany`, `china/henan`) |
| `<PBF>` | Path to the local `.osm.pbf` file |

#### Options

| Option | Description |
|---|---|
| `--registrar <REGISTRAR>` | Registrar account id (32-byte hex). When set, runs the full local-import workflow: verify against Geofabrik MD5, store in Logos Storage, and print the registration tx. Without this flag, only validates and catalogs locally. |
| `-h, --help` | Print help |

#### Examples

```sh
# Validate and catalog only:
osm import germany ./germany-latest.osm.pbf

# Full workflow (verify, store, print registration tx):
osm import germany ./germany-latest.osm.pbf \
  --registrar 4287ea6f05d6736f5ae9167a6f3dd46ef858aee54aba295cd547117e53e204b4
```

---

### `update`

Check whether Geofabrik has published a newer snapshot for a region than
what is in your local catalog or the on-chain registry. Compares the version
(Geofabrik snapshot date) of your hosted copy against Geofabrik's current
published version.

#### Arguments

| Argument | Description |
|---|---|
| `<REGION>` | Region path (e.g. `germany`, `china/henan`) |

#### Options

| Option | Description |
|---|---|
| `-h, --help` | Print help |

#### Example

```sh
osm update germany
# Output: germany: local version 20261109, latest 20261109 — up to date
```

---

### `catalog`

List your local catalog: every region you have hosted or imported, with its
CID, MD5, and file path. The catalog is stored as JSON in your cache
directory.

#### Options

| Option | Description |
|---|---|
| `-h, --help` | Print help |

#### Example

```sh
osm catalog
# Output:
# china/henan    cid=zDvZRwzkyRRY...  md5=0acc723b...  /path/to/henan.osm.pbf
# germany        cid=zDvZRwzm6bBf...  md5=a1b2c3...    /path/to/germany.osm.pbf
```

---

### `lookup`

Read registry entries directly from the LEZ chain. No wallet needed, this
is a read-only query. You can look up one region by its path, all
subregions of a parent, or find which region holds a given storage CID.

#### Options

| Option | Description |
|---|---|
| `--region <REGION>` | Look up one region by its Geofabrik path (e.g. `germany`, `china/henan`) |
| `--parent <PARENT>` | Look up every registered subregion of a parent path (e.g. `us`, `india`, `china`, `russia`) |
| `--cid <CID>` | Find the region whose stored snapshot has this Logos Storage CID |
| `-h, --help` | Print help |

#### Examples

```sh
osm lookup --region china/henan
# Output:
# china/henan  china  level=1  mirrors=3  registrars=1
#   4287ea6f... v20261106 ts=... cid=zDvZRwzm5xvy... md5=4695a425...
#   4287ea6f... v20261108 ts=... cid=zDvZRwzkweTn... md5=0c7d0316...
#   4287ea6f... v20261109 ts=... cid=zDvZRwzkyRRY... md5=0acc723b...

osm lookup --parent us
# Output: every registered US subregion with its mirrors

osm lookup --cid zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3
# Output: the china/henan entry that holds this CID
```

---

## Common workflows

### Host and register a region (full pipeline)

```sh
# 1. Download from Geofabrik, verify MD5, store in Logos Storage
osm host germany --registrar <your-hex32>

# 2. Register on-chain (inspect the tx first, then sign and submit)
osm register germany --registrar <your-hex32> --sign
```

### Mirror a region someone else has hosted

```sh
# 1. Look up the region on-chain to get its CID and checksum
osm lookup --region china/henan

# 2. Fetch it from Logos Storage and verify the checksum
osm fetch china/henan \
  --cid <cid-from-lookup> \
  --checksum <md5-from-lookup>

# 3. Re-host it under your own account (downloads from Geofabrik, re-stores, re-registers)
osm host china/henan --registrar <your-hex32>
osm register china/henan --registrar <your-hex32> --sign
```

### Bulk host multiple regions

```sh
osm host-bulk --regions germany,france,spain --registrar <your-hex32>
osm register-bulk --regions germany,france,spain --registrar <your-hex32> --sign
```

### Import a local PBF file you already downloaded

```sh
osm import germany ./germany-latest.osm.pbf --registrar <your-hex32>
osm register germany --registrar <your-hex32> --sign
```

### Browse the registry without a wallet

```sh
osm lookup --region germany           # one region's on-chain entry
osm lookup --parent us                # all US subregions
osm regions                           # the full 72-region set
osm catalog                           # your local hosted regions
```

### Check for updates

```sh
osm update germany                    # is there a newer Geofabrik snapshot?
```
