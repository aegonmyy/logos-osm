//! Print the testnet's genesis-funded accounts so a deploy can name a payer.
//!
//! `testnet_initial_state` is the crate the LEZ repo uses to build the
//! network's initial state, and it publishes the private keys of the funded
//! accounts. If the public testnet runs that genesis, these are usable as a
//! payer.
use testnet_initial_state::{initial_pub_accounts_private_keys, initial_public_user_accounts};

fn main() {
    println!("=== public accounts with published private keys ===");
    for a in initial_pub_accounts_private_keys() {
        println!("  account_id = {}", a.account_id);
    }
    println!("=== initial public user accounts ===");
    for a in initial_public_user_accounts() {
        println!("  {a:?}");
    }
}
