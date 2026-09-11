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
use logos_osm::registry::{
    build_init, build_register_region, build_register_regions_batch, to_identities,
};
use logos_osm::regions::{Level, REGIONS};
use logos_osm::{OsmClient, UpdateStatus};
use logos_osm::storage::{CodexStorage, MemoryStorage};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "osm", version, about = "OpenStreetMap distribution on Logos: host, register, fetch, import")]
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
    /// Import a local PBF into the catalog (structure-validated).
    Import { region: String, pbf: PathBuf },
    /// Check whether a newer snapshot is published for a region.
    Update { region: String },
    /// List the local catalog.
    Catalog,
}

/// A built LEZ transaction, printed as JSON for the wallet to submit.
#[derive(Serialize)]
struct TxCmd {
    /// The deployed program the tx targets.
    program_id_hex: String,
    /// Account ids (32-byte hex each) in the order the guest asserts on.
    accounts_hex: Vec<String>,
    /// Wallet identity kinds, parallel to `accounts_hex`:
    /// "sign" for the registrar, "read" for the rest.
    signing: Vec<&'static str>,
    /// risc0-serde instruction words (hex, 4 bytes each).
    instruction_hex: Vec<String>,
}

fn hex32(bytes: [u8; 32]) -> String {
    hex::encode(bytes)
}

fn account_hex(id: &lee_core::account::AccountId) -> String {
    hex32(*id.value())
}

fn print_tx(label: &str, built: &logos_osm::registry::OsmTxBuilt, signer: &lee_core::account::AccountId) {
    let ids = to_identities(built, signer);
    let signing: Vec<&'static str> = ids
        .iter()
        .map(|id| match id {
            wallet::AccountIdentity::Public(_) => "sign",
            _ => "read",
        })
        .collect();
    let cmd = TxCmd {
        program_id_hex: {
            let words = osm_registry::osm_registry_id();
            words.iter().map(|w| format!("{w:08x}")).collect()
        },
        accounts_hex: built.accounts.iter().map(account_hex).collect(),
        signing,
        instruction_hex: built.instruction.iter().map(|w| format!("{w:08x}")).collect(),
    };
    println!("--- {label} transaction (submit via the wallet) ---");
    println!("{}", serde_json::to_string_pretty(&cmd).unwrap());
}

fn parse_account(hex_str: &str) -> Result<lee_core::account::AccountId> {
    let bytes: [u8; 32] = hex::decode(hex_str.trim())?
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("account id must be 32 bytes hex, got {} bytes", v.len()))?;
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
    let program_id = osm_registry::osm_registry_id();

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
                    let built = build_register_region(&program_id, &registrar, &snap.registration(None));
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
                regions.split(',').map(str::trim).map(String::from).collect()
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
                    .with_context(|| format!("{region} is not in the local catalog (host it first)"))?;
                let snap = logos_osm::HostedSnapshot {
                    region: rec.region,
                    cid: rec.cid.context("catalog record has no cid")?,
                    checksum: rec.md5,
                    version: rec.version,
                    bytes: 0,
                    path: rec.path,
                };
                let built = build_register_region(&program_id, &registrar, &snap.registration(None));
                print_tx("RegisterRegion", &built, &registrar);
                Ok(())
            })?;
        }
        Cmd::RegisterBulk { regions, registrar } => {
            let registrar = parse_account(&registrar)?;
            let wanted: Vec<String> =
                regions.split(',').map(str::trim).map(String::from).collect();
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
                let built = build_register_regions_batch(&program_id, &registrar, &regs);
                print_tx("RegisterRegionsBatch", &built, &registrar);
                Ok(())
            })?;
        }
        Cmd::Init { owner } => {
            let owner = parse_account(&owner)?;
            let built = build_init(&program_id, &owner);
            print_tx("Init", &built, &owner);
        }
        Cmd::Fetch {
            region,
            cid,
            checksum,
        } => {
            let checksum: [u8; 16] = hex::decode(checksum.trim())?
                .try_into()
                .map_err(|v: Vec<u8>| anyhow::anyhow!("checksum must be 16 bytes hex, got {}", v.len()))?;
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
        Cmd::Import { region, pbf } => {
            with_client!(|client: OsmClient<_>| async move {
                let summary = client.import_local(&region, &pbf).await?;
                println!(
                    "imported {} ({} bytes, {} blobs, md5 {})",
                    summary.region,
                    summary.bytes,
                    summary.blobs,
                    hex::encode(summary.md5)
                );
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
                        println!("{region}: update available (local {local} -> published {published})");
                    }
                    UpdateStatus::NotImported { published } => {
                        println!("{region}: not imported locally (published {published})");
                    }
                }
                Ok(())
            })?;
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
