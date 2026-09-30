//! `osm` — the OpenStreetMap distribution lifecycle CLI.
//!
//! Covers the full flow: discover regions, host (download from Geofabrik,
//! verify the published MD5, store in Logos Storage), build the LEZ
//! registration transaction for the wallet to submit, fetch a snapshot from
//! storage (Geofabrik fallback), import locally, and check for updates.
//!
//! On-chain submission follows the wallet pattern: the CLI **builds** the
//! transaction (accounts + risc0-serde instruction words, printed as JSON)
//! and the wallet submits it — the SDK stays transport-agnostic, and the
//! integration test performs the actual on-chain submission end to end.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use logos_osm::regions::{Level, REGIONS};
use logos_osm::registry::{
    RegionEntry, build_init, build_register_region, build_register_regions_batch, region_pda,
};
use sequencer_service_rpc::{RpcClient as _, SequencerClient, SequencerClientBuilder};
use logos_osm::storage::{CodexStorage, MemoryStorage};
use logos_osm::{OsmClient, UpdateStatus};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "osm",
    version,
    about = "OpenStreetMap distribution on Logos: host, register, fetch, import"
)]
struct Cli {
    /// Logos Storage (Codex) REST endpoint.
    #[arg(long, env = "OSM_STORAGE_URL", default_value = "http://127.0.0.1:8080")]
    storage_url: String,
    /// Use the in-memory storage backend (offline demo; nothing persists).
    #[arg(long)]
    memory_storage: bool,
    /// Download cache + catalog directory.
    #[arg(long, env = "OSM_CACHE", default_value = "osm-cache")]
    cache: PathBuf,
    /// Geofabrik base URL (mirror / fixture override).
    #[arg(long, env = "OSM_GEOFABRIK_URL", default_value = logos_osm::geofabrik::DEFAULT_BASE)]
    geofabrik: String,
    /// The deployed registry program's account id (base58), as returned by the
    /// deployment. Required for the commands that build a registration
    /// transaction: v0.3 addresses a program by its chosen account, not by the
    /// image id of its bytecode.
    #[arg(long, env = "OSM_PROGRAM_ACCOUNT")]
    program_account: Option<String>,
    /// LEZ sequencer JSON-RPC endpoint, used by `lookup` to read registry
    /// accounts. Read-only: a query needs no wallet.
    #[arg(
        long,
        env = "OSM_SEQUENCER",
        default_value = "https://testnet.lez.logos.co"
    )]
    sequencer: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the predefined, non-overlapping region set.
    Regions {
        /// Only subregions of this parent (e.g. `us`).
        #[arg(long)]
        parent: Option<String>,
        /// Filter by level (`country` or `subregion`).
        #[arg(long)]
        level: Option<String>,
    },
    /// Cross-check the closed set against Geofabrik's live index.
    Discover,
    /// Host one region: download, verify MD5, store in Logos Storage.
    /// Prints the snapshot + the LEZ registration transaction.
    Host {
        region: String,
        /// Registrar account id (32-byte hex) for the tx; the tx is printed
        /// for the wallet to submit.
        #[arg(long)]
        registrar: Option<String>,
    },
    /// Bulk host many regions with per-region opt-out.
    HostBulk {
        /// Comma-separated region paths (or `all` for the closed set).
        #[arg(long)]
        regions: String,
        /// Comma-separated region paths to skip.
        #[arg(long, default_value = "")]
        opt_out: String,
    },
    /// Build the LEZ registration tx for a hosted (catalog) region.
    Register {
        region: String,
        /// Registrar account id (32-byte hex).
        #[arg(long)]
        registrar: String,
    },
    /// Build a batch registration tx (bulk hosting on-chain).
    RegisterBulk {
        /// Comma-separated region paths.
        #[arg(long)]
        regions: String,
        /// Registrar account id (32-byte hex).
        #[arg(long)]
        registrar: String,
    },
    /// Build the registry `Init` tx (deploy-time, once).
    Init {
        /// Owner account id (32-byte hex).
        #[arg(long)]
        owner: String,
    },
    /// Fetch a registered snapshot from Logos Storage (Geofabrik fallback),
    /// verify the checksum, and import locally.
    Fetch {
        region: String,
        /// Storage CID from the registry.
        #[arg(long)]
        cid: String,
        /// Published MD5 (hex) from the registry.
        #[arg(long)]
        checksum: String,
    },
    /// Import a local PBF: validate + catalog it. With --registrar, run the
    /// full local-import workflow: verify against Geofabrik's published MD5,
    /// store in Logos Storage, and print the LEZ registration tx.
    Import {
        region: String,
        pbf: PathBuf,
        /// Registrar account id (32-byte hex): verify + store + print the
        /// registration tx (the full local-import workflow).
        #[arg(long)]
        registrar: Option<String>,
    },
    /// Check whether a newer snapshot is published for a region.
    Update { region: String },
    /// List the local catalog.
    Catalog,
    /// Read registry entries from the chain: one region, every subregion of a
    /// parent, or the region holding a given storage CID.
    Lookup {
        /// Look up one region by its Geofabrik path (e.g. `germany`).
        #[arg(long)]
        region: Option<String>,
        /// Look up every registered subregion of a parent path (e.g. `us`).
        #[arg(long)]
        parent: Option<String>,
        /// Find the region whose stored snapshot has this Logos Storage CID.
        #[arg(long)]
        cid: Option<String>,
    },
}

/// A built LEZ transaction, printed as JSON for the wallet to submit.
#[derive(Serialize)]
struct TxCmd {
    /// The image id of the registry bytecode (bytecode identity, for
    /// verifying the committed artifact). Not the program's address.
    image_id_hex: String,
    /// The deployed program's account id: the transaction target in v0.3.
    program_account_hex: String,
    /// Account ids (32-byte hex each) in the order the guest's plan expects.
    accounts_hex: Vec<String>,
    /// The account that must sign (its signature sets `is_authorized` in the
    /// guest's account metadata).
    signer_hex: String,
    /// borsh-encoded instruction (hex).
    instruction_hex: String,
    /// Instruction encoding marker.
    encoding: &'static str,
}

fn hex32(bytes: [u8; 32]) -> String {
    hex::encode(bytes)
}

fn account_hex(id: &lee_core::account::AccountId) -> String {
    hex32(*id.value())
}

fn print_tx(
    label: &str,
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &lee_core::account::AccountId,
) {
    let cmd = TxCmd {
        image_id_hex: {
            let words = osm_registry::osm_registry_id();
            words.iter().map(|w| format!("{w:08x}")).collect()
        },
        program_account_hex: account_hex(&built.program_account_id),
        accounts_hex: built.accounts.iter().map(account_hex).collect(),
        signer_hex: account_hex(signer),
        instruction_hex: hex::encode(&built.instruction),
        encoding: "borsh",
    };
    println!("--- {label} transaction (submit via the wallet) ---");
    println!("{}", serde_json::to_string_pretty(&cmd).unwrap());
}

/// Connect to the sequencer for read-only registry queries.
fn chain_client(url: &str) -> Result<SequencerClient> {
    SequencerClientBuilder::default()
        .build(url.to_string())
        .with_context(|| format!("--sequencer is not a usable URL: {url}"))
}

/// One region's on-chain entry, or `None` when nothing is registered for it.
async fn fetch_entry(
    client: &SequencerClient,
    program_account: &lee_core::account::AccountId,
    path: &str,
) -> Result<Option<RegionEntry>> {
    let account = client
        .get_account(region_pda(program_account, path))
        .await
        .with_context(|| format!("reading {path} from the sequencer"))?;
    let shard = account.data.shard(*program_account);
    if shard.as_ref().is_empty() {
        return Ok(None);
    }
    Ok(Some(
        borsh::from_slice(shard.as_ref()).with_context(|| format!("decoding {path}"))?,
    ))
}

/// Print one entry in a stable, greppable form.
fn print_entry(path: &str, entry: &RegionEntry) {
    println!(
        "{}\t{}\tlevel={}\tmirrors={}\tregistrars={}",
        entry.region,
        if entry.parent.is_empty() { "-" } else { &entry.parent },
        entry.level,
        entry.mirrors.len(),
        entry.registrars().len()
    );
    for m in &entry.mirrors {
        println!(
            "  {} v{} ts={} cid={} md5={} url={}",
            hex32(m.registrar),
            m.version,
            m.timestamp,
            m.cid,
            hex::encode(m.checksum),
            m.source_url
        );
    }
    let _ = path;
}

/// The configured program account, or an error naming what to set.
fn program_account(raw: Option<&str>) -> Result<lee_core::account::AccountId> {
    let raw = raw.ok_or_else(|| {
        anyhow::anyhow!(
            "no program account: pass --program-account <base58> (or set OSM_PROGRAM_ACCOUNT) \
             to the address the deployment returned"
        )
    })?;
    raw.parse()
        .map_err(|e| anyhow::anyhow!("--program-account must be a base58 account id: {e}"))
}

fn parse_account(hex_str: &str) -> Result<lee_core::account::AccountId> {
    let bytes: [u8; 32] = hex::decode(hex_str.trim())?
        .try_into()
        .map_err(|v: Vec<u8>| {
            anyhow::anyhow!("account id must be 32 bytes hex, got {} bytes", v.len())
        })?;
    Ok(lee_core::account::AccountId::new(bytes))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "osm=info".into()),
        )
        .init();
    let cli = Cli::parse();
    // Hoisted before the subcommand match moves `cli`.
    let program_account_arg = cli.program_account.clone();
    let sequencer_url = cli.sequencer.clone();

    // The client is generic over the storage backend; the CLI exposes the
    // real node and an in-memory mode for offline demos.
    macro_rules! with_client {
        ($body:expr) => {
            if cli.memory_storage {
                let client: OsmClient<MemoryStorage> =
                    OsmClient::new(MemoryStorage::new(), &cli.cache)
                        .with_geofabrik_base(&cli.geofabrik);
                let r: anyhow::Result<()> = $body(client).await;
                r
            } else {
                let client: OsmClient<CodexStorage> =
                    OsmClient::new(CodexStorage::new(&cli.storage_url), &cli.cache)
                        .with_geofabrik_base(&cli.geofabrik);
                let r: anyhow::Result<()> = $body(client).await;
                r
            }
        };
    }

    match cli.cmd {
        Cmd::Regions { parent, level } => {
            let level = match level.as_deref() {
                None => None,
                Some("country") => Some(Level::Country),
                Some("subregion") => Some(Level::Subregion),
                Some(other) => anyhow::bail!("unknown level {other:?} (country|subregion)"),
            };
            for r in REGIONS {
                if let Some(p) = &parent {
                    if r.parent != Some(p.as_str()) {
                        continue;
                    }
                }
                if let Some(l) = level {
                    if r.level != l {
                        continue;
                    }
                }
                println!("{}\t{}\t{}", r.path, r.continent, r.source_url());
            }
        }
        Cmd::Discover => {
            with_client!(|client: OsmClient<_>| async move {
                let available = client.discover().await?;
                println!("{} region(s) available upstream:", available.len());
                for info in &available {
                    println!("{}\t{}", info.region.path, info.pbf_url);
                }
                Ok(())
            })?;
        }
        Cmd::Host { region, registrar } => {
            with_client!(|client: OsmClient<_>| async move {
                let snap = client.host_region(&region).await?;
                client.catalog_record_hosted(&snap)?;
                println!(
                    "{}: {} bytes, version {}, md5 {}",
                    snap.region,
                    snap.bytes,
                    snap.version,
                    hex::encode(snap.checksum)
                );
                println!("storage cid: {}", snap.cid);
                if let Some(reg) = registrar {
                    let registrar = parse_account(&reg)?;
                    let built =
                        build_register_region(&program_account(program_account_arg.as_deref())?, &registrar, &snap.registration(None));
                    print_tx("RegisterRegion", &built, &registrar);
                } else {
                    println!("(pass --registrar <hex> to also print the registration tx)");
                }
                Ok(())
            })?;
        }
        Cmd::HostBulk { regions, opt_out } => {
            let wanted: Vec<String> = if regions == "all" {
                REGIONS.iter().map(|r| r.path.to_string()).collect()
            } else {
                regions
                    .split(',')
                    .map(str::trim)
                    .map(String::from)
                    .collect()
            };
            let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
            let opt_out: Vec<&str> = if opt_out.is_empty() {
                Vec::new()
            } else {
                opt_out.split(',').map(str::trim).collect()
            };
            with_client!(|client: OsmClient<_>| async move {
                let report = client.host_regions_bulk(&wanted, &opt_out).await?;
                for s in &report.hosted {
                    client.catalog_record_hosted(s)?;
                    println!(
                        "hosted {}\t{}\t{}\t{}",
                        s.region,
                        s.cid,
                        s.version,
                        hex::encode(s.checksum)
                    );
                }
                for s in &report.skipped {
                    println!("skipped {s}");
                }
                println!(
                    "{} hosted, {} skipped",
                    report.hosted.len(),
                    report.skipped.len()
                );
                Ok(())
            })?;
        }
        Cmd::Register { region, registrar } => {
            let registrar = parse_account(&registrar)?;
            with_client!(|client: OsmClient<_>| async move {
                let rec = client
                    .catalog()
                    .into_iter()
                    .find(|c| c.region == region)
                    .with_context(|| {
                        format!("{region} is not in the local catalog (host it first)")
                    })?;
                let snap = logos_osm::HostedSnapshot {
                    region: rec.region,
                    cid: rec.cid.context("catalog record has no cid")?,
                    checksum: rec.md5,
                    version: rec.version,
                    bytes: 0,
                    path: rec.path,
                };
                let built =
                    build_register_region(&program_account(program_account_arg.as_deref())?, &registrar, &snap.registration(None));
                print_tx("RegisterRegion", &built, &registrar);
                Ok(())
            })?;
        }
        Cmd::RegisterBulk { regions, registrar } => {
            let registrar = parse_account(&registrar)?;
            let wanted: Vec<String> = regions
                .split(',')
                .map(str::trim)
                .map(String::from)
                .collect();
            with_client!(|client: OsmClient<_>| async move {
                let catalog = client.catalog();
                let mut regs = Vec::new();
                for region in &wanted {
                    let rec = catalog
                        .iter()
                        .find(|c| &c.region == region)
                        .with_context(|| format!("{region} is not in the local catalog"))?;
                    let snap = logos_osm::HostedSnapshot {
                        region: rec.region.clone(),
                        cid: rec.cid.clone().context("catalog record has no cid")?,
                        checksum: rec.md5,
                        version: rec.version,
                        bytes: 0,
                        path: rec.path.clone(),
                    };
                    regs.push(snap.registration(None));
                }
                let built = build_register_regions_batch(&program_account(program_account_arg.as_deref())?, &registrar, &regs);
                print_tx("RegisterRegionsBatch", &built, &registrar);
                Ok(())
            })?;
        }
        Cmd::Init { owner } => {
            let owner = parse_account(&owner)?;
            let built = build_init(&program_account(program_account_arg.as_deref())?, &owner);
            print_tx("Init", &built, &owner);
        }
        Cmd::Fetch {
            region,
            cid,
            checksum,
        } => {
            let checksum: [u8; 16] =
                hex::decode(checksum.trim())?
                    .try_into()
                    .map_err(|v: Vec<u8>| {
                        anyhow::anyhow!("checksum must be 16 bytes hex, got {}", v.len())
                    })?;
            with_client!(|client: OsmClient<_>| async move {
                let outcome = client.fetch_snapshot(&region, &cid, &checksum).await?;
                let summary = client.import_local(&region, &outcome.path).await?;
                println!(
                    "fetched {} ({} bytes, {} blobs, md5 verified)",
                    summary.region, summary.bytes, summary.blobs
                );
                Ok(())
            })?;
        }
        Cmd::Import {
            region,
            pbf,
            registrar,
        } => {
            let reg = registrar.map(|r| parse_account(&r)).transpose()?;
            with_client!(|client: OsmClient<_>| async move {
                if let Some(registrar) = reg {
                    // Full local-import workflow: verify upstream + store + tx.
                    let snap = client.host_local(&region, &pbf).await?;
                    client.catalog_record_hosted(&snap)?;
                    println!(
                        "hosted local {} ({} bytes, md5 verified against Geofabrik)",
                        snap.region, snap.bytes
                    );
                    println!("storage cid: {}", snap.cid);
                    let built =
                        build_register_region(&program_account(program_account_arg.as_deref())?, &registrar, &snap.registration(None));
                    print_tx("RegisterRegion", &built, &registrar);
                } else {
                    let summary = client.import_local(&region, &pbf).await?;
                    println!(
                        "imported {} ({} bytes, {} blobs, md5 {})",
                        summary.region,
                        summary.bytes,
                        summary.blobs,
                        hex::encode(summary.md5)
                    );
                }
                Ok(())
            })?;
        }
        Cmd::Update { region } => {
            with_client!(|client: OsmClient<_>| async move {
                match client.update_check(&region).await? {
                    UpdateStatus::UpToDate { local, published } => {
                        println!("{region}: up to date (local {local}, published {published})");
                    }
                    UpdateStatus::UpdateAvailable { local, published } => {
                        println!(
                            "{region}: update available (local {local} -> published {published})"
                        );
                    }
                    UpdateStatus::NotImported { published } => {
                        println!("{region}: not imported locally (published {published})");
                    }
                }
                Ok(())
            })?;
        }
        Cmd::Lookup {
            region,
            parent,
            cid,
        } => {
            let program_account = program_account(program_account_arg.as_deref())?;
            let client = chain_client(&sequencer_url)?;
            let selected = [region.is_some(), parent.is_some(), cid.is_some()]
                .iter()
                .filter(|x| **x)
                .count();
            if selected != 1 {
                anyhow::bail!("pass exactly one of --region, --parent, or --cid");
            }

            if let Some(path) = region {
                match fetch_entry(&client, &program_account, &path).await? {
                    Some(entry) => print_entry(&path, &entry),
                    None => println!("{path}\tnot registered"),
                }
            } else if let Some(parent_path) = parent {
                let children = logos_osm::registry::children_of(&parent_path);
                if children.is_empty() {
                    anyhow::bail!("{parent_path} has no subregions in the predefined set");
                }
                for child in children {
                    match fetch_entry(&client, &program_account, child.path).await? {
                        Some(entry) => print_entry(child.path, &entry),
                        None => println!("{}\tnot registered", child.path),
                    }
                }
            } else if let Some(wanted) = cid {
                // The chain cannot be enumerated by CID, and the region set is
                // closed, so a CID search walks the set and reads each entry.
                let mut hits = 0;
                for r in REGIONS {
                    if let Some(entry) = fetch_entry(&client, &program_account, r.path).await? {
                        if entry.mirrors.iter().any(|m| m.cid == wanted) {
                            print_entry(r.path, &entry);
                            hits += 1;
                        }
                    }
                }
                if hits == 0 {
                    println!("{wanted}\tno region on chain carries this CID");
                }
            }
        }
        Cmd::Catalog => {
            with_client!(|client: OsmClient<_>| async move {
                for rec in client.catalog() {
                    println!(
                        "{}\tv{}\t{}\t{}",
                        rec.region,
                        rec.version,
                        rec.cid.as_deref().unwrap_or("-"),
                        rec.path.display()
                    );
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}
