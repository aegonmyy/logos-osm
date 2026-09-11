//! Shared types for the OSM registry LEZ program.
//!
//! Used by BOTH:
//! - the RISC0 guest (`methods/osm`), which deserializes [`Instruction`] and
//!   reads/writes account `data` as borsh-serialized records; and
//! - the host client (`logos_osm::registry`), which serializes [`Instruction`]
//!   into the transaction's instruction words and derives the region accounts.
//!
//! They depend only on `serde` + `borsh`, so they compile for the risc0 guest
//! target as well as the host.
//!
//! ## Registry model
//!
//! Map data is public, so — unlike a private-vault registry — the on-chain
//! records carry the useful public facts verbatim: which region, which
//! snapshot version, which checksum, stored under which CID, by which
//! registrar, when. One **region account** per region of the predefined set
//! holds a [`RegionEntry`] whose `mirrors` list carries one [`Mirror`] per
//! (registrar, registration) — so redundant hosting by multiple accounts and
//! version history over time are both directly verifiable from the chain,
//! which is exactly what the prize's adoption criteria measure.

#![forbid(unsafe_code)]

pub mod set;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Max mirrors retained per region (adoption requires >= 3 distinct
/// registrars; history beyond this cap is pruned oldest-first so the
/// *freshness* trail keeps the most recent entries).
pub const MAX_MIRRORS: usize = 32;

/// Max regions in one `RegisterRegionsBatch` transaction.
pub const MAX_BATCH: usize = 24;

/// Registry state, borsh-serialized into the registry account's `data`.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct RegistryState {
    /// The deployer/authority `AccountId` (32 bytes). Set on `Init`.
    /// Registration itself is **permissionless** (any signer may mirror a
    /// region — the adoption criteria depend on multiple distinct accounts
    /// hosting the same region); the owner is recorded for provenance and
    /// future admin instructions.
    pub owner: [u8; 32],
    /// Number of regions with at least one mirror ever registered.
    pub region_count: u32,
    /// Total registrations (mirror appends) ever accepted.
    pub registration_count: u64,
    /// `1` once `Init` has run.
    pub initialized: u8,
}

/// One hosted mirror of a region snapshot, registered by one account.
///
/// The append-only `mirrors` list of a [`RegionEntry`] is the version
/// history: a registrar advancing to a newer Geofabrik snapshot appends a new
/// [`Mirror`] (new `cid`, `checksum`, `version`, `timestamp`), so both
/// "who mirrors this region" and "was this mirror maintained" are readable
/// directly from the chain.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct Mirror {
    /// The registrar's `AccountId` (the transaction signer).
    pub registrar: [u8; 32],
    /// The Logos Storage CID of the stored PBF snapshot (public locator).
    pub cid: String,
    /// Geofabrik's published MD5 for this snapshot (raw 16 bytes).
    pub checksum: [u8; 16],
    /// The snapshot's Geofabrik version date in `YYYYMMDD` form (e.g.
    /// `20260524`). Parsed from the dated filename inside the `.md5`
    /// (`germany-260524.osm.pbf`).
    pub version: u32,
    /// Registration time, unix seconds. The guest keeps `mirrors` sorted
    /// ascending by `timestamp`.
    pub timestamp: u64,
    /// `true` while the registrar asserts the bytes are stored under `cid`.
    pub hosted: bool,
}

/// A region's on-chain record, borsh-serialized into its region account's
/// `data`. Fields mirror the prize's registry schema (A.2); `region` is the
/// Geofabrik path and the unique key.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct RegionEntry {
    /// The region's Geofabrik path (`germany`, `us/california`). Unique.
    pub region: String,
    /// The containing region's path, or empty for a top-level country.
    pub parent: String,
    /// `0` = country, `1` = subregion.
    pub level: u8,
    /// Mirrors, ordered by `timestamp` ascending (oldest first).
    pub mirrors: Vec<Mirror>,
}

impl RegionEntry {
    /// The latest (newest-timestamp) mirror, if any.
    pub fn latest_mirror(&self) -> Option<&Mirror> {
        self.mirrors.last()
    }

    /// The distinct registrars currently mirroring this region.
    pub fn registrars(&self) -> Vec<[u8; 32]> {
        let mut seen = std::vec::Vec::new();
        for m in &self.mirrors {
            if !seen.contains(&m.registrar) {
                seen.push(m.registrar);
            }
        }
        seen
    }
}

/// One registration inside a batch; mirrors [`Instruction::RegisterRegion`]'s
/// payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct RegionRegistration {
    /// The region's Geofabrik path (must be in the predefined set).
    pub region: String,
    /// Logos Storage CID of the stored snapshot.
    pub cid: String,
    /// Geofabrik's published MD5 (raw 16 bytes).
    pub checksum: [u8; 16],
    /// Snapshot version date, `YYYYMMDD`.
    pub version: u32,
    /// Registration time, unix seconds.
    pub timestamp: u64,
}

/// The typed instruction the guest deserializes from the transaction's
/// instruction words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum Instruction {
    /// Initialize the registry, recording `owner`. Run once.
    ///
    /// Accounts (pre_states): `[registry, owner_signer]`.
    Init { owner: [u8; 32] },
    /// Register (or advance) one region mirror. Permissionless: any signer.
    ///
    /// Accounts (pre_states): `[registry, region_account, registrar_signer]`.
    /// The region account must be the PDA seeded by `SHA-256(region_path)`.
    RegisterRegion { registration: RegionRegistration },
    /// Register many regions in one transaction (bulk hosting).
    ///
    /// Accounts (pre_states): `[registry, registrar_signer, region_0, ...]`,
    /// matching `registrations` in order. 1..=[`MAX_BATCH`] entries.
    RegisterRegionsBatch {
        registrations: Vec<RegionRegistration>,
    },
}

/// Numeric opcodes (diagnostics / IDL). Match the [`Instruction`] variant
/// order and the hand-written IDL.
pub mod abi {
    pub const INIT: u8 = 1;
    pub const REGISTER_REGION: u8 = 2;
    pub const REGISTER_REGIONS_BATCH: u8 = 3;
}

/// The PDA seed for a region account: `SHA-256("osm-region:" || path)`.
///
/// Domain-separated from a bare path hash so the seed cannot collide with any
/// other PDA family this program may grow.
pub fn region_seed(path: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"osm-region:");
    h.update(path.as_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_entry_roundtrip_borsh() {
        let e = RegionEntry {
            region: "us/california".into(),
            parent: "us".into(),
            level: 1,
            mirrors: std::vec![Mirror {
                registrar: [7; 32],
                cid: "zDv...".into(),
                checksum: [1; 16],
                version: 20260524,
                timestamp: 1_780_000_000,
                hosted: true,
            }],
        };
        let bytes = borsh::to_vec(&e).unwrap();
        let back: RegionEntry = borsh::from_slice(&bytes).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn instruction_roundtrip_borsh() {
        let i = Instruction::RegisterRegion {
            registration: RegionRegistration {
                region: "germany".into(),
                cid: "cid".into(),
                checksum: [9; 16],
                version: 20260801,
                timestamp: 42,
            },
        };
        let bytes = borsh::to_vec(&i).unwrap();
        let back: Instruction = borsh::from_slice(&bytes).unwrap();
        assert_eq!(i, back);
    }

    #[test]
    fn region_seed_is_domain_separated() {
        // Different paths => different seeds; stable across calls.
        let a = region_seed("germany");
        let b = region_seed("us/california");
        assert_ne!(a, b);
        assert_eq!(a, region_seed("germany"));
        // A bare SHA-256 of the path (no domain tag) must differ.
        let bare: [u8; 32] = Sha256::digest(b"germany").into();
        assert_ne!(a, bare);
    }

    #[test]
    fn registrars_dedup() {
        let e = RegionEntry {
            region: "germany".into(),
            parent: String::new(),
            level: 0,
            mirrors: std::vec![
                Mirror {
                    registrar: [1; 32],
                    timestamp: 1,
                    ..Default::default()
                },
                Mirror {
                    registrar: [1; 32],
                    timestamp: 2,
                    ..Default::default()
                },
                Mirror {
                    registrar: [2; 32],
                    timestamp: 3,
                    ..Default::default()
                },
            ],
        };
        assert_eq!(e.registrars().len(), 2);
        assert_eq!(e.latest_mirror().unwrap().timestamp, 3);
    }
}
