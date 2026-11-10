# OSM CLI Reference

The `osm` command-line tool covers the full lifecycle of OpenStreetMap data
on Logos: discover regions, host them in Logos Storage, register on the LEZ
chain, fetch by CID, import locally, check for updates, and query the
on-chain registry.

## Global options

These go before the subcommand:

| Option | Description | Default |
|---|---|---|
| `--storage-url <URL>` | Logos Storage (Codex) REST endpoint | `http://127.0.0.1:8080` |
| `--memory-storage` | Use in-memory storage (offline demo, nothing persists) | off |
| `--cache <DIR>` | Download cache and catalog directory | `osm-cache` |
| `--geofabrik <URL>` | Geofabrik base URL (mirror or fixture override) | `https://download.geofabrik.de` |
| `--program-account <BASE58>` | Deployed registry program account (required for tx-building commands) | env `OSM_PROGRAM_ACCOUNT` |
| `--sequencer <URL>` | LEZ sequencer JSON-RPC endpoint (read-only queries) | `https://testnet.lez.logos.co` |

```sh
osm --storage-url http://127.0.0.1:8080 \
    --program-account D3f2BijChudS9Y1BKa4XVfVbZjmkzgGzdZjQGkDz1BVt \
    --sequencer https://testnet.lez.logos.co \
    <subcommand> [options]
```

## Subcommands

### `regions`

List the predefined, non-overlapping region set (72 regions).

```sh
osm regions                    # all 72 regions
osm regions --parent us        # US subregions only
osm regions --level country    # country-level entries only
```

### `discover`

Cross-check the closed set against Geofabrik's live index. Verifies that the
region URLs and paths in the frozen set still match what Geofabrik publishes.

```sh
osm discover
```

### `host`

Host one region: download the PBF from Geofabrik, verify its MD5 against
Geofabrik's published checksum, store it in Logos Storage, and print the LEZ
registration transaction (as JSON for the wallet to submit). Does not submit
to the chain on its own.

```sh
osm host germany --registrar <your-hex32>
osm host china/henan --registrar <your-hex32>
```

### `host-bulk`

Bulk host many regions in one go, with per-region opt-out. Downloads and
stores each region, then prints a batch registration transaction.

```sh
osm host-bulk --regions germany,france,spain --registrar <your-hex32>
osm host-bulk --regions all --opt-out russia/siberian-fed-district --registrar <your-hex32>
```

### `register`

Build the LEZ registration transaction for a region you have already hosted
(in your local catalog). By default this only builds and prints the tx as
JSON. Pass `--sign` to open your LEZ wallet, sign, and submit to the sequencer.

```sh
# Build only (safe, inspect first):
osm register germany --registrar <your-hex32>

# Sign and submit to testnet:
osm register germany --registrar <your-hex32> --sign
osm register germany --registrar <your-hex32> --sign --wallet-dir ~/.logos-osm/wallet --wallet-password testpw
```

### `register-bulk`

Build a batch registration transaction for multiple regions. Same `--sign`
opt-in as `register`.

```sh
osm register-bulk --regions germany,france,spain --registrar <your-hex32>
osm register-bulk --regions germany,france --registrar <your-hex32> --sign
```

### `init`

Build the registry `Init` transaction. This is a one-time operation at
deploy time that initializes the on-chain registry with an owner account.

```sh
osm init --owner <your-hex32>
```

### `fetch`

Fetch a registered snapshot from Logos Storage (with Geofabrik as fallback),
verify the checksum, and import it to your local catalog. You need the CID
and checksum from the on-chain registry.

```sh
osm fetch china/henan \
  --cid zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3 \
  --checksum 0acc723baca10b2860d82e706f7fcd7d
```

### `import`

Import a local PBF file. Without `--registrar`, it validates and catalogs the
file locally. With `--registrar`, it runs the full local-import workflow:
verify against Geofabrik's published MD5, store in Logos Storage, and print
the LEZ registration transaction.

```sh
# Validate and catalog only:
osm import germany ./germany-latest.osm.pbf

# Full workflow (verify, store, print registration tx):
osm import germany ./germany-latest.osm.pbf --registrar <your-hex32>
```

### `update`

Check whether Geofabrik has published a newer snapshot for a region than
what is in your local catalog or the on-chain registry.

```sh
osm update germany
osm update china/henan
```

### `catalog`

List your local catalog: every region you have hosted or imported, with its
CID, MD5, and file path.

```sh
osm catalog
```

### `lookup`

Read registry entries directly from the chain. No wallet needed, read-only.
Query by region path, by parent (to get all subregions), or by CID (to find
which region holds a given storage CID).

```sh
osm lookup --region germany
osm lookup --parent us
osm lookup --cid zDvZRwzkyRRYCsLTGLBuEdjYcWXEBNP1wUh559sDYr2PN98ajwT3
```

## Common workflows

### Host and register a region (the full pipeline)

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

# 2. Fetch it from Logos Storage and verify
osm fetch china/henan \
  --cid <cid-from-lookup> \
  --checksum <md5-from-lookup>

# 3. Re-host it under your own account
osm host china/henan --registrar <your-hex32>
osm register china/henan --registrar <your-hex32> --sign
```

### Bulk host multiple regions

```sh
osm host-bulk --regions germany,france,spain --registrar <your-hex32>
osm register-bulk --regions germany,france,spain --registrar <your-hex32> --sign
```

### Import a local PBF file

```sh
osm import germany ./germany-latest.osm.pbf --registrar <your-hex32>
osm register germany --registrar <your-hex32> --sign
```

### Browse the registry without a wallet

```sh
osm lookup --region germany           # one region
osm lookup --parent us                # all US subregions
osm regions                           # the full 72-region set
osm catalog                           # your local hosted regions
```

### Check for updates

```sh
osm update germany                    # is there a newer Geofabrik snapshot?
```
