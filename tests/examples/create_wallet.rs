//! Create a dedicated LEZ wallet for personal Logos use.
//!
//! Generates a fresh keypair, registers a named account, funds it from the
//! public testnet's genesis account (the only source of tokens — there is no
//! faucet), and prints:
//!   - the account ID (hex) — this is the value for the osm app's REGISTRAR
//!     field and any address someone uses to send you LEZ
//!   - the recovery mnemonic — WRITE IT DOWN; it regenerates the keypair
//!     if the wallet files are lost
//!
//! The wallet persists at ~/.logos-osm/wallet/ (config.json + storage/).
//!
//! Run: cargo run -p osm-integration-tests --example create_wallet
use std::time::Duration;

use anyhow::{Context, Result};
use osm_integration_tests::{fund_account_from_genesis, GENESIS_FUND_AMOUNT};
use wallet::cli::{
    account::{AccountSubcommand, NewSubcommand},
    Command, SubcommandReturnValue,
};
use wallet::config::{SequencerConnectionData, WalletConfigOverrides};
use wallet::WalletCore;

#[tokio::main]
async fn main() -> Result<()> {
    // Persistent wallet location alongside the osm state.
    let dir = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(".logos-osm")
        .join("wallet");
    std::fs::create_dir_all(&dir)?;

    let overrides = WalletConfigOverrides {
        sequencer: Some(SequencerConnectionData {
            sequencer_addr: "https://testnet.lez.logos.co".parse().unwrap(),
            basic_auth: None,
        }),
        seq_tx_poll_max_blocks: Some(200),
        seq_poll_max_retries: Some(2000),
        seq_poll_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let (mut wallet, mnemonic) = WalletCore::new_init_storage(
        dir.join("config.json"),
        dir.join("storage"),
        Some(overrides),
        "testpw",
    )?;

    // Save the mnemonic IMMEDIATELY, before any network step. An earlier
    // version printed it only at the end, so a funding failure lost the
    // phrase entirely (it is not in the storage file — only derived keys are).
    let mnemonic_path = dir.join("mnemonic.txt");
    std::fs::write(&mnemonic_path, mnemonic.to_string())
        .context("writing mnemonic file")?;
    println!();
    println!(
        "=== RECOVERY MNEMONIC (saved to {}) — KEEP SECRET ===",
        mnemonic_path.display()
    );
    println!("    {mnemonic}");
    println!();

    // A dedicated labelled account — "primary" is the one for everything.
    let label = "primary";
    let result = wallet::cli::execute_subcommand(
        &mut wallet,
        Command::Account(AccountSubcommand::New(NewSubcommand::Public {
            cci: None,
            label: Some(label.to_string().into()),
        })),
    )
    .await
    .with_context(|| format!("creating account {label}"))?;
    let SubcommandReturnValue::RegisterAccount { account_id } = result else {
        anyhow::bail!("expected a registered account id for {label}");
    };

    // Sync + fund from genesis (no faucet — the genesis keys are the only
    // source; funding once means the nonce is ours alone rather than racing
    // every other builder transacting from genesis directly).
    wallet
        .sync_to_latest_block()
        .await
        .context("initial sync")?;
    // Fund from genesis with retry. The sequencer flakes on tx submission /
    // inclusion, and `fund_account_from_genesis` is idempotent (it re-checks
    // the balance and re-fetches the nonce each call), so retrying the whole
    // call is safe — a tx that already landed shows up as funded on retry.
    let mut funded = false;
    for attempt in 1..=6u32 {
        match fund_account_from_genesis(
            &mut wallet,
            "https://testnet.lez.logos.co",
            account_id,
            GENESIS_FUND_AMOUNT,
        )
        .await
        {
            Ok(()) => {
                funded = true;
                break;
            }
            Err(e) => {
                eprintln!("funding attempt {attempt}/6 failed: {e:#}");
                if attempt < 6 {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
            }
        }
    }
    if !funded {
        anyhow::bail!(
            "funding from genesis failed after 6 attempts — the wallet + mnemonic \
             are saved at {} (account {account_id}); retry funding separately",
            dir.display()
        );
    }
    wallet
        .sync_to_latest_block()
        .await
        .context("post-funding sync")?;

    println!();
    println!("=== YOUR LOGOS WALLET ===");
    println!("  account id (hex):  {account_id}");
    println!("    — paste this in the osm app's REGISTRAR field");
    println!("    — this is your address for receiving LEZ");
    println!();
    println!("  RECOVERY MNEMONIC (write it down, keep it secret):");
    println!("    {mnemonic}");
    println!();
    println!("  wallet storage:    {}", dir.display());
    println!("  password:          testpw");
    println!("  funded:            {GENESIS_FUND_AMOUNT} LEZ (testnet)");
    Ok(())
}
