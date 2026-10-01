//! Public LEZ testnet deployment + lifecycle (supportability criterion).
//!
//! Deploys the OSM-registry program to the official public testnet (default
//! `https://testnet.lez.logos.co`, override with `OSM_TESTNET_URL`) and
//! exercises the on-chain lifecycle there: Init, one RegisterRegion, and a
//! small RegisterRegionsBatch, reading state back from the chain after every
//! step (a green run is itself proof of inclusion). Mirrors
//! `osm_registry_live.rs` (standalone sequencer) but replaces
//! `test_fixtures::TestContext` with a `WalletCore` pointed at the public
//! network — the connection pattern proven by the sibling vault repo's
//! testnet test, including retrying through the public endpoint's
//! transient 502s.
//!
//! Ignored by default (hits the public network; block production is
//! intermittent). Run with:
//!   RISC0_DEV_MODE=0 cargo test -p osm-integration-tests \
//!     --test osm_registry_testnet -- --ignored --nocapture

use std::time::Duration;

use anyhow::{bail, Context, Result};
use lee::ProgramId;
use lee_core::account::AccountId;
use logos_osm::registry::{
    build_init, build_register_region, build_register_regions_batch, decode_region_entry,
    decode_registry_state, region_pda, registry_pda, RegionRegistration,
};
use osm_integration_tests::{deploy_program, fund_account_from_genesis, GENESIS_FUND_AMOUNT};
use wallet::cli::{
    account::{AccountSubcommand, NewSubcommand},
    Command, SubcommandReturnValue,
};
use wallet::config::{SequencerConnectionData, WalletConfigOverrides};
use wallet::WalletCore;

/// The documented deterministic program id (README / docs). The testnet
/// deployment must reproduce it — same committed ELF, same PDA derivation.
const EXPECTED_PROGRAM_ID_HEX: &str =
    "c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79";

fn testnet_url() -> String {
    std::env::var("OSM_TESTNET_URL").unwrap_or_else(|_| "https://testnet.lez.logos.co".to_owned())
}

/// The public endpoint flaps (upstream sequencer down for minutes behind an
/// nginx that 502s). Retry a network op through those transient failures.
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
                        || m.contains("rejected")
                        || m.contains("timed out")
                        || m.contains("error sending request")
                        || m.contains("connection");
                    if transient && attempt < 1200 {
                        if attempt % 30 == 1 {
                            eprintln!("[{}] transient (attempt {attempt}): {m}; retrying", $label);
                        }
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }};
}

async fn send_and_await(
    wallet: &mut WalletCore,
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &AccountId,
    program_account: AccountId,
    label: &str,
) -> Result<()> {
    // v0.3: shard-scoped account mentions + borsh instruction + the target
    // program's account id.
    let _ = program_account;
    let mentions = built.mentions(signer);
    let h = wallet
        .send_pub_tx(
            mentions,
            built.instruction.clone(),
            built.program_account_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{label} send_pub_tx failed: {e:?}"))?;
    net_retry!(wallet.poll_transaction(h), &format!("{label}-poll"));
    net_retry!(wallet.sync_to_latest_block(), &format!("{label}-sync"));
    println!("{label}: tx 0x{}", hex32(h.0));
    Ok(())
}

fn hex32(h: [u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// A `[u32; 8]` program id (big-endian words) as hex.
fn hex_program(p: &ProgramId) -> String {
    p.iter()
        .flat_map(|w| w.to_be_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn registration(region: &str, cid: &str, version: u32, timestamp: u64) -> RegionRegistration {
    RegionRegistration {
        region: region.into(),
        cid: cid.into(),
        source_url: logos_osm::regions::by_path(region)
            .map(|r| r.source_url().to_string())
            .unwrap_or_default(),
        checksum: [0x5a; 16],
        version,
        timestamp,
    }
}

async fn new_public_account(wallet: &mut WalletCore, label: &str) -> Result<AccountId> {
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

#[tokio::test]
#[ignore = "hits the live public LEZ testnet; block production is intermittent"]
async fn osm_registry_lifecycle_on_public_testnet() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("osm-testnet-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;

    // Patient polling: the public testnet produces blocks slowly.
    let overrides = WalletConfigOverrides {
        sequencers: Some(vec![SequencerConnectionData {
            sequencer_addr: testnet_url().parse().unwrap(),
            basic_auth: None,
        }]),
        seq_tx_poll_max_blocks: Some(200),
        seq_poll_max_retries: Some(2000),
        seq_poll_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let (mut wallet, _mnemonic) = WalletCore::new_init_storage(
        dir.join("config.json"),
        dir.join("storage"),
        dir.join("statistics.json"),
        Some(overrides),
        "testpw",
    )
    .await?;

    let start = net_retry!(wallet.sync_to_latest_block(), "initial-sync");
    println!("testnet: {} (start block {start})", testnet_url());

    // The image id is the bytecode identity, checked against the committed
    // artifact. Under v0.3 it is NOT the program's address.
    let image_id = osm_registry::osm_registry_id();
    let got = hex_program(&image_id);
    assert_eq!(
        got, EXPECTED_PROGRAM_ID_HEX,
        "committed image id drifted from docs — rebuild via tools/guest-builder and re-pin"
    );

    // 1) Deploy the committed guest artifact and keep the address it lands at.
    // v0.3 chooses the program's account at deploy time, and PDAs and the
    // transaction target both key off that account.
    //
    // The payer must already hold tokens. A deploy is a sequence of
    // fee-bearing transactions (one per bytecode segment), and a freshly
    // created account has a zero balance, so the later segments are rejected
    // with `Incorrect fee` once the fee market charges for them. The v0.3
    // deploy helper says as much: "a freshly-claimed account can't pay for
    // its own claim".
    let payer = new_public_account(&mut wallet, "osm-testnet-payer").await?;
    net_retry!(wallet.sync_to_latest_block(), "post-payer-sync");
    // The payer has to hold tokens before the first fee-bearing transaction.
    // A freshly claimed account holds none, and the public testnet has no
    // faucet, so fund it from the published genesis account. See
    // docs/TESTNET_FUNDING.md for how that was established.
    fund_account_from_genesis(&mut wallet, &testnet_url(), payer, GENESIS_FUND_AMOUNT)
        .await
        .context("funding the deploy payer from the genesis account")?;
    net_retry!(wallet.sync_to_latest_block(), "post-funding-sync");
    let program_account = net_retry!(
        deploy_program(
            &mut wallet,
            osm_registry::osm_registry_elf().to_vec(),
            payer
        ),
        "deploy"
    );
    net_retry!(wallet.sync_to_latest_block(), "post-deploy-sync");
    println!("deployed program at account {program_account}");

    // 2) Owner + one registrar. Each of them sends transactions (Init,
    //    RegisterRegion), and under v0.3 the signer pays its own fee when no
    //    explicit payer is given, so both need a balance too.
    let owner = new_public_account(&mut wallet, "osm-testnet-owner").await?;
    let registrar = new_public_account(&mut wallet, "osm-testnet-registrar").await?;
    net_retry!(wallet.sync_to_latest_block(), "post-accounts-sync");
    fund_account_from_genesis(&mut wallet, &testnet_url(), owner, GENESIS_FUND_AMOUNT)
        .await
        .context("funding the owner")?;
    fund_account_from_genesis(&mut wallet, &testnet_url(), registrar, GENESIS_FUND_AMOUNT)
        .await
        .context("funding the registrar")?;
    net_retry!(wallet.sync_to_latest_block(), "post-funding-accounts-sync");

    // 3) Init.
    send_and_await(
        &mut wallet,
        &build_init(&program_account, &owner),
        &owner,
        program_account,
        "Init",
    )
    .await?;
    let reg_state: osm_core::RegistryState = {
        let acc = wallet
            .get_account_public(registry_pda(&program_account))
            .await?;
        decode_registry_state(acc.data.shard(program_account).as_ref())
            .context("decoding RegistryState after Init")?
    };
    assert_eq!(reg_state.initialized, 1);
    assert_eq!(reg_state.owner, *owner.value());
    println!(
        "Init: owner bound, registry PDA {}",
        hex32(*registry_pda(&program_account).value())
    );

    // 4) Register one region (germany) + a small batch, read back the entries.
    let germany_pda = region_pda(&program_account, "germany");
    let t = 1_790_000_000u64;
    send_and_await(
        &mut wallet,
        &build_register_region(
            &program_account,
            &registrar,
            &registration("germany", "cid-testnet-germany", 20260821, t),
        ),
        &registrar,
        program_account,
        "RegisterRegion(germany)",
    )
    .await?;
    let entry = {
        let acc = wallet.get_account_public(germany_pda).await?;
        decode_region_entry(acc.data.shard(program_account).as_ref()).context("decoding germany")?
    };
    assert_eq!(entry.region, "germany");
    assert_eq!(entry.parent, "");
    assert_eq!(entry.level, 0);
    assert_eq!(entry.mirrors.len(), 1);
    assert_eq!(entry.mirrors[0].cid, "cid-testnet-germany");
    assert_eq!(entry.mirrors[0].registrar, *registrar.value());
    println!(
        "germany registered; region PDA {}",
        hex32(*germany_pda.value())
    );

    let batch = vec![
        registration("france", "cid-testnet-france", 20260821, t + 1),
        registration("us/california", "cid-testnet-ca", 20260821, t + 2),
    ];
    send_and_await(
        &mut wallet,
        &build_register_regions_batch(&program_account, &registrar, &batch),
        &registrar,
        program_account,
        "RegisterRegionsBatch",
    )
    .await?;
    for r in &batch {
        let acc = wallet
            .get_account_public(region_pda(&program_account, &r.region))
            .await?;
        let e = decode_region_entry(acc.data.shard(program_account).as_ref())
            .with_context(|| format!("decoding {} after batch", r.region))?;
        assert_eq!(e.mirrors.len(), 1);
        assert_eq!(e.mirrors[0].cid, r.cid);
    }
    // california is a subregion: table-derived parent/level.
    let acc = wallet
        .get_account_public(region_pda(&program_account, "us/california"))
        .await?;
    let e = decode_region_entry(acc.data.shard(program_account).as_ref())?;
    assert_eq!(e.parent, "us");
    assert_eq!(e.level, 1);

    let reg_state: osm_core::RegistryState = {
        let acc = wallet
            .get_account_public(registry_pda(&program_account))
            .await?;
        decode_registry_state(acc.data.shard(program_account).as_ref())
            .context("decoding final RegistryState")?
    };
    assert_eq!(reg_state.region_count(), 3);
    assert_eq!(reg_state.registration_count, 3);
    println!(
        "testnet lifecycle OK: 3 regions, 3 registrations; program {got}; registry PDA {}",
        hex32(*registry_pda(&program_account).value())
    );

    Ok(())
}
