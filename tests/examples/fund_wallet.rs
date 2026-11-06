//! Fund the existing personal wallet at ~/.logos-osm/wallet from the testnet
//! genesis accounts — with diagnostics and adaptive amounts. Opens the
//! EXISTING wallet (does NOT re-create it), so the mnemonic stays intact.
//!
//! Why this exists: `fund_account_from_genesis` is hardcoded to the FIRST
//! genesis account and a fixed amount. As of 2026-11-06 that account holds
//! ~0.9T atomic units — less than the 1T `GENESIS_FUND_AMOUNT` — so its
//! transfers fail block inclusion for insufficient funds (the sequencer still
//! accepts them, they just never land). The testnet declares TWO genesis
//! accounts, so here we check both and fund from whichever can cover the
//! transfer; if neither covers the full amount, we take half of the richest
//! account's remaining balance (leaving the other half for other testnet
//! users).
//!
//! Run: cargo run -p osm-integration-tests --example fund_wallet
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use common::transaction::LeeTransaction;
use lee::public_transaction::{Message, WitnessSet};
use lee::{AccountId, FeeDeclaration, PrivateKey, PublicTransaction};
use lee_core::account::ProgramShardSelector;
use lee_core::native_token;
use osm_integration_tests::GENESIS_FUND_AMOUNT;
use sequencer_service_rpc::{RpcClient as _, SequencerClientBuilder};
use testnet_initial_state::initial_pub_accounts_private_keys;
use wallet::config::{SequencerConnectionData, WalletConfigOverrides};
use wallet::WalletCore;

const PRIMARY_ACCOUNT_ID: &str = "5Ui7XuhyF7sJvreKyyv5ax3M5bzhyKJaNaus8hSCUVyD";
const SEQUENCER: &str = "https://testnet.lez.logos.co";

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(".logos-osm")
        .join("wallet");

    let overrides = WalletConfigOverrides {
        sequencer: Some(SequencerConnectionData {
            sequencer_addr: SEQUENCER.parse().unwrap(),
            basic_auth: None,
        }),
        seq_tx_poll_max_blocks: Some(200),
        seq_poll_max_retries: Some(2000),
        seq_poll_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let mut wallet = WalletCore::new_update_chain(
        dir.join("config.json"),
        dir.join("storage"),
        Some(overrides),
    )
    .context("opening existing wallet — run create_wallet first")?;

    let target = AccountId::from_str(PRIMARY_ACCOUNT_ID).context("parsing primary account id")?;

    println!("syncing wallet to latest block...");
    wallet.sync_to_latest_block().await.context("sync")?;

    // ---- diagnostics: both genesis accounts + the target ----
    let genesis = initial_pub_accounts_private_keys();
    let mut best = 0usize;
    let mut best_bal = 0u128;
    for (i, acc) in genesis.iter().enumerate() {
        let bal = wallet.get_account_balance(acc.account_id).await?;
        let nonce = wallet.get_accounts_nonces(&[acc.account_id]).await?[0];
        println!(
            "genesis[{i}] {}: balance={bal} nonce={nonce:?}",
            acc.account_id
        );
        if bal > best_bal {
            best_bal = bal;
            best = i;
        }
    }

    let target_bal = wallet.get_account_balance(target).await?;
    println!("target  {target}: balance={target_bal}");
    println!("desired amount: {GENESIS_FUND_AMOUNT}");

    if target_bal >= GENESIS_FUND_AMOUNT {
        println!("ALREADY FUNDED: target balance {target_bal} >= {GENESIS_FUND_AMOUNT}.");
        return Ok(());
    }
    if best_bal == 0 {
        anyhow::bail!("all genesis accounts are empty — cannot fund");
    }

    // Full amount if the richest account covers it; otherwise half of what
    // it has left, leaving the other half for other testnet users.
    let amount = if best_bal >= GENESIS_FUND_AMOUNT {
        GENESIS_FUND_AMOUNT
    } else {
        best_bal / 2
    };
    println!("funding {amount} from genesis[{best}] (it holds {best_bal})");

    let source = &genesis[best];
    let mut ok = false;
    for attempt in 1..=3u32 {
        match transfer_from(
            &mut wallet,
            SEQUENCER,
            source.account_id,
            &source.pub_sign_key,
            target,
            amount,
        )
        .await
        {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) => {
                eprintln!("funding attempt {attempt}/3 failed: {e:#}");
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    }

    // Re-query: a late-landing tx may have funded it even if the loop errored.
    let final_bal = wallet.get_account_balance(target).await?;
    println!("final target balance: {final_bal}");
    if ok || final_bal >= amount {
        println!("SUCCESS: wallet {target} funded with {amount}.");
    } else {
        anyhow::bail!("funding failed after 3 attempts; final balance {final_bal}");
    }
    Ok(())
}

/// Transfer `amount` from a genesis-held account to `target`, waiting for
/// inclusion. Mirrors `fund_account_from_genesis` but with a parameterized
/// source account.
async fn transfer_from(
    wallet: &mut WalletCore,
    sequencer_url: &str,
    source_id: AccountId,
    source_key: &PrivateKey,
    target: AccountId,
    amount: u128,
) -> Result<()> {
    let nonce = wallet.get_accounts_nonces(&[source_id]).await?[0];
    let shard_selectors = vec![
        ProgramShardSelector::native_balance(source_id),
        ProgramShardSelector::native_balance(target),
    ];
    let message = Message::try_new_with_fees(
        native_token::NATIVE_TOKEN_PROGRAM_ID,
        shard_selectors,
        vec![nonce],
        native_token::Instruction::Transfer { amount },
        // Generous ceiling: the fee market has no history to price against,
        // and an under-priced declaration fails the screen.
        FeeDeclaration::new(source_id, 2_000_000, 0, u128::MAX >> 1),
    )
    .context("building the transfer")?;
    let witness_set = WitnessSet::for_message(&message, &[source_key]);
    let tx = LeeTransaction::Public(PublicTransaction::new(message, witness_set));

    let client = SequencerClientBuilder::default().build(sequencer_url)?;
    let hash = client
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("submitting the transfer: {e}"))?;
    println!("transfer {amount} {source_id} -> {target}: tx {hash:?}");

    // Wait for inclusion (up to 120s), then confirm the balance moved.
    for _ in 0..60 {
        if wallet.poll_transaction(hash).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let after = wallet
        .get_account_public(target)
        .await?
        .data
        .native_balance()
        .unwrap_or(0);
    anyhow::ensure!(
        after >= amount,
        "the transfer did not land: {target} holds {after}"
    );
    Ok(())
}
