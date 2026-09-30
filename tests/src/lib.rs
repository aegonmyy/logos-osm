//! Integration-test crate: end-to-end runs against a standalone LEZ
//! sequencer (Docker) and real storage nodes.

use anyhow::Result;
use lee::AccountId;
use program_loader_core::MAX_SEGMENT_DATA_LEN;
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
