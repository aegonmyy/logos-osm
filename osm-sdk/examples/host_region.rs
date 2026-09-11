//! Host one region end-to-end against real Geofabrik + a local Codex node:
//!
//! ```text
//! cargo run -p logos-osm --example host_region -- kenya http://127.0.0.1:8080
//! ```
//!
//! Downloads the region PBF (streaming), verifies it against Geofabrik's
//! published MD5, stores it in Logos Storage, prints the snapshot and the
//! ready-to-submit registration transaction.

use logos_osm::registry::build_register_region;
use logos_osm::storage::CodexStorage;
use logos_osm::OsmClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let region = std::env::args().nth(1).unwrap_or_else(|| "kenya".into());
    let storage_url =
        std::env::args().nth(2).unwrap_or_else(|| "http://127.0.0.1:8080".into());

    let client = OsmClient::new(CodexStorage::new(storage_url), "osm-cache");
    let snap = client.host_region(&region).await?;
    client.catalog_record_hosted(&snap)?;

    println!("region   : {}", snap.region);
    println!("bytes    : {}", snap.bytes);
    println!("version  : {} (YYYYMMDD)", snap.version);
    println!("md5      : {}", hex::encode(snap.checksum));
    println!("cid      : {}", snap.cid);
    println!("local    : {}", snap.path.display());

    // The registration tx for this snapshot (the wallet submits it).
    let program_id = osm_registry::osm_registry_id();
    let registrar =
        lee_core::account::AccountId::new([0x11; 32]); // demo registrar
    let built = build_register_region(&program_id, &registrar, &snap.registration(None));
    println!("tx words : {} u32 instruction words", built.instruction.len());
    Ok(())
}
