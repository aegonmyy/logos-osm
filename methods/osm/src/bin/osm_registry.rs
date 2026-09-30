//! LEZ RISC0 guest: the OSM region registry.
//!
//! ## Account model
//!
//! Two families of PDA accounts (derived from the program's own account id):
//! - a single **registry** shard (seed [`REGISTRY_SEED`]) holding
//!   [`osm_core::RegistryState`]; and
//! - one **region** shard per predefined region (seed = `SHA-256("osm-region:"
//!   || path)`) holding [`osm_core::RegionEntry`] — the region's mirrors with
//!   full version history.
//!
//! Registration is **permissionless**: any signer may mirror a region. The
//! prize's adoption criteria (multiple distinct registrars per covered region,
//! version advancement over time) are computed directly from these shards, so
//! the chain is the adoption ledger.
//!
//! The guest embeds the frozen region table (`osm_core::set::REGIONS`) and
//! rejects any path outside it, so the chain itself enforces the
//! non-overlapping partition (appendix A.4). `parent` and `level` are taken
//! from the embedded table, never from the client's claim.
//!
//! ## v0.3 program model
//!
//! The program is split into `plan` and `apply`, and `run_program` dispatches
//! whichever the runtime asks for:
//!
//! - [`plan`] sees the instruction and the **account metadata** (ids and
//!   authorization), never account contents. It validates the structure that
//!   metadata can establish (the registry and region accounts really are the
//!   PDAs this program derives, the registrar signed) and emits one
//!   [`Effect`] per shard the program writes.
//! - [`apply`] runs once per emitted effect with that shard's **pre-data**. It
//!   enforces the state-dependent preconditions (registry initialized, version
//!   in range) and returns the shard's new contents. A failed precondition
//!   panics, which rejects the transaction.
//!
//! A program writes only its own shard on an account, so the registrar's
//! account is referenced for its authorization flag and never written. The
//! v0.2 claim-on-first-touch dance (a signer's second transaction failing with
//! `NonDefaultAccountWithDefaultOwner`) does not arise in this model: there is
//! no program-owned-account claim to make.

use lee_core::account::{AccountId, ShardData};
use lee_core::program::{
    AccountMeta, PdaSeed, Plan, PlanInput, ProgramId, run_program,
};

use osm_core::set::{Level, REGIONS};
use osm_core::{
    Effect, Instruction, MAX_BATCH, MAX_MIRRORS, MAX_REGIONS, Mirror, RegionEntry,
    RegionRegistration, RegistryState, region_seed,
};

/// Fixed PDA seed for the single registry account.
pub const REGISTRY_SEED: [u8; 32] = *b"/OSM/REGISTRY/V1/SEED/0000000000";

/// Accepted `version` range: any YYYYMMDD date. Grouped YYYY_MM_DD for
/// readability; clippy's uniform-grouping rule would obscure the dates.
#[allow(clippy::inconsistent_digit_grouping)]
pub const MIN_VERSION: u32 = 1_970_01_01; // 1970-01-01
#[allow(clippy::inconsistent_digit_grouping)]
pub const MAX_VERSION: u32 = 9_999_12_31; // 9999-12-31

fn main() {
    run_program(plan, apply)
}

/// Derive the registry PDA account id from the program's own account id.
fn registry_pda_id(program: &AccountId) -> AccountId {
    AccountId::for_public_pda(program, &PdaSeed::new(REGISTRY_SEED))
}

/// Derive a region PDA account id from the program's own account id.
fn region_pda_id(program: &AccountId, region_path: &str) -> AccountId {
    AccountId::for_public_pda(program, &PdaSeed::new(region_seed(region_path)))
}

/// The embedded-table entry for a region path, or panic (reject) if the path
/// is outside the predefined closed set.
fn require_in_set(path: &str) -> (&'static str, u8) {
    let r = REGIONS
        .iter()
        .find(|r| r.path == path)
        .unwrap_or_else(|| panic!("RegisterRegion: {path} is outside the predefined region set"));
    let level = match r.level {
        Level::Country => 0u8,
        Level::Subregion => 1u8,
    };
    (r.parent.unwrap_or(""), level)
}

/// The planning half: validate what account metadata can establish, then emit
/// one effect per shard this program writes.
fn plan(input: &PlanInput, instruction: Instruction) -> Plan {
    let mut plan = Plan::new(input);
    let program = input.self_account_id;

    match instruction {
        Instruction::Init { owner } => {
            let [reg, owner_acc] = exactly::<2>(&input.accounts);
            assert_eq!(
                reg.account_id,
                registry_pda_id(&program),
                "Init: first account must be the registry PDA"
            );
            assert!(owner_acc.is_authorized, "Init: owner must sign");
            assert_eq!(
                owner_acc.account_id.value(),
                &owner,
                "Init: signer does not match owner in instruction"
            );
            plan.effect(reg, &Effect::Init { owner });
        }

        Instruction::RegisterRegion { registration } => {
            let [reg, region, registrar] = exactly::<3>(&input.accounts);
            assert_eq!(
                reg.account_id,
                registry_pda_id(&program),
                "registry account is not the registry PDA"
            );
            assert!(
                registrar.is_authorized,
                "RegisterRegion: registrar must sign"
            );
            let path = registration.region.as_str();
            // The embedded table is the authority for parent/level, and rejects
            // regions outside the frozen closed set.
            require_in_set(path);
            assert_eq!(
                region.account_id,
                region_pda_id(&program, path),
                "RegisterRegion: region account is not the PDA for region {path}"
            );
            plan.effect(
                reg,
                &Effect::RecordRegistrations {
                    paths: vec![path.to_string()],
                    registrations: 1,
                },
            );
            plan.effect(
                region,
                &Effect::AppendMirror {
                    registrar: *registrar.account_id.value(),
                    registration,
                },
            );
        }

        Instruction::RegisterRegionsBatch { registrations } => {
            let n = registrations.len();
            assert!(
                (1..=MAX_BATCH).contains(&n),
                "RegisterRegionsBatch: need 1..={MAX_BATCH} registrations, got {n}"
            );
            assert_eq!(
                input.accounts.len(),
                2 + n,
                "RegisterRegionsBatch: expected 2 + {n} accounts"
            );
            let reg = &input.accounts[0];
            let registrar = &input.accounts[1];
            assert_eq!(
                reg.account_id,
                registry_pda_id(&program),
                "registry account is not the registry PDA"
            );
            assert!(
                registrar.is_authorized,
                "RegisterRegionsBatch: registrar must sign"
            );

            let mut paths = Vec::with_capacity(n);
            for (r, region_meta) in registrations.iter().zip(input.accounts[2..].iter()) {
                let path = r.region.as_str();
                require_in_set(path);
                assert_eq!(
                    region_meta.account_id,
                    region_pda_id(&program, path),
                    "RegisterRegion: region account is not the PDA for region {path}"
                );
                plan.effect(
                    region_meta,
                    &Effect::AppendMirror {
                        registrar: *registrar.account_id.value(),
                        registration: r.clone(),
                    },
                );
                paths.push(path.to_string());
            }
            plan.effect(
                reg,
                &Effect::RecordRegistrations {
                    paths,
                    registrations: n as u64,
                },
            );
        }
    }

    plan
}

/// The apply half: enforce state-dependent preconditions against one shard's
/// pre-data and return its new contents. Panicking rejects the transaction.
fn apply(effect: Effect, pre: &ShardData) -> Option<ShardData> {
    Some(match effect {
        Effect::Init { owner } => {
            assert!(
                pre.is_empty(),
                "Init: registry already initialized"
            );
            ShardData::from(&RegistryState {
                owner,
                regions: Vec::new(),
                registration_count: 0,
                initialized: 1,
            })
        }

        Effect::RecordRegistrations {
            paths,
            registrations,
        } => {
            let mut st = RegistryState::try_from(pre).expect("registry not initialized");
            assert_eq!(st.initialized, 1, "registry not initialized");
            for p in &paths {
                if !st.regions.iter().any(|r| r == p) {
                    st.regions.push(p.clone());
                }
            }
            assert!(
                st.regions.len() <= MAX_REGIONS,
                "registry covers more regions than the predefined set holds"
            );
            st.regions.sort();
            st.registration_count = st
                .registration_count
                .checked_add(registrations)
                .expect("registration counter overflow");
            ShardData::from(&st)
        }

        Effect::AppendMirror {
            registrar,
            registration,
        } => {
            let path = registration.region.as_str();
            let (parent, level) = require_in_set(path);
            assert!(
                (MIN_VERSION..=MAX_VERSION).contains(&registration.version),
                "RegisterRegion: version {version} is not a YYYYMMDD date",
                version = registration.version
            );

            let mut entry = if pre.is_empty() {
                RegionEntry {
                    region: path.to_string(),
                    parent: parent.to_string(),
                    level,
                    mirrors: Vec::new(),
                }
            } else {
                let e = RegionEntry::try_from(pre).expect("region shard holds a different shape");
                assert_eq!(
                    e.region, path,
                    "RegisterRegion: region account holds a different region"
                );
                e
            };

            entry.mirrors.push(Mirror {
                registrar,
                cid: registration.cid,
                checksum: registration.checksum,
                version: registration.version,
                timestamp: registration.timestamp,
                hosted: true,
            });
            // Keep mirrors ordered by timestamp (oldest first) for version
            // sorting, and cap retained history at MAX_MIRRORS (prune oldest).
            entry.mirrors.sort_by_key(|m| m.timestamp);
            while entry.mirrors.len() > MAX_MIRRORS {
                entry.mirrors.remove(0);
            }
            ShardData::from(&entry)
        }
    })
}

/// Assert the account slice has exactly `n` entries and return them.
fn exactly<const N: usize>(accounts: &[AccountMeta]) -> [&AccountMeta; N] {
    accounts
        .iter()
        .collect::<Vec<&AccountMeta>>()
        .try_into()
        .expect("wrong number of accounts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use osm_core::set::by_path;
    use std::collections::HashMap;

    const PROG_ID: ProgramId = [1, 0, 0, 0, 0, 0, 0, 0];
    const OWNER: [u8; 32] = [7; 32];
    const REGISTRAR: [u8; 32] = [9; 32];

    /// The program's own account id, as the host derives it.
    fn program() -> AccountId {
        AccountId::from_builtin_program(PROG_ID)
    }

    fn meta(id: AccountId, authorized: bool) -> AccountMeta {
        AccountMeta::new(id, authorized, program())
    }

    fn signer_meta(id: [u8; 32]) -> AccountMeta {
        AccountMeta::new(AccountId::new(id), true, program())
    }

    fn registration(path: &str, ts: u64) -> RegionRegistration {
        RegionRegistration {
            region: path.into(),
            cid: format!("cid-{path}"),
            checksum: [0xab; 16],
            version: 20260524,
            timestamp: ts,
        }
    }

    /// Run plan, then apply every effect against a shard map, exactly as the
    /// runtime does.
    fn run(
        accounts: Vec<AccountMeta>,
        instruction: Instruction,
        shards: &mut HashMap<AccountId, ShardData>,
    ) -> Vec<AccountId> {
        let input = PlanInput {
            self_account_id: program(),
            caller_account_id: None,
            accounts,
            instruction_data: borsh::to_vec(&instruction).unwrap(),
        };
        let built = plan(&input, instruction);
        let mut written = Vec::new();
        for effect in &built.output().effects {
            let id = effect.selector.account_id;
            let pre = shards.get(&id).cloned().unwrap_or_default();
            let effect_data: Effect = borsh::from_slice(&effect.data).unwrap();
            let post = apply(effect_data, &pre).expect("every effect writes");
            shards.insert(id, post);
            written.push(id);
        }
        written
    }

    fn registry_shard() -> HashMap<AccountId, ShardData> {
        let mut shards = HashMap::new();
        // An initialized registry.
        shards.insert(
            registry_pda_id(&program()),
            ShardData::from(&RegistryState {
                owner: OWNER,
                regions: Vec::new(),
                registration_count: 0,
                initialized: 1,
            }),
        );
        shards
    }

    #[test]
    fn init_binds_owner() {
        let mut shards = HashMap::new();
        let reg = meta(registry_pda_id(&program()), false);
        let owner = signer_meta(OWNER);
        run(
            vec![reg, owner],
            Instruction::Init { owner: OWNER },
            &mut shards,
        );
        let st =
            RegistryState::try_from(shards.get(&registry_pda_id(&program())).unwrap()).unwrap();
        assert_eq!(st.owner, OWNER);
        assert_eq!(st.initialized, 1);
        assert_eq!(st.region_count(), 0);
    }

    #[test]
    fn register_creates_entry_from_embedded_table() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(region_pda_id(&program(), "us/california"), false);
        let who = signer_meta(REGISTRAR);
        run(
            vec![reg, r, who],
            Instruction::RegisterRegion {
                registration: registration("us/california", 100),
            },
            &mut shards,
        );
        let entry =
            RegionEntry::try_from(shards.get(&region_pda_id(&program(), "us/california")).unwrap())
                .unwrap();
        assert_eq!(entry.region, "us/california");
        // parent/level come from the table, not the client.
        assert_eq!(entry.parent, "us");
        assert_eq!(entry.level, 1);
        assert_eq!(entry.mirrors.len(), 1);
        assert_eq!(entry.mirrors[0].registrar, REGISTRAR);
        let st =
            RegistryState::try_from(shards.get(&registry_pda_id(&program())).unwrap()).unwrap();
        assert_eq!(st.region_count(), 1);
        assert_eq!(st.registration_count, 1);
    }

    #[test]
    fn second_registrar_appends_and_keeps_history() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(region_pda_id(&program(), "germany"), false);
        run(
            vec![reg.clone(), r.clone(), signer_meta(REGISTRAR)],
            Instruction::RegisterRegion {
                registration: registration("germany", 100),
            },
            &mut shards,
        );
        run(
            vec![reg, r, signer_meta([0xdd; 32])],
            Instruction::RegisterRegion {
                registration: RegionRegistration {
                    region: "germany".into(),
                    cid: "cid-v2".into(),
                    checksum: [0xcd; 16],
                    version: 20260601,
                    timestamp: 200,
                },
            },
            &mut shards,
        );
        let entry =
            RegionEntry::try_from(shards.get(&region_pda_id(&program(), "germany")).unwrap())
                .unwrap();
        assert_eq!(entry.mirrors.len(), 2);
        assert_eq!(entry.mirrors[0].timestamp, 100);
        assert_eq!(entry.mirrors[1].timestamp, 200);
        assert_eq!(entry.registrars().len(), 2);
        assert_eq!(entry.latest_mirror().unwrap().version, 20260601);
        let st =
            RegistryState::try_from(shards.get(&registry_pda_id(&program())).unwrap()).unwrap();
        assert_eq!(st.region_count(), 1, "same region, no double count");
        assert_eq!(st.registration_count, 2);
    }

    #[test]
    #[should_panic(expected = "outside the predefined region set")]
    fn rejects_region_outside_the_closed_set() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(AccountId::new([0x11; 32]), false); // not the PDA for "narnia"
        run(
            vec![reg, r, signer_meta(REGISTRAR)],
            Instruction::RegisterRegion {
                registration: registration("narnia", 100),
            },
            &mut shards,
        );
    }

    #[test]
    #[should_panic(expected = "outside the predefined region set")]
    fn rejects_decomposed_country_files() {
        // Compile-time guard: none of the decomposed countries are in the set,
        // so registering e.g. `us` hits the closed-set rejection. This test
        // documents that intent explicitly.
        for d in osm_core::set::DECOMPOSED {
            assert!(by_path(d).is_none());
        }
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(AccountId::new([0x22; 32]), false);
        run(
            vec![reg, r, signer_meta(REGISTRAR)],
            Instruction::RegisterRegion {
                registration: registration("us", 100),
            },
            &mut shards,
        );
    }

    #[test]
    #[should_panic(expected = "registrar must sign")]
    fn unsigned_registration_rejected() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(region_pda_id(&program(), "germany"), false);
        let who = meta(AccountId::new(REGISTRAR), false);
        run(
            vec![reg, r, who],
            Instruction::RegisterRegion {
                registration: registration("germany", 100),
            },
            &mut shards,
        );
    }

    #[test]
    #[should_panic(expected = "not the PDA for region")]
    fn wrong_region_account_rejected() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        // The PDA for germany, but the instruction claims france.
        let r = meta(region_pda_id(&program(), "germany"), false);
        run(
            vec![reg, r, signer_meta(REGISTRAR)],
            Instruction::RegisterRegion {
                registration: registration("france", 100),
            },
            &mut shards,
        );
    }

    #[test]
    fn batch_registers_multiple_regions() {
        let mut shards = registry_shard();
        let paths = ["germany", "france", "us/california"];
        let mut accounts = vec![meta(registry_pda_id(&program()), false), signer_meta(REGISTRAR)];
        for p in paths {
            accounts.push(meta(region_pda_id(&program(), p), false));
        }
        let regs: Vec<RegionRegistration> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| registration(p, 100 + i as u64))
            .collect();
        run(
            accounts,
            Instruction::RegisterRegionsBatch {
                registrations: regs,
            },
            &mut shards,
        );
        let st =
            RegistryState::try_from(shards.get(&registry_pda_id(&program())).unwrap()).unwrap();
        assert_eq!(st.region_count(), 3);
        assert_eq!(st.registration_count, 3);
        for p in paths {
            let e = RegionEntry::try_from(shards.get(&region_pda_id(&program(), p)).unwrap())
                .unwrap();
            assert_eq!(e.region, p);
        }
    }

    #[test]
    #[should_panic(expected = "need 1..=24")]
    fn batch_over_cap_rejected() {
        let mut shards = registry_shard();
        let mut accounts = vec![meta(registry_pda_id(&program()), false), signer_meta(REGISTRAR)];
        let regs: Vec<RegionRegistration> = (0..=MAX_BATCH)
            .map(|i| {
                let p = REGIONS[i % REGIONS.len()].path;
                accounts.push(meta(region_pda_id(&program(), p), false));
                registration(p, 100)
            })
            .collect();
        run(
            accounts,
            Instruction::RegisterRegionsBatch {
                registrations: regs,
            },
            &mut shards,
        );
    }

    #[test]
    fn mirror_history_capped_at_max() {
        let mut shards = registry_shard();
        let reg = meta(registry_pda_id(&program()), false);
        let r = meta(region_pda_id(&program(), "kenya"), false);
        for i in 0..(MAX_MIRRORS as u64 + 3) {
            run(
                vec![reg.clone(), r.clone(), signer_meta(REGISTRAR)],
                Instruction::RegisterRegion {
                    registration: RegionRegistration {
                        region: "kenya".into(),
                        cid: format!("cid-{i}"),
                        checksum: [0; 16],
                        version: 20260101,
                        timestamp: i + 1,
                    },
                },
                &mut shards,
            );
        }
        let entry =
            RegionEntry::try_from(shards.get(&region_pda_id(&program(), "kenya")).unwrap()).unwrap();
        assert_eq!(entry.mirrors.len(), MAX_MIRRORS);
        // Oldest pruned; the latest is the newest registration.
        assert_eq!(entry.mirrors[0].timestamp, 4);
        assert_eq!(entry.latest_mirror().unwrap().timestamp, MAX_MIRRORS as u64 + 3);
    }
}
