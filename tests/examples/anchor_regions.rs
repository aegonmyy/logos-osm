//! Anchor real OSM regions on the live LEZ testnet.
//!
//! The full pipeline in one run: fund a payer from the published genesis
//! account, deploy the committed registry program, download each region from
//! Geofabrik, verify it against the published MD5, store it in Logos Storage
//! (a real Codex node, not the in-memory backend), and register it on-chain.
//! Everything is resumable: a state file records the program account, the
//! transacting accounts, and every region already anchored, so a re-run skips
//! what is done.
//!
//! Usage:
//! ```text
//! OSM_REGIONS=ethiopia,pakistan cargo run -p osm-integration-tests \
//!   --example anchor_regions
//! ```
//!
//! Env:
//! - `OSM_REGIONS`      comma-separated region paths (default: a 3-region test set)
//! - `OSM_CODEX_URL`    Logos Storage node (default http://127.0.0.1:8080)
//! - `OSM_ANCHOR_DIR`   persistent state (default ~/osm-anchor)
//! - `OSM_TESTNET_URL`  sequencer (default https://testnet.lez.logos.co)
//!
//! Output: `OSM_ANCHOR_DIR/summary.json` — program account, registry PDA, and
//! per-region {cid, md5, version, region PDA}. That file is the artifact to
//! show anyone evaluating the registry: CIDs resolve to real hosted bytes,
//! checksums match Geofabrik, entries are on a public chain.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use lee_core::account::AccountId;
use logos_osm::registry::{
    build_init, build_register_regions_batch, decode_region_entry, region_pda, registry_pda,
};
use logos_osm::storage::CodexStorage;
use logos_osm::OsmClient;
use osm_integration_tests::{deploy_program, fund_account_from_genesis, GENESIS_FUND_AMOUNT};
use serde_json::json;
use wallet::config::{SequencerConnectionData, WalletConfigOverrides};
use wallet::WalletCore;

/// The public endpoint flaps (upstream sequencer down behind an nginx that
/// 502s for minutes). Retry a read through those transient failures only.
macro_rules! net_retry {
    ($op:expr, $label:expr) => {{
        let mut attempt = 0;
        loop {
            attempt += 1;
            match $op.await {
                Ok(v) => break v,
                Err(e) => {
                    let m = e.to_string();
                    let transient = m.contains("502")
                        || m.contains("timed out")
                        || m.contains("connection closed");
                    if transient && attempt < 12 {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                    anyhow::bail!("{} failed after {attempt} attempts: {m}", $label);
                }
            }
        }
    }};
}

#[derive(Default)]
struct AnchorState {
    program_account_hex: Option<String>,
    payer_hex: Option<String>,
    registrar_hex: Option<String>,
    /// region path -> (cid, md5 hex, version)
    done: BTreeMap<String, (String, String, u32)>,
}

fn anchor_dir() -> PathBuf {
    let dir = std::env::var("OSM_ANCHOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join("osm-anchor")
        });
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[allow(clippy::field_reassign_with_default)]
fn load_state() -> AnchorState {
    let p = anchor_dir().join("anchor-state.json");
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return AnchorState::default();
    };
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    let mut s = AnchorState::default();
    s.program_account_hex = v["program_account_hex"].as_str().map(String::from);
    s.payer_hex = v["payer_hex"].as_str().map(String::from);
    s.registrar_hex = v["registrar_hex"].as_str().map(String::from);
    if let Some(obj) = v["done"].as_object() {
        for (k, e) in obj {
            s.done.insert(
                k.clone(),
                (
                    e["cid"].as_str().unwrap_or_default().into(),
                    e["md5"].as_str().unwrap_or_default().into(),
                    e["version"].as_u64().unwrap_or_default() as u32,
                ),
            );
        }
    }
    s
}

fn save_state(s: &AnchorState) -> Result<()> {
    let done: serde_json::Map<String, _> = s
        .done
        .iter()
        .map(|(k, (cid, md5, ver))| (k.clone(), json!({ "cid": cid, "md5": md5, "version": ver })))
        .collect();
    let v = json!({
        "program_account_hex": s.program_account_hex,
        "payer_hex": s.payer_hex,
        "registrar_hex": s.registrar_hex,
        "done": done,
    });
    let p = anchor_dir().join("anchor-state.json");
    std::fs::write(&p, serde_json::to_vec_pretty(&v)?)?;
    Ok(())
}

fn parse_account(hex: &str) -> Result<AccountId> {
    let bytes: [u8; 32] = hex::decode(hex.trim())?.try_into().map_err(|v: Vec<u8>| {
        anyhow::anyhow!("account id must be 32 bytes hex, got {}", v.len())
    })?;
    Ok(AccountId::new(bytes))
}

async fn new_account(wallet: &mut WalletCore, label: &str) -> Result<AccountId> {
    use wallet::cli::{
        account::{AccountSubcommand, NewSubcommand},
        Command, SubcommandReturnValue,
    };
    let result = wallet::cli::execute_subcommand(
        wallet,
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
    wallet: &mut WalletCore,
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &AccountId,
    label: &str,
) -> Result<()> {
    let mentions = built.mentions(signer);
    let h = wallet
        .send_pub_tx(
            mentions,
            built.instruction.clone(),
            built.program_account_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{label} send failed: {e:?}"))?;
    net_retry!(wallet.poll_transaction(h), format!("{label}-poll"));
    net_retry!(wallet.sync_to_latest_block(), format!("{label}-sync"));
    println!("  {label}: included");
    Ok(())
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
    let testnet_url =
        std::env::var("OSM_TESTNET_URL").unwrap_or_else(|_| "https://testnet.lez.logos.co".into());

    let dir = anchor_dir();
    println!("anchor dir : {}", dir.display());
    println!("storage    : {codex_url}");
    println!("testnet    : {testnet_url}");
    println!("regions    : {}\n", regions.join(", "));

    // A quick health check on the storage node before touching the chain.
    // (No HTTP client crate in this crate's deps; the CLI's SDK has one but a
    // plain TCP connect is enough to know the node is up.)
    {
        use std::net::TcpStream;
        let hostport = codex_url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        match TcpStream::connect(hostport) {
            Ok(_) => {}
            Err(e) => bail!("storage node at {codex_url} is unreachable: {e}"),
        }
    }

    // ---- wallet (fresh per anchor dir; state file makes later runs resume) ----
    let overrides = WalletConfigOverrides {
        sequencer: Some(SequencerConnectionData {
            sequencer_addr: testnet_url.parse().unwrap(),
            basic_auth: None,
        }),
        seq_tx_poll_max_blocks: Some(200),
        seq_poll_max_retries: Some(2000),
        seq_poll_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let (mut wallet, _mnemonic) = WalletCore::new_init_storage(
        dir.join("config.json"),
        dir.join("storage"),
        Some(overrides),
        "osm-anchor",
    )?;
    net_retry!(wallet.sync_to_latest_block(), "initial-sync");

    let mut state = load_state();

    // ---- payer + registrar, funded from the published genesis account ----
    // The transacting accounts must belong to THIS wallet. A previous run's
    // account IDs in the state file are useless here: the wallet has no keys
    // for them and cannot sign. Always create fresh ones (the genesis
    // funding is idempotent, and the on-chain deployment and hosted regions
    // survive any wallet change).
    let payer = new_account(&mut wallet, "osm-payer").await?;
    state.payer_hex = Some(hex::encode(payer.value()));
    save_state(&state)?;
    let registrar = new_account(&mut wallet, "osm-registrar").await?;
    state.registrar_hex = Some(hex::encode(registrar.value()));
    save_state(&state)?;
    net_retry!(wallet.sync_to_latest_block(), "post-accounts-sync");

    println!("funding payer + registrar from the published genesis account...");
    fund_account_from_genesis(&mut wallet, &testnet_url, payer, GENESIS_FUND_AMOUNT)
        .await
        .context("funding the payer")?;
    fund_account_from_genesis(&mut wallet, &testnet_url, registrar, GENESIS_FUND_AMOUNT)
        .await
        .context("funding the registrar")?;
    net_retry!(wallet.sync_to_latest_block(), "post-funding-sync");

    // ---- deploy (once) ----
    let program_account = match &state.program_account_hex {
        Some(h) => parse_account(h)?,
        None => {
            println!("deploying the committed registry program...");
            let a = deploy_program(
                &mut wallet,
                osm_registry::osm_registry_elf().to_vec(),
                payer,
            )
            .await
            .context("deploying the program")?;
            state.program_account_hex = Some(hex::encode(a.value()));
            save_state(&state)?;
            net_retry!(wallet.sync_to_latest_block(), "post-deploy-sync");
            println!("  program account: {}", hex::encode(a.value()));
            a
        }
    };

    // ---- Init (once): an empty registry shard means Init has not run ----
    let reg_acc = net_retry!(
        wallet.get_account_public(registry_pda(&program_account)),
        "read-registry"
    );
    if reg_acc.data.shard(program_account).is_empty() {
        println!("Init: binding the registry owner...");
        send_and_await(
            &mut wallet,
            &build_init(&program_account, &registrar),
            &registrar,
            "Init",
        )
        .await?;
    } else {
        println!("registry already initialized");
    }

    // ---- host every pending region (download -> verify MD5 -> store) ----
    // A region already downloaded to <anchor>/prefetch/<region>.osm.pbf is
    // imported through the local path (same MD5 verification, same storage
    // upload, same registration) instead of being downloaded again.
    let pending: Vec<&String> = regions
        .iter()
        .filter(|r| !state.done.contains_key(*r))
        .collect();
    let mut hosted: Vec<(String, logos_osm::registry::RegionRegistration)> = Vec::new();
    if !pending.is_empty() {
        let client = OsmClient::new(CodexStorage::new(&codex_url), dir.join("cache"));
        for r in &pending {
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
            state.done.insert(
                r.to_string(),
                (snap.cid.clone(), hex::encode(snap.checksum), snap.version),
            );
            // The registration the chain gets: the region, the CID, Geofabrik's
            // checksum, the version, and a timestamp.
            hosted.push(((**r).clone(), snap.registration(None)));
            save_state(&state)?; // after each region: a crash costs one region
        }
    } else {
        println!("all regions already hosted");
    }

    // ---- register in batches of 12 ----
    // The registration list includes BOTH newly hosted regions and regions
    // from a previous run (state file) that have not yet been registered
    // on-chain. Checking the on-chain entry avoids re-registering regions
    // that are already there (idempotent re-runs).
    let mut to_register: Vec<(String, logos_osm::registry::RegionRegistration)> = hosted;

    // Add regions from the state file that are hosted but have no on-chain
    // mirror yet (a previous run hosted them but the chain stalled before
    // the registration could be submitted).
    for r in &regions {
        if to_register.iter().any(|(n, _)| n == r) {
            continue;
        }
        if let Some((cid, md5, version)) = state.done.get(r) {
            // Check whether this region already has an on-chain entry.
            let acc = net_retry!(
                wallet.get_account_public(region_pda(&program_account, r)),
                format!("precheck-{r}")
            );
            let has_entry = !acc.data.shard(program_account).is_empty();
            if !has_entry {
                let checksum: [u8; 16] = hex::decode(md5)?.try_into().map_err(|v: Vec<u8>| {
                    anyhow::anyhow!("md5 must be 16 bytes, got {}", v.len())
                })?;
                to_register.push((
                    r.clone(),
                    logos_osm::registry::RegionRegistration {
                        region: r.clone(),
                        cid: cid.clone(),
                        source_url: String::new(),
                        checksum,
                        version: *version,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    },
                ));
            }
        }
    }

    if to_register.is_empty() {
        println!("all regions already registered on-chain");
    }

    for (i, chunk) in to_register.chunks(12).enumerate() {
        let regs: Vec<_> = chunk.iter().map(|(_, r)| r.clone()).collect();
        let names: Vec<&str> = chunk.iter().map(|(n, _)| n.as_str()).collect();
        println!("registering batch {}: {}", i + 1, names.join(", "));
        let built = build_register_regions_batch(&program_account, &registrar, &regs);
        send_and_await(&mut wallet, &built, &registrar, &format!("batch {}", i + 1)).await?;
    }

    // ---- verify every requested region reads back from the chain ----
    let mut summary = serde_json::Map::new();
    let mut failures = 0;
    for r in &regions {
        let Some((cid, md5, version)) = state.done.get(r) else {
            failures += 1;
            println!("!! {r}: never hosted");
            continue;
        };
        let acc = net_retry!(
            wallet.get_account_public(region_pda(&program_account, r)),
            format!("read-{r}")
        );
        let entry = decode_region_entry(acc.data.shard(program_account).as_ref())
            .with_context(|| format!("decoding the on-chain entry for {r}"))?;
        let ok = entry.mirrors.iter().any(|m| &m.cid == cid);
        if !ok {
            failures += 1;
            println!("!! {r}: on chain but the CID is missing (registered by someone else, or the batch failed)");
        }
        summary.insert(
            r.clone(),
            json!({
                "cid": cid,
                "md5": md5,
                "version": version,
                "region_pda": hex::encode(region_pda(&program_account, r).value()),
                "on_chain": ok,
            }),
        );
    }

    let out = json!({
        "program_account": hex::encode(program_account.value()),
        "registry_pda": hex::encode(registry_pda(&program_account).value()),
        "testnet": testnet_url,
        "storage": codex_url,
        "regions": summary,
    });
    let sp = dir.join("summary.json");
    std::fs::write(&sp, serde_json::to_vec_pretty(&out)?)?;
    println!("\nsummary written: {}", sp.display());
    println!("{}", serde_json::to_string_pretty(&out)?);

    if failures > 0 {
        bail!("{failures} region(s) failed verification");
    }
    Ok(())
}
