//! Integration-test crate: end-to-end runs against a standalone LEZ
//! sequencer (Docker) and real storage nodes.

use std::time::Duration;

use anyhow::{Context, Result};
use common::transaction::LeeTransaction;
use lee::public_transaction::{Message, WitnessSet};
use lee::{AccountId, FeeDeclaration, PublicTransaction};
use lee_core::account::ProgramShardSelector;
use lee_core::native_token;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
use sequencer_service_rpc::{RpcClient as _, SequencerClientBuilder};
use wallet::{WalletCore, program_facades::program_loader::ProgramLoader};

/// Deploy `bytecode` through the program loader, returning the **account id**
/// the deployed program lives at.
///
/// Under the v0.3 program model a program's address is chosen at deploy time
/// and is not a function of its bytecode, so this return value is what every
/// later call needs: PDAs are derived from it, and it is the transaction
/// target. It is not the image id.
///
/// `payer` must be an existing funded account: a freshly claimed account
/// cannot pay for its own claim.
pub async fn deploy_program(
    wallet_core: &mut WalletCore,
    bytecode: Vec<u8>,
    payer: AccountId,
) -> Result<AccountId> {
    let segment_count = bytecode.len().div_ceil(MAX_SEGMENT_DATA_LEN);
    let header = wallet_core.create_new_account_public(None).0;
    let segments: Vec<AccountId> = std::iter::repeat_with(|| wallet_core.create_new_account_public(None).0)
        .take(segment_count)
        .collect();
    wallet_core.store_persistent_data()?;

    ProgramLoader(wallet_core)
        .deploy(header, &segments, bytecode, true, Some(payer))
        .await
}

/// Enough native tokens for a deploy plus dozens of region registrations.
pub const GENESIS_FUND_AMOUNT: u128 = 1_000_000_000_000;

/// The public endpoint flaps (upstream sequencer down behind an nginx that
/// 502s for minutes). Retry a read through those transient failures only; a
/// genuine rejection still fails immediately.
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
                        || m.contains("error sending request")
                        || m.contains("connection");
                    if transient && attempt < 400 {
                        if attempt % 30 == 1 {
                            eprintln!("[{}] transient (attempt {attempt}): {m}", $label);
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

/// Fund `target` from the public testnet's published genesis account.
///
/// Under v0.3 a deploy is a sequence of fee-bearing transactions, so the payer
/// has to hold tokens before it can deploy anything, and a freshly claimed
/// account holds none. The public testnet ships no faucet; the one route to
/// funds is the genesis state (`testnet_initial_state`), which publishes the
/// signing keys of two public accounts that hold balances.
///
/// Those keys are public, so that balance is shared with everyone. Funding a
/// dedicated account once and spending from it means our nonce is ours alone;
/// the alternative, transacting from the genesis account directly, races every
/// other builder on the network.
pub async fn fund_account_from_genesis(
    wallet: &mut WalletCore,
    sequencer_url: &str,
    target: AccountId,
    amount: u128,
) -> Result<()> {
    let before = net_retry!(wallet.get_account_public(target), "fund-balance")
        .data
        .native_balance()
        .unwrap_or(0);
    if before >= amount {
        return Ok(());
    }

    let source = testnet_initial_state::initial_pub_accounts_private_keys()
        .into_iter()
        .next()
        .context("the genesis state declares no funded account")?;
    let nonce = net_retry!(wallet.get_accounts_nonces(&[source.account_id]), "fund-nonce")[0];

    let shard_selectors = vec![
        ProgramShardSelector::native_balance(source.account_id),
        ProgramShardSelector::native_balance(target),
    ];
    let message = Message::try_new_with_fees(
        native_token::NATIVE_TOKEN_PROGRAM_ID,
        shard_selectors,
        vec![nonce],
        native_token::Instruction::Transfer { amount },
        // Generous ceiling: the fee market on a fresh chain has no history to
        // price against, and an under-priced declaration fails the screen.
        FeeDeclaration::new(source.account_id, 2_000_000, 0, u128::MAX >> 1),
    )
    .context("building the genesis transfer")?;
    let witness_set = WitnessSet::for_message(&message, &[&source.pub_sign_key]);
    let tx = LeeTransaction::Public(PublicTransaction::new(message, witness_set));

    let client = SequencerClientBuilder::default().build(sequencer_url.to_string())?;
    let hash = client
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("submitting the genesis transfer: {e}"))?;
    println!("funding {target}: tx {hash:?}");

    // Wait for inclusion, then confirm the balance actually moved. A timeout
    // here is a real failure: the deploy that follows would fail the fee
    // screen with a far less legible error.
    for _ in 0..60 {
        if wallet.poll_transaction(hash).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let after = net_retry!(wallet.get_account_public(target), "fund-balance-after")
        .data
        .native_balance()
        .unwrap_or(0);
    anyhow::ensure!(
        after > before,
        "the genesis transfer did not land: {target} still holds {after}"
    );
    Ok(())
}