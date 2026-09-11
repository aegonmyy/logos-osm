//! End-to-end registry lifecycle against a standalone LEZ sequencer.
//!
//! Proves, on a real chain (Docker stack via `test_fixtures::TestContext`):
//! 1. the committed guest artifact deploys and its pinned program id derives
//!    the expected PDAs;
//! 2. `Init` binds the owner;
//! 3. `RegisterRegion` is permissionless — **three distinct registrars**
//!    mirror the same region (the prize's adoption bar) and the chain keeps
//!    the full version history (newer snapshots append);
//! 4. `RegisterRegionsBatch` registers several regions in one tx;
//! 5. querying the region/registry accounts decodes back exactly what was
//!    registered (mirrors, registrars, versions, counters).
//!
//! Run with the stack up:
//! ```text
//! cargo test -p osm-integration-tests --test osm_registry_live -- --nocapture
//! ```

use std::time::Duration;

use anyhow::{bail, Context, Result};
use lee_core::account::AccountId;
use logos_osm::registry::{
    build_init, build_register_region, build_register_regions_batch, decode_region_entry,
    decode_registry_state, region_pda, registry_pda, to_identities, RegionRegistration,
};
use test_fixtures::{TestContext, TIME_TO_WAIT_FOR_BLOCK_SECONDS};
use wallet::cli::{
    account::{AccountSubcommand, NewSubcommand},
    Command, SubcommandReturnValue,
};
use wallet::AccountIdentity;

/// Map a built tx's accounts to wallet identities: the signer signs, the
/// PDAs are read-only/unsigned.
fn to_live(
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &AccountId,
) -> (Vec<AccountIdentity>, Vec<u32>) {
    (to_identities(built, signer), built.instruction.clone())
}

async fn wait_block() {
    tokio::time::sleep(Duration::from_secs(TIME_TO_WAIT_FOR_BLOCK_SECONDS)).await;
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

async fn submit(
    ctx: &mut TestContext,
    built: &logos_osm::registry::OsmTxBuilt,
    signer: &AccountId,
    what: &str,
) -> Result<()> {
    let (identities, instruction) = to_live(built, signer);
    let program_id = osm_registry::osm_registry_id();
    let h = ctx
        .wallet_mut()
        .send_pub_tx(identities, instruction, program_id)
        .await
        .map_err(|e| anyhow::anyhow!("{what} send_pub_tx failed: {e:?}"))?;
    ctx.wallet_mut()
        .poll_transaction(h)
        .await
        .with_context(|| format!("{what} not included"))?;
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;
    Ok(())
}

fn registration(region: &str, cid: &str, version: u32, timestamp: u64) -> RegionRegistration {
    RegionRegistration {
        region: region.into(),
        cid: cid.into(),
        checksum: [0x5a; 16],
        version,
        timestamp,
    }
}

#[tokio::test]
#[ignore = "requires Docker + the standalone LEZ sequencer stack"]
async fn osm_registry_full_lifecycle_on_sequencer() -> Result<()> {
    let mut ctx = TestContext::new().await?;
    let program_id = osm_registry::osm_registry_id();

    // 1) Deploy the committed guest artifact.
    let elf = osm_registry::osm_registry_elf();
    let elf_path = std::env::temp_dir().join(format!("osm-registry-{}.elf", std::process::id()));
    std::fs::write(&elf_path, elf)?;
    wallet::cli::execute_subcommand(
        ctx.wallet_mut(),
        Command::DeployProgram {
            binary_filepath: elf_path.clone(),
        },
    )
    .await
    .context("deploying osm-registry program")?;
    let _ = std::fs::remove_file(&elf_path);
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;

    // 2) Owner + THREE distinct registrars (the adoption bar is >= 3
    //    distinct accounts mirroring a region).
    let owner = new_public_account(&mut ctx, "osm-owner").await?;
    let registrar_a = new_public_account(&mut ctx, "osm-registrar-a").await?;
    let registrar_b = new_public_account(&mut ctx, "osm-registrar-b").await?;
    let registrar_c = new_public_account(&mut ctx, "osm-registrar-c").await?;
    wait_block().await;
    ctx.wallet_mut().sync_to_latest_block().await?;

    // 3) Init.
    submit(&mut ctx, &build_init(&program_id, &owner), &owner, "Init").await?;
    let reg_state: osm_core::RegistryState = {
        let acc = ctx
            .wallet()
            .get_account_public(registry_pda(&program_id))
            .await?;
        decode_registry_state(acc.data.as_ref()).context("decoding RegistryState after Init")?
    };
    assert_eq!(reg_state.initialized, 1);
    assert_eq!(reg_state.owner, *owner.value());
    assert_eq!(reg_state.region_count, 0);

    // 4) Registrar A mirrors germany.
    let germany_pda = region_pda(&program_id, "germany");
    submit(
        &mut ctx,
        &build_register_region(
            &program_id,
            &registrar_a,
            &registration("germany", "cid-germany-a", 20260801, 1_787_000_000),
        ),
        &registrar_a,
        "RegisterRegion(germany, A)",
    )
    .await?;
    let entry = {
        let acc = ctx.wallet().get_account_public(germany_pda).await?;
        decode_region_entry(acc.data.as_ref()).context("decoding germany after A")?
    };
    assert_eq!(entry.region, "germany");
    // parent/level come from the embedded table, not the client.
    assert_eq!(entry.parent, "");
    assert_eq!(entry.level, 0);
    assert_eq!(entry.mirrors.len(), 1);
    assert_eq!(entry.mirrors[0].registrar, *registrar_a.value());
    assert_eq!(entry.mirrors[0].cid, "cid-germany-a");
    assert_eq!(entry.mirrors[0].version, 20260801);

    // 5) Registrar B mirrors the SAME region with a NEWER snapshot version —
    //    the append-only history must keep both.
    submit(
        &mut ctx,
        &build_register_region(
            &program_id,
            &registrar_b,
            &registration("germany", "cid-germany-b", 20260815, 1_787_864_000),
        ),
        &registrar_b,
        "RegisterRegion(germany, B)",
    )
    .await?;
    let entry = {
        let acc = ctx.wallet().get_account_public(germany_pda).await?;
        decode_region_entry(acc.data.as_ref()).context("decoding germany after B")?
    };
    assert_eq!(entry.mirrors.len(), 2, "history must be append-only");
    assert_eq!(entry.registrars().len(), 2);
    assert_eq!(entry.latest_mirror().unwrap().version, 20260815);
    assert_eq!(entry.latest_mirror().unwrap().cid, "cid-germany-b");
    // Timestamp-ordered.
    assert!(entry.mirrors[0].timestamp <= entry.mirrors[1].timestamp);

    // 6) Registrar C brings the region to the adoption bar (>= 3 distinct).
    submit(
        &mut ctx,
        &build_register_region(
            &program_id,
            &registrar_c,
            &registration("germany", "cid-germany-c", 20260821, 1_788_000_000),
        ),
        &registrar_c,
        "RegisterRegion(germany, C)",
    )
    .await?;
    let entry = {
        let acc = ctx.wallet().get_account_public(germany_pda).await?;
        decode_region_entry(acc.data.as_ref()).context("decoding germany after C")?
    };
    assert_eq!(entry.registrars().len(), 3, "adoption bar reached");
    assert_eq!(entry.latest_mirror().unwrap().version, 20260821);

    // 7) Batch: registrar C mirrors three regions in one transaction.
    let batch = vec![
        registration("france", "cid-france-c", 20260821, 1_788_000_100),
        registration("us/california", "cid-california-c", 20260821, 1_788_000_101),
        registration("kenya", "cid-kenya-c", 20260821, 1_788_000_102),
    ];
    submit(
        &mut ctx,
        &build_register_regions_batch(&program_id, &registrar_c, &batch),
        &registrar_c,
        "RegisterRegionsBatch",
    )
    .await?;
    for r in &batch {
        let acc = ctx
            .wallet()
            .get_account_public(region_pda(&program_id, &r.region))
            .await?;
        let e = decode_region_entry(acc.data.as_ref())
            .with_context(|| format!("decoding {} after batch", r.region))?;
        assert_eq!(e.mirrors.len(), 1);
        assert_eq!(e.mirrors[0].cid, r.cid);
        assert_eq!(e.region, r.region);
    }
    // The california subregion entry carries its table parent/level.
    let acc = ctx
        .wallet()
        .get_account_public(region_pda(&program_id, "us/california"))
        .await?;
    let e = decode_region_entry(acc.data.as_ref())?;
    assert_eq!(e.parent, "us");
    assert_eq!(e.level, 1);

    // 8) Registry counters: 4 regions ever registered (germany, france,
    //    us/california, kenya), 6 registrations total (3 + 3).
    let reg_state: osm_core::RegistryState = {
        let acc = ctx
            .wallet()
            .get_account_public(registry_pda(&program_id))
            .await?;
        decode_registry_state(acc.data.as_ref()).context("decoding final RegistryState")?
    };
    assert_eq!(reg_state.region_count, 4);
    assert_eq!(reg_state.registration_count, 6);

    Ok(())
}
