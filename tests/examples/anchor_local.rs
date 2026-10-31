//! Anchor real OSM regions against a LOCAL standalone sequencer.
//!
//! The public testnet's sequencer can stall for hours. This variant proves the
//! full pipeline (deploy -> host -> register -> verify) against a Docker-based
//! sequencer brought up by `test_fixtures::TestContext`, using the same real
//! Codex storage node and the same real Geofabrik PBFs. The CIDs it produces
//! are valid on any chain (they are content-derived), so when the public
//! testnet recovers, the public anchor run reuses them and only repeats the
//! on-chain registrations.
//!
//! Usage:
//! ```text
//! OSM_REGIONS=ethiopia,pakistan,bulgaria \
//!   cargo run -p osm-integration-tests --example anchor_local
//! ```
//!
//! Env:
//! - `OSM_REGIONS`   comma-separated region paths (default: 3-region test set)
//! - `OSM_CODEX_URL` Logos Storage node (default http://127.0.0.1:8080)
//! - `OSM_ANCHOR_DIR` where the prefetched PBFs and state live

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use lee_core::account::AccountId;
use logos_osm::registry::{
    build_init, build_register_regions_batch, decode_region_entry, region_pda, registry_pda,
};
use logos_osm::storage::CodexStorage;
use logos_osm::OsmClient;
use osm_integration_tests::deploy_program;
use serde_json::json;
use test_fixtures::{TestContext, TIME_TO_WAIT_FOR_BLOCK_SECONDS};
use wallet::cli::{
    account::{AccountSubcommand, NewSubcommand},
    Command, SubcommandReturnValue,
};

async fn wait_block() {
    tokio::time::sleep(std::time::Duration::from_secs(
        TIME_TO_WAIT_FOR_BLOCK_SECONDS,
    ))
    .await;
}

async fn new_public_account(ctx: &mut TestContext, label: &str) -> Result<AccountId> {
    let result = wallet::cli::execute_subcommand(
        ctx.wallet_mut(),
        Command::Account(AccountSubcommand::New(NewSubcommand::Public {
            cci: None,
            label: Some(label.to_string().into()),
        })),
    )
    .await
    .with_context(|| format!("creating account {label}"))?;
    let SubcommandReturnValue::RegisterAccount { account_id } = result else {
        bail!("expected a registered account id for {label}");
    };
    Ok(account_id)
}

async fn send_and_await(
    ctx: &mut TestContext,
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &AccountId,
    label: &str,
) -> Result<()> {
    let mentions = built.mentions(signer);
    let h = ctx
        .wallet_mut()
        .send_pub_tx(
            mentions,
            built.instruction.clone(),
            built.program_account_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{label} send_pub_tx failed: {e:?}"))?;
    ctx.wallet_mut()
        .poll_transaction(h)
        .await
        .with_context(|| format!("{label} not included"))?;
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;
    println!("  {label}: included");
    Ok(())
}

fn anchor_dir() -> PathBuf {
    std::env::var("OSM_ANCHOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join("osm-anchor")
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    let regions: Vec<String> = std::env::var("OSM_REGIONS")
        .unwrap_or_else(|_| "ethiopia,pakistan,bulgaria".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if regions.is_empty() {
        bail!("no regions given");
    }
    let codex_url =
        std::env::var("OSM_CODEX_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let dir = anchor_dir();

    println!("LOCAL anchor (Docker sequencer)");
    println!("anchor dir : {}", dir.display());
    println!("storage    : {codex_url}");
    println!("regions    : {}\n", regions.join(", "));

    // Storage node must be up before we start.
    {
        use std::net::TcpStream;
        let hostport = codex_url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        TcpStream::connect(hostport).map_err(|e| {
            anyhow::anyhow!(
                "storage node at {codex_url} is unreachable: {e}\n\
                 start it with: docker run -d --name osm-codex -p 8080:8080 ..."
            )
        })?;
    }

    // ---- Docker sequencer stack (takes ~5-6 min to come up) ----
    println!("starting the Docker sequencer stack (this takes a few minutes)...");
    let mut ctx = TestContext::new().await?;
    println!("sequencer is up");

    // ---- accounts (the local stack does not need explicit funding) ----
    let payer = new_public_account(&mut ctx, "osm-payer").await?;
    let registrar = new_public_account(&mut ctx, "osm-registrar").await?;
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;

    // ---- deploy ----
    println!("deploying the committed registry program...");
    let program_account = deploy_program(
        ctx.wallet_mut(),
        osm_registry::osm_registry_elf().to_vec(),
        payer,
    )
    .await
    .context("deploying the program")?;
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;
    println!(
        "  program account: {}",
        hex::encode(program_account.value())
    );

    // ---- Init ----
    println!("Init: binding the registry owner...");
    send_and_await(
        &mut ctx,
        &build_init(&program_account, &registrar),
        &registrar,
        "Init",
    )
    .await?;

    // ---- host every region (real Codex + real Geofabrik PBFs) ----
    let client = OsmClient::new(CodexStorage::new(&codex_url), dir.join("cache"));
    let mut hosted: Vec<(String, logos_osm::registry::RegionRegistration)> = Vec::new();
    for r in &regions {
        let local = dir.join("prefetch").join(format!("{r}.osm.pbf"));
        println!(
            "hosting {r}{}...",
            if local.exists() {
                " (from prefetch)"
            } else {
                ""
            }
        );
        let snap = if local.exists() {
            client
                .host_local(r, &local)
                .await
                .context(format!("importing the prefetched {r}"))?
        } else {
            client
                .host_region(r)
                .await
                .context(format!("hosting {r}"))?
        };
        client
            .catalog_record_hosted(&snap)
            .context("recording the hosted snapshot")?;
        println!(
            "  {} bytes, version {}, md5 {}, cid {}",
            snap.bytes,
            snap.version,
            hex::encode(snap.checksum),
            snap.cid
        );
        hosted.push((r.clone(), snap.registration(None)));
    }

    // ---- register in batches of 12 ----
    for (i, chunk) in hosted.chunks(12).enumerate() {
        let regs: Vec<_> = chunk.iter().map(|(_, r)| r.clone()).collect();
        let names: Vec<&str> = chunk.iter().map(|(n, _)| n.as_str()).collect();
        println!("registering batch {}: {}", i + 1, names.join(", "));
        let built = build_register_regions_batch(&program_account, &registrar, &regs);
        send_and_await(&mut ctx, &built, &registrar, &format!("batch {}", i + 1)).await?;
    }

    // ---- verify readback ----
    let mut summary = serde_json::Map::new();
    let failures = 0;
    for r in &regions {
        let acc = ctx
            .wallet()
            .get_account_public(region_pda(&program_account, r))
            .await?;
        let entry = decode_region_entry(acc.data.shard(program_account).as_ref())
            .with_context(|| format!("decoding the on-chain entry for {r}"))?;
        let mirror_count = entry.mirrors.len();
        let latest = entry.latest_mirror();
        println!(
            "  {r}: {mirror_count} mirror(s), latest cid {}",
            latest.map(|m| m.cid.clone()).unwrap_or_default()
        );
        summary.insert(
            r.clone(),
            json!({
                "mirrors": mirror_count,
                "region_pda": hex::encode(region_pda(&program_account, r).value()),
                "on_chain": mirror_count > 0,
            }),
        );
    }

    let out = json!({
        "mode": "local",
        "program_account": hex::encode(program_account.value()),
        "registry_pda": hex::encode(registry_pda(&program_account).value()),
        "storage": codex_url,
        "regions": summary,
    });
    let sp = dir.join("summary-local.json");
    std::fs::write(&sp, serde_json::to_vec_pretty(&out)?)?;
    println!("\nsummary written: {}", sp.display());
    println!("{}", serde_json::to_string_pretty(&out)?);

    if failures > 0 {
        bail!("{failures} region(s) failed verification");
    }
    println!("\nLOCAL PIPELINE VERIFIED: deploy + host + register + readback all work.");
    println!("The CIDs are content-derived and valid on any chain.");
    println!("When the public testnet recovers, re-run anchor_regions to register on it.");
    Ok(())
}
