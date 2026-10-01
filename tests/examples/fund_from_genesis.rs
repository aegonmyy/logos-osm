//! Fund our own testnet account from a published genesis account.
//!
//! The public testnet's genesis is `testnet_initial_state`, which publishes
//! the signing keys of two funded public accounts. This creates a fresh
//! account of ours, transfers to it from one of those, and checks the balance
//! landed.
//!
//! Why fund our own rather than spend from the genesis account directly: the
//! genesis key is public, so anyone can spend it, and a shared account means a
//! shared nonce. A dedicated account removes both.
//!
//! Run: cargo run -p osm-integration-tests --example fund_from_genesis

use anyhow::{bail, Context, Result};
use common::transaction::LeeTransaction;
use lee::public_transaction::{Message, WitnessSet};
use lee::{FeeDeclaration, PublicTransaction};
use lee_core::account::ProgramShardSelector;
use lee_core::native_token;
use sequencer_service_rpc::{RpcClient as _, SequencerClientBuilder};
use std::time::Duration;
use testnet_initial_state::{initial_pub_accounts_private_keys, initial_public_user_accounts};
use wallet::cli::{
    account::{AccountSubcommand, NewSubcommand},
    Command, SubcommandReturnValue,
};
use wallet::config::{SequencerConnectionData, WalletConfigOverrides};
use wallet::WalletCore;

const TESTNET: &str = "https://testnet.lez.logos.co";
/// 1,000 LGO, ample for a deploy plus dozens of registrations.
const FUND_AMOUNT: u128 = 1_000_000_000_000;

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("osm-fund");
    std::fs::create_dir_all(&dir)?;

    // 1. A wallet of ours, so the deploy can sign from it afterwards.
    let overrides = WalletConfigOverrides {
        sequencers: Some(vec![SequencerConnectionData {
            sequencer_addr: TESTNET.parse().unwrap(),
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
        "osm-fund",
    )
    .await?;
    // No chain sync: account creation, nonce lookup and submission all talk to
    // the node directly, and a full replay of the chain is minutes of work
    // this flow does not need.

    let target = match wallet::cli::execute_subcommand(
        &mut wallet,
        Command::Account(AccountSubcommand::New(NewSubcommand::Public {
            cci: None,
            label: Some("osm-funded".into()),
        })),
    )
    .await
    .context("creating our account")?
    {
        SubcommandReturnValue::RegisterAccount { account_id } => account_id,
        other => bail!("unexpected result creating account: {other:?}"),
    };
    println!("our account: {target}");

    let before = wallet
        .get_account_public(target)
        .await?
        .data
        .native_balance()
        .unwrap_or(0);
    println!("balance before: {before}");
    if before > 0 {
        println!("already funded; nothing to do");
        return Ok(());
    }

    // 2. The genesis account and its published signing key.
    let source = initial_pub_accounts_private_keys()
        .into_iter()
        .next()
        .context("no genesis account")?;
    let funded = initial_public_user_accounts()
        .into_iter()
        .find(|a| a.account_id == source.account_id)
        .context("genesis account has no declared balance")?;
    println!(
        "genesis account: {} ({})",
        source.account_id, funded.balance
    );
    if funded.balance < FUND_AMOUNT {
        bail!(
            "genesis balance {} is below the {} we want to move",
            funded.balance,
            FUND_AMOUNT
        );
    }

    // 3. Build and sign the transfer with the genesis key, then submit it.
    let nonce = wallet.get_accounts_nonces(&[source.account_id]).await?[0];
    let shard_selectors = vec![
        ProgramShardSelector::native_balance(source.account_id),
        ProgramShardSelector::native_balance(target),
    ];
    let message = Message::try_new_with_fees(
        native_token::NATIVE_TOKEN_PROGRAM_ID,
        shard_selectors,
        vec![nonce],
        native_token::Instruction::Transfer {
            amount: FUND_AMOUNT,
        },
        FeeDeclaration::new(source.account_id, 2_000_000, 0, u128::MAX >> 1),
    )
    .context("building the transfer")?;
    let witness_set = WitnessSet::for_message(&message, &[&source.pub_sign_key]);
    let tx = LeeTransaction::Public(PublicTransaction::new(message, witness_set));

    let client = SequencerClientBuilder::default().build(TESTNET.to_string())?;
    let hash = client
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("submitting: {e}"))?;
    println!("submitted: {hash:?}");

    // 4. Wait for it, then confirm the balance moved.
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
    println!("balance after: {after}");
    if after <= before {
        bail!("the transfer did not land: still {after}");
    }
    println!("\nfunded: {target}");
    Ok(())
}
