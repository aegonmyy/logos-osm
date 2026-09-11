//! The LEZ OSM region-registry client (host side).
//!
//! The registry is the public index of hosted region snapshots: one region
//! PDA per closed-set region holding a [`RegionEntry`] — the append-only
//! [`Mirror`] list of (registrar, CID, MD5, version, timestamp) records.
//! Registration is **permissionless** — any signer may mirror; adoption
//! wants ≥ 3 distinct registrars — so this client takes an explicit
//! registrar account rather than a privileged owner.
//!
//! ## Encoding contract with the guest
//!
//! The guest (`methods/osm`) decodes its instruction with
//! `lee_core::program::read_lee_inputs::<osm_core::Instruction>()`, i.e.
//! **risc0-serde** (`risc0_zkvm::serde::Deserializer`) over the transaction's
//! `Vec<u32>` instruction words. This client therefore builds the typed
//! [`Instruction`] and serializes it with `risc0_zkvm::serde::to_vec` — the
//! mirror of the guest's decoder. The account list each instruction needs is
//! derived here so the submitter passes exactly the accounts the guest
//! asserts on, in order:
//!
//! - `Init`: `[registry_pda, owner]`
//! - `RegisterRegion`: `[registry_pda, region_pda, registrar]`
//! - `RegisterRegionsBatch`: `[registry_pda, registrar, region_pda_0, …]`
//!
//! [`to_identities`] maps that list onto wallet identities: the signer is
//! `Public` (signs), everything else `PublicNoSign` (read-only) — exactly the
//! authorization set the guest asserts on.
//!
//! ## Queries
//!
//! The registry accounts are public; a reader (the CLI, the SDK facade)
//! fetches the account bytes over the wallet/sequencer API and decodes with
//! [`decode_region_entry`] / [`decode_registry_state`]. Query helpers here
//! cover by-region, by-parent (subregion enumeration via the frozen table),
//! and mirror lookup by CID.

use lee_core::account::AccountId;
use lee_core::program::{PdaSeed, ProgramId};
use risc0_zkvm::serde::to_vec;
use wallet::AccountIdentity;

pub use osm_core::{
    Instruction, Mirror, RegionEntry, RegionRegistration, RegistryState, MAX_BATCH, MAX_MIRRORS,
};

/// The guest's fixed registry PDA seed (must match
/// `methods/osm/src/bin/osm_registry.rs::REGISTRY_SEED`).
pub const REGISTRY_SEED: [u8; 32] = *b"/OSM/REGISTRY/V1/SEED/0000000000";

/// Derive the registry PDA account id for a deployed program.
pub fn registry_pda(program_id: &ProgramId) -> AccountId {
    AccountId::for_public_pda(program_id, &PdaSeed::new(REGISTRY_SEED))
}

/// Derive a region's PDA account id for `(program_id, region_path)`.
///
/// The seed is `SHA-256("osm-region:" || path)` (see
/// [`osm_core::region_seed`]) — the same derivation the guest asserts on.
pub fn region_pda(program_id: &ProgramId, region_path: &str) -> AccountId {
    AccountId::for_public_pda(
        program_id,
        &PdaSeed::new(osm_core::region_seed(region_path)),
    )
}

/// An instruction plus the exact accounts the guest asserts on, ready for the
/// wallet's public-transaction submission.
#[derive(Clone, Debug)]
pub struct OsmTxBuilt {
    /// Account ids in the order the guest's `execute` expects them.
    pub accounts: Vec<AccountId>,
    /// risc0-serde-encoded `osm_core::Instruction` words.
    pub instruction: Vec<u32>,
}

fn words(i: &Instruction) -> Vec<u32> {
    to_vec(i).expect("osm_core::Instruction serializes to risc0-serde words")
}

/// Build `Init`: binds `owner` as the registry authority (provenance only —
/// registration itself stays permissionless).
pub fn build_init(program_id: &ProgramId, owner: &AccountId) -> OsmTxBuilt {
    OsmTxBuilt {
        accounts: vec![registry_pda(program_id), *owner],
        instruction: words(&Instruction::Init {
            owner: *owner.value(),
        }),
    }
}

/// Build `RegisterRegion` for one mirror of one region.
pub fn build_register_region(
    program_id: &ProgramId,
    registrar: &AccountId,
    registration: &RegionRegistration,
) -> OsmTxBuilt {
    OsmTxBuilt {
        accounts: vec![
            registry_pda(program_id),
            region_pda(program_id, &registration.region),
            *registrar,
        ],
        instruction: words(&Instruction::RegisterRegion {
            registration: registration.clone(),
        }),
    }
}

/// Build `RegisterRegionsBatch` (bulk hosting): 1..=[`MAX_BATCH`]
/// registrations in one transaction.
pub fn build_register_regions_batch(
    program_id: &ProgramId,
    registrar: &AccountId,
    registrations: &[RegionRegistration],
) -> OsmTxBuilt {
    assert!(
        (1..=MAX_BATCH).contains(&registrations.len()),
        "batch registration takes 1..={MAX_BATCH} regions, got {}",
        registrations.len()
    );
    let mut accounts = vec![registry_pda(program_id), *registrar];
    for r in registrations {
        accounts.push(region_pda(program_id, &r.region));
    }
    OsmTxBuilt {
        accounts,
        instruction: words(&Instruction::RegisterRegionsBatch {
            registrations: registrations.to_vec(),
        }),
    }
}

/// Map a built tx's accounts to wallet identities: `signer` signs (`Public`),
/// every other account (the PDAs) is passed unsigned (`PublicNoSign`).
pub fn to_identities(built: &OsmTxBuilt, signer: &AccountId) -> Vec<AccountIdentity> {
    built
        .accounts
        .iter()
        .map(|a| {
            if a == signer {
                AccountIdentity::Public(*a)
            } else {
                AccountIdentity::PublicNoSign(*a)
            }
        })
        .collect()
}

/// Decode a registry account's data bytes.
pub fn decode_registry_state(bytes: &[u8]) -> anyhow::Result<RegistryState> {
    borsh::from_slice(bytes).map_err(|e| anyhow::anyhow!("decoding RegistryState: {e}"))
}

/// Decode a region account's data bytes.
pub fn decode_region_entry(bytes: &[u8]) -> anyhow::Result<RegionEntry> {
    borsh::from_slice(bytes).map_err(|e| anyhow::anyhow!("decoding RegionEntry: {e}"))
}

// ---------------------------------------------------------------------------
// Client-side query views over decoded entries (chain-reader-agnostic)
// ---------------------------------------------------------------------------

/// Query result: one region's on-chain record.
#[derive(Debug, Clone)]
pub struct RegionQuery {
    /// The frozen-table region this entry belongs to (None when the entry's
    /// path is outside the closed set — possible only for a foreign program).
    pub region: Option<&'static crate::regions::Region>,
    /// The decoded on-chain entry.
    pub entry: RegionEntry,
}

impl RegionQuery {
    /// The latest (newest) mirror, if any.
    pub fn latest(&self) -> Option<&Mirror> {
        self.entry.latest_mirror()
    }

    /// Distinct registrars currently mirroring this region (adoption wants
    /// >= 3).
    pub fn registrars(&self) -> Vec<[u8; 32]> {
        self.entry.registrars()
    }

    /// The mirror hosting under `cid`, if any.
    pub fn mirror_by_cid(&self, cid: &str) -> Option<&Mirror> {
        self.entry.mirrors.iter().find(|m| m.cid == cid)
    }
}

/// A query-shaped view of a region with no on-chain data yet (mirrors stay
/// empty); used to enumerate candidates from the frozen table.
pub fn placeholder_entry(region: &'static crate::regions::Region) -> RegionEntry {
    RegionEntry {
        region: region.path.to_string(),
        parent: region.parent.unwrap_or("").to_string(),
        level: if matches!(region.level, crate::regions::Level::Country) {
            0
        } else {
            1
        },
        mirrors: Vec::new(),
    }
}

/// All closed-set regions (the query universe: the chain cannot enumerate
/// accounts by itself — the closed set is what makes exhaustive query
/// possible client-side). Callers filter with [`RegionQuery`] data once
/// fetched.
pub fn all_regions() -> &'static [crate::regions::Region] {
    crate::regions::REGIONS
}

/// All subregions of `parent_path` from the frozen table (the "query by
/// parent" criterion: e.g. every hosted US state).
pub fn children_of(parent_path: &str) -> Vec<&'static crate::regions::Region> {
    crate::regions::children_of(parent_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::by_path;

    const PROG: ProgramId = [7, 0, 0, 0, 0, 0, 0, 0];
    const REGISTRAR: [u8; 32] = [9; 32];

    fn registrar() -> AccountId {
        AccountId::new(REGISTRAR)
    }

    fn registration(path: &str) -> RegionRegistration {
        RegionRegistration {
            region: path.into(),
            cid: "cid0".into(),
            checksum: [0xab; 16],
            version: 20260821,
            timestamp: 1_787_000_000,
        }
    }

    #[test]
    fn region_pda_is_path_seeded() {
        // Different paths -> different accounts; stable across calls; and
        // distinct from the registry PDA.
        let a = region_pda(&PROG, "germany");
        let b = region_pda(&PROG, "us/california");
        assert_ne!(a, b);
        assert_eq!(a, region_pda(&PROG, "germany"));
        assert_ne!(a, registry_pda(&PROG));
    }

    #[test]
    fn init_accounts_and_words() {
        let owner = registrar();
        let built = build_init(&PROG, &owner);
        assert_eq!(built.accounts.len(), 2);
        assert_eq!(built.accounts[0], registry_pda(&PROG));
        assert_eq!(built.accounts[1], owner);
        assert!(!built.instruction.is_empty());
        // Round-trip: the words decode back to the same instruction (this is
        // the guest's exact decode path).
        let back: Instruction =
            risc0_zkvm::serde::from_slice(&built.instruction).expect("decode init words");
        assert_eq!(back, Instruction::Init { owner: REGISTRAR });
    }

    #[test]
    fn register_accounts_and_words() {
        let built = build_register_region(&PROG, &registrar(), &registration("germany"));
        assert_eq!(built.accounts.len(), 3);
        assert_eq!(built.accounts[0], registry_pda(&PROG));
        assert_eq!(built.accounts[1], region_pda(&PROG, "germany"));
        assert_eq!(built.accounts[2], registrar());
        let back: Instruction = risc0_zkvm::serde::from_slice(&built.instruction).unwrap();
        assert_eq!(
            back,
            Instruction::RegisterRegion {
                registration: registration("germany")
            }
        );
    }

    #[test]
    fn batch_accounts_and_words() {
        let regs: Vec<RegionRegistration> = ["germany", "france", "us/california"]
            .iter()
            .map(|p| registration(p))
            .collect();
        let built = build_register_regions_batch(&PROG, &registrar(), &regs);
        assert_eq!(built.accounts.len(), 2 + regs.len());
        assert_eq!(built.accounts[1], registrar());
        for (i, r) in regs.iter().enumerate() {
            assert_eq!(built.accounts[2 + i], region_pda(&PROG, &r.region));
        }
        let back: Instruction = risc0_zkvm::serde::from_slice(&built.instruction).unwrap();
        match back {
            Instruction::RegisterRegionsBatch { registrations } => {
                assert_eq!(registrations, regs);
            }
            other => panic!("wrong instruction {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "1..=24")]
    fn batch_over_cap_panics() {
        let regs: Vec<RegionRegistration> = (0..=MAX_BATCH)
            .map(|i| {
                let p = crate::regions::REGIONS[i].path;
                RegionRegistration {
                    region: p.to_string(),
                    ..registration("germany")
                }
            })
            .collect();
        build_register_regions_batch(&PROG, &registrar(), &regs);
    }

    #[test]
    fn identities_sign_only_the_registrar() {
        let built = build_register_region(&PROG, &registrar(), &registration("kenya"));
        let ids = to_identities(&built, &registrar());
        assert_eq!(ids.len(), 3);
        for (id, acct) in ids.iter().zip(built.accounts.iter()) {
            if acct == &registrar() {
                assert!(matches!(id, AccountIdentity::Public(_)));
            } else {
                assert!(matches!(id, AccountIdentity::PublicNoSign(_)));
            }
        }
    }

    #[test]
    fn entries_roundtrip_through_borsh_decode() {
        let mut entry = placeholder_entry(by_path("germany").unwrap());
        entry.mirrors.push(Mirror {
            registrar: REGISTRAR,
            cid: "cid0".into(),
            checksum: [1; 16],
            version: 20260821,
            timestamp: 5,
            hosted: true,
        });
        let bytes = borsh::to_vec(&entry).unwrap();
        let back = decode_region_entry(&bytes).unwrap();
        assert_eq!(back.region, "germany");
        assert_eq!(back.parent, "");
        assert_eq!(back.level, 0);
        assert_eq!(back.registrars().len(), 1);
    }

    #[test]
    fn children_of_enumerates_the_closed_set() {
        let kids = children_of("us");
        assert_eq!(kids.len(), 8);
        assert!(kids.iter().all(|r| r.parent == Some("us")));
        assert!(children_of("narnia").is_empty());
    }
}
