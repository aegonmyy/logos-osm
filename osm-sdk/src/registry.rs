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
//! Under the v0.3 program model the guest decodes its instruction with
//! **borsh** (`osm_core::Instruction`), and each account is addressed by a
//! `ProgramShardSelector` naming the account and the program shard the call
//! writes. This client builds the typed [`Instruction`] and borsh-serializes
//! it, and derives the shard selectors in the order the guest's `plan`
//! expects:
//!
//! - `Init`: `[registry_pda, owner]`
//! - `RegisterRegion`: `[registry_pda, region_pda, registrar]`
//! - `RegisterRegionsBatch`: `[registry_pda, registrar, region_pda_0, …]`
//!
//! [`OsmTxBuilt::selectors`] produces the selector list; the registrar's
//! signature (its presence in the transaction's witness set) is what sets
//! `is_authorized` in the guest's account metadata.
//!
//! ## Queries
//!
//! The registry shards are public; a reader (the CLI, the SDK facade)
//! fetches the account bytes over the wallet/sequencer API and decodes with
//! [`decode_region_entry`] / [`decode_registry_state`]. Query helpers here
//! cover by-region, by-parent (subregion enumeration via the frozen table),
//! and mirror lookup by CID.

use lee_core::account::{AccountId, ProgramShardSelector};
use lee_core::program::PdaSeed;
use wallet::{AccountIdentity, AccountMention};

pub use osm_core::{
    Effect, Instruction, MAX_BATCH, MAX_MIRRORS, Mirror, RegionEntry, RegionRegistration,
    RegistryState,
};

/// The guest's fixed registry PDA seed (must match
/// `methods/osm/src/bin/osm_registry.rs::REGISTRY_SEED`).
pub const REGISTRY_SEED: [u8; 32] = *b"/OSM/REGISTRY/V1/SEED/0000000000";

/// Derive the registry PDA account id for a **deployed** program.
///
/// Under the v0.3 program model a deployed program lives at whatever account
/// address the deployer chose, and the same bytecode may be deployed at
/// several addresses. The PDA family is therefore seeded by the program's
/// *account id*, not by its image id, and the guest agrees: it derives from
/// `PlanInput.self_account_id`, which is the program's own account.
pub fn registry_pda(program_account: &AccountId) -> AccountId {
    AccountId::for_public_pda(program_account, &PdaSeed::new(REGISTRY_SEED))
}

/// Derive a region's PDA account id for `(program_account, region_path)`.
///
/// The seed is `SHA-256("osm-region:" || path)` (see
/// [`osm_core::region_seed`]) — the same derivation the guest asserts on.
pub fn region_pda(program_account: &AccountId, region_path: &str) -> AccountId {
    AccountId::for_public_pda(
        program_account,
        &PdaSeed::new(osm_core::region_seed(region_path)),
    )
}

/// An instruction plus the exact accounts the guest's `plan` expects, ready
/// for the wallet's public-transaction submission.
#[derive(Clone, Debug)]
pub struct OsmTxBuilt {
    /// The program account (transaction target).
    pub program_account_id: AccountId,
    /// Account ids in the order the guest's `plan` expects them.
    pub accounts: Vec<AccountId>,
    /// borsh-encoded `osm_core::Instruction`.
    pub instruction: Vec<u8>,
}

impl OsmTxBuilt {
    /// The shard selectors the transaction carries, in guest order. Every
    /// selector names this program's shard: the PDAs are written by it, and
    /// the registrar account is referenced for authorization only.
    #[must_use]
    pub fn selectors(&self) -> Vec<ProgramShardSelector> {
        self.accounts
            .iter()
            .map(|a| ProgramShardSelector::new(*a, self.program_account_id))
            .collect()
    }

    /// The wallet mentions for submission: `signer` signs,
    /// every other account is referenced unsigned, and each is scoped to this
    /// program's shard. The signer's signature is what sets `is_authorized` in
    /// the guest's account metadata.
    #[must_use]
    pub fn mentions(&self, signer: &AccountId) -> Vec<AccountMention> {
        self.accounts
            .iter()
            .map(|a| {
                let identity = if a == signer {
                    AccountIdentity::Public(*a)
                } else {
                    AccountIdentity::PublicNoSign(*a)
                };
                identity.select_program_shard(self.program_account_id)
            })
            .collect()
    }
}

fn words(i: &Instruction) -> Vec<u8> {
    borsh::to_vec(i).expect("osm_core::Instruction serializes with borsh")
}

/// Build `Init`: binds `owner` as the registry authority (provenance only —
/// registration itself stays permissionless).
pub fn build_init(program_account: &AccountId, owner: &AccountId) -> OsmTxBuilt {
    OsmTxBuilt {
        program_account_id: *program_account,
        accounts: vec![registry_pda(program_account), *owner],
        instruction: words(&Instruction::Init {
            owner: *owner.value(),
        }),
    }
}

/// Build `RegisterRegion` for one mirror of one region.
pub fn build_register_region(
    program_account: &AccountId,
    registrar: &AccountId,
    registration: &RegionRegistration,
) -> OsmTxBuilt {
    OsmTxBuilt {
        program_account_id: *program_account,
        accounts: vec![
            registry_pda(program_account),
            region_pda(program_account, &registration.region),
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
    program_account: &AccountId,
    registrar: &AccountId,
    registrations: &[RegionRegistration],
) -> OsmTxBuilt {
    assert!(
        (1..=MAX_BATCH).contains(&registrations.len()),
        "batch registration takes 1..={MAX_BATCH} regions, got {}",
        registrations.len()
    );
    let mut accounts = vec![registry_pda(program_account), *registrar];
    for r in registrations {
        accounts.push(region_pda(program_account, &r.region));
    }
    OsmTxBuilt {
        program_account_id: *program_account,
        accounts,
        instruction: words(&Instruction::RegisterRegionsBatch {
            registrations: registrations.to_vec(),
        }),
    }
}

/// Resolve a region to the entry the registry records for it on-chain.
///
/// This is the consumer path: a Basecamp module that wants map data needs to
/// turn a region into the CID of the bytes an evaluator (or any reader) can
/// fetch and re-verify. Returns `Ok(None)` when the region holds no
/// registration yet, which is an ordinary answer rather than an error.
///
/// Reads the sequencer's `getAccount` directly, so it needs no wallet and no
/// local storage node.
///
/// ```no_run
/// # async fn demo() -> anyhow::Result<()> {
/// use logos_osm::registry::{resolve_region, RegionEntry};
/// # let program_account = lee_core::account::AccountId::new([0u8; 32]);
/// let entry: Option<RegionEntry> =
///     resolve_region("https://testnet.lez.logos.co", &program_account, "germany").await?;
/// if let Some(e) = entry {
///     if let Some(latest) = e.latest_mirror() {
///         println!("germany is at {} (v{})", latest.cid, latest.version);
///     }
/// }
/// # Ok(()) }
/// ```
pub async fn resolve_region(
    sequencer_url: &str,
    program_account: &AccountId,
    region_path: &str,
) -> anyhow::Result<Option<RegionEntry>> {
    use sequencer_service_rpc::{RpcClient as _, SequencerClientBuilder};

    let client = SequencerClientBuilder::default()
        .build(sequencer_url.to_string())
        .map_err(|e| anyhow::anyhow!("sequencer {sequencer_url}: {e}"))?;
    let pda = region_pda(program_account, region_path);
    let account = client
        .get_account(pda)
        .await
        .map_err(|e| anyhow::anyhow!("reading {region_path} from the sequencer: {e}"))?;

    let shard = account.data.shard(*program_account);
    if shard.as_ref().is_empty() {
        return Ok(None);
    }
    let entry: RegionEntry = borsh::from_slice(shard.as_ref())
        .map_err(|e| anyhow::anyhow!("decoding {region_path}: {e}"))?;
    Ok(Some(entry))
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

    /// A deployed program's account id. Under v0.3 this is the address the
    /// deployer chose, so the tests use an arbitrary one.
    fn prog() -> AccountId {
        AccountId::new([7; 32])
    }
    const REGISTRAR: [u8; 32] = [9; 32];

    fn registrar() -> AccountId {
        AccountId::new(REGISTRAR)
    }

    fn registration(path: &str) -> RegionRegistration {
        RegionRegistration {
            region: path.into(),
            cid: "cid0".into(),
            source_url: crate::regions::by_path(path)
                .map(|r| r.source_url().to_string())
                .unwrap_or_default(),
            checksum: [0xab; 16],
            version: 20260821,
            timestamp: 1_787_000_000,
        }
    }

    #[test]
    fn region_pda_is_path_seeded() {
        // Different paths -> different accounts; stable across calls; and
        // distinct from the registry PDA.
        let a = region_pda(&prog(), "germany");
        let b = region_pda(&prog(), "us/california");
        assert_ne!(a, b);
        assert_eq!(a, region_pda(&prog(), "germany"));
        assert_ne!(a, registry_pda(&prog()));
    }

    #[test]
    fn init_accounts_and_words() {
        let owner = registrar();
        let built = build_init(&prog(), &owner);
        assert_eq!(built.accounts.len(), 2);
        assert_eq!(built.accounts[0], registry_pda(&prog()));
        assert_eq!(built.accounts[1], owner);
        assert_eq!(built.program_account_id, prog());
        assert!(!built.instruction.is_empty());
        // Round-trip: the bytes decode back to the same instruction (this is
        // the guest's exact decode path).
        let back: Instruction = borsh::from_slice(&built.instruction).expect("decode init");
        assert_eq!(back, Instruction::Init { owner: REGISTRAR });
    }

    #[test]
    fn register_accounts_and_words() {
        let built = build_register_region(&prog(), &registrar(), &registration("germany"));
        assert_eq!(built.accounts.len(), 3);
        assert_eq!(built.accounts[0], registry_pda(&prog()));
        assert_eq!(built.accounts[1], region_pda(&prog(), "germany"));
        assert_eq!(built.accounts[2], registrar());
        let back: Instruction = borsh::from_slice(&built.instruction).unwrap();
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
        let built = build_register_regions_batch(&prog(), &registrar(), &regs);
        assert_eq!(built.accounts.len(), 2 + regs.len());
        assert_eq!(built.accounts[1], registrar());
        for (i, r) in regs.iter().enumerate() {
            assert_eq!(built.accounts[2 + i], region_pda(&prog(), &r.region));
        }
        let back: Instruction = borsh::from_slice(&built.instruction).unwrap();
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
        build_register_regions_batch(&prog(), &registrar(), &regs);
    }

    #[test]
    fn selectors_name_the_program_shard() {
        let built = build_register_region(&prog(), &registrar(), &registration("kenya"));
        let selectors = built.selectors();
        assert_eq!(selectors.len(), 3);
        for (sel, acct) in selectors.iter().zip(built.accounts.iter()) {
            assert_eq!(sel.account_id, *acct);
            // Every selector names this program's shard: the PDAs are written
            // by it and the registrar is referenced for authorization.
            assert_eq!(sel.program_account_id, prog());
        }
    }

    #[test]
    fn entries_roundtrip_through_borsh_decode() {
        let mut entry = placeholder_entry(by_path("germany").unwrap());
        entry.mirrors.push(Mirror {
            registrar: REGISTRAR,
            cid: "cid0".into(),
            source_url: "https://download.geofabrik.de/europe/germany-latest.osm.pbf".into(),
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
