//! LEZ RISC0 guest: the OSM region registry.
//!
//! ## Account model
//!
//! Two families of PDA accounts (derived from `(self_program_id, seed)`):
//! - a single **registry** PDA (seed [`REGISTRY_SEED`]) holding
//!   [`RegistryState`]; and
//! - one **region** PDA per predefined region (seed = `SHA-256("osm-region:"
//!   || path)`) holding [`RegionEntry`] — the region's mirrors with full
//!   version history.
//!
//! Registration is **permissionless**: any signer may mirror a region. The
//! prize's adoption criteria (>= 3 distinct registrars per covered region,
//! version advancement over time) are computed directly from these accounts,
//! so the chain is the adoption ledger.
//!
//! The guest embeds the frozen region table (`osm_core::set::REGIONS`) and
//! rejects any path outside it, so the chain itself enforces the
//! non-overlapping partition (appendix A.4). `parent` and `level` are taken
//! from the embedded table — never from the client's claim.
//!
//! ## Instructions
//!
//! See [`osm_core::Instruction`]. The dispatch lives in [`execute`], a pure
//! function over `(pre_states, instruction, self_program_id)` returning
//! post-states; `main()` wires it to the RISC0 host bridge. This split lets
//! the exact on-chain logic run as ordinary host unit tests (`#[cfg(test)]`).

use lee_core::account::{AccountId, AccountWithMetadata, Data};
use lee_core::program::{
    read_lee_inputs, AccountPostState, Claim, PdaSeed, ProgramId, ProgramOutput,
};

use osm_core::set::{Level, REGIONS};
use osm_core::{
    region_seed, Instruction, Mirror, RegionEntry, RegionRegistration, RegistryState, MAX_BATCH,
    MAX_MIRRORS,
};

/// Fixed PDA seed for the single registry account.
pub const REGISTRY_SEED: [u8; 32] = *b"/OSM/REGISTRY/V1/SEED/0000000000";

fn main() {
    let (
        lee_core::program::ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction,
        },
        instruction_words,
    ) = read_lee_inputs::<Instruction>();

    let post_states = execute(self_program_id, &pre_states, &instruction);
    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_words,
        pre_states,
        post_states,
    )
    .write();
}

/// Derive the registry PDA account id for `program_id`.
fn registry_pda_id(program_id: &ProgramId) -> AccountId {
    AccountId::for_public_pda(program_id, &PdaSeed::new(REGISTRY_SEED))
}

/// Derive a region PDA account id for `(program_id, region_path)`.
fn region_pda_id(program_id: &ProgramId, region_path: &str) -> AccountId {
    AccountId::for_public_pda(program_id, &PdaSeed::new(region_seed(region_path)))
}

/// Decode a borsh-serialized `T` from account `data`, or `None` if empty.
fn decode<T: borsh::BorshDeserialize>(data: &Data) -> Option<T> {
    if data.as_ref().is_empty() {
        return None;
    }
    borsh::from_slice(data.as_ref()).ok()
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

/// The pure on-chain dispatch. Returns post-states, or panics (rejecting the
/// transaction) on any violation.
fn execute(
    self_program_id: ProgramId,
    pre_states: &[AccountWithMetadata],
    instruction: &Instruction,
) -> Vec<AccountPostState> {
    match instruction {
        Instruction::Init { owner } => {
            let [reg, owner_acc] = exactly(pre_states);
            assert!(
                reg.account_id == registry_pda_id(&self_program_id),
                "Init: first account must be the registry PDA"
            );
            assert!(
                reg.account.data.as_ref().is_empty(),
                "Init: registry already initialized"
            );
            assert!(owner_acc.is_authorized, "Init: owner must sign");
            assert_eq!(
                owner_acc.account_id.value(),
                owner,
                "Init: signer does not match owner in instruction"
            );

            let mut registry = reg.account.clone();
            registry.data = Data::try_from(
                borsh::to_vec(&RegistryState {
                    owner: *owner,
                    region_count: 0,
                    registration_count: 0,
                    initialized: 1,
                })
                .unwrap(),
            )
            .unwrap();

            // Claim the owner account (Claim::Authorized) so the registry
            // program owns it — the recurring-signer pattern (see the vault's
            // grant registry): after the first transaction the owner's nonce
            // is no longer default, and an unclaimed account with a default
            // program owner would fail `NonDefaultAccountWithDefaultOwner` on
            // every subsequent call.
            vec![
                AccountPostState::new_claimed(registry, Claim::Pda(PdaSeed::new(REGISTRY_SEED))),
                AccountPostState::new_claimed(owner_acc.account.clone(), Claim::Authorized),
            ]
        }

        Instruction::RegisterRegion { registration } => {
            let [reg, region, registrar] = exactly(pre_states);
            let (reg_acc, entry_acc) =
                apply_registration(reg, region, registrar, registration, &self_program_id);
            vec![
                AccountPostState::new(reg_acc),
                entry_acc,
                registrar_post_state(registrar),
            ]
        }

        Instruction::RegisterRegionsBatch { registrations } => {
            let n = registrations.len();
            assert!(
                n >= 1 && n <= MAX_BATCH,
                "RegisterRegionsBatch: need 1..={MAX_BATCH} registrations, got {n}"
            );
            assert_eq!(
                pre_states.len(),
                2 + n,
                "RegisterRegionsBatch: expected 2 + {n} accounts"
            );
            let reg = &pre_states[0];
            let registrar = &pre_states[1];

            let mut registry: RegistryState = require_registry(reg, &self_program_id);
            assert!(
                registrar.is_authorized,
                "RegisterRegionsBatch: registrar must sign"
            );

            let mut posts = Vec::with_capacity(n);
            for (r, region_pre) in registrations.iter().zip(pre_states[2..].iter()) {
                posts.push(register_one(
                    &mut registry,
                    region_pre,
                    registrar,
                    r,
                    &self_program_id,
                ));
            }
            // The registry account post-state is written once, after the loop.
            let mut reg_out = reg.account.clone();
            reg_out.data = Data::try_from(borsh::to_vec(&registry).unwrap()).unwrap();
            let mut out = Vec::with_capacity(2 + n);
            out.push(AccountPostState::new(reg_out));
            out.push(registrar_post_state(registrar));
            out.extend(posts);
            out
        }
    }
}

/// The registrar's post-state.
///
/// A registrar's **first** registration writes their still-default account,
/// which is fine unclaimed — but the sequencer advances the signer's nonce on
/// inclusion, so the account is non-default afterwards while its program
/// owner is still default. A **subsequent** registration by the same signer
/// (e.g. one region, then a batch) then fails execution validation with
/// `NonDefaultAccountWithDefaultOwner` unless this program claims the
/// account. Registration is permissionless and repeatable by design, so the
/// signer is claimed with `Claim::Authorized` on first touch;
/// `new_claimed_if_default` makes the claim idempotent (accounts this
/// program already owns are written plainly).
fn registrar_post_state(registrar: &AccountWithMetadata) -> AccountPostState {
    AccountPostState::new_claimed_if_default(registrar.account.clone(), Claim::Authorized)
}

/// Shared registration path for both the scalar and batch instructions.
/// Returns the (registry account, region post-state) pair.
#[allow(clippy::too_many_arguments)]
fn apply_registration(
    reg: &AccountWithMetadata,
    region: &AccountWithMetadata,
    registrar: &AccountWithMetadata,
    registration: &RegionRegistration,
    program_id: &ProgramId,
) -> (lee_core::account::Account, AccountPostState) {
    let mut registry: RegistryState = require_registry(reg, program_id);
    let entry_acc = register_one(&mut registry, region, registrar, registration, program_id);
    let mut reg_acc = reg.account.clone();
    reg_acc.data = Data::try_from(borsh::to_vec(&registry).unwrap()).unwrap();
    (reg_acc, entry_acc)
}

/// Apply one registration to `(registry_state, region_account)`, returning
/// the region post-state. Mutates `registry` (counters).
fn register_one(
    registry: &mut RegistryState,
    region: &AccountWithMetadata,
    registrar: &AccountWithMetadata,
    registration: &RegionRegistration,
    program_id: &ProgramId,
) -> AccountPostState {
    let path = registration.region.as_str();
    let seed = region_seed(path);
    assert!(
        region.account_id == AccountId::for_public_pda(program_id, &PdaSeed::new(seed)),
        "RegisterRegion: region account is not the PDA for region {path}"
    );
    assert!(
        registrar.is_authorized,
        "RegisterRegion: registrar must sign"
    );
    // The embedded table is the authority for parent/level, and rejects
    // regions outside the frozen closed set.
    let (parent, level) = require_in_set(path);
    assert!(
        registration.version >= 1_970_010_1 && registration.version <= 9_999_123_1,
        "RegisterRegion: version {version} is not a YYYYMMDD date",
        version = registration.version
    );

    let mirror = Mirror {
        registrar: *registrar.account_id.value(),
        cid: registration.cid.clone(),
        checksum: registration.checksum,
        version: registration.version,
        timestamp: registration.timestamp,
        hosted: true,
    };

    let existing: Option<RegionEntry> = decode(&region.account.data);
    let (mut entry, created) = match existing {
        Some(e) => {
            assert_eq!(
                e.region, path,
                "RegisterRegion: region account holds a different region"
            );
            (e, false)
        }
        None => (
            RegionEntry {
                region: path.to_string(),
                parent: parent.to_string(),
                level,
                mirrors: Vec::new(),
            },
            true,
        ),
    };
    if created {
        registry.region_count = registry.region_count.checked_add(1).unwrap();
    }
    entry.mirrors.push(mirror);
    // Keep mirrors ordered by timestamp (oldest first) for version sorting,
    // and cap retained history at MAX_MIRRORS (prune oldest).
    entry.mirrors.sort_by_key(|m| m.timestamp);
    while entry.mirrors.len() > MAX_MIRRORS {
        entry.mirrors.remove(0);
    }
    registry.registration_count = registry.registration_count.checked_add(1).unwrap();

    let mut acc = region.account.clone();
    acc.data = Data::try_from(borsh::to_vec(&entry).unwrap()).unwrap();
    if created {
        AccountPostState::new_claimed(acc, Claim::Pda(PdaSeed::new(seed)))
    } else {
        AccountPostState::new(acc)
    }
}

/// Decode the registry, verify it is initialized.
fn require_registry(reg: &AccountWithMetadata, program_id: &ProgramId) -> RegistryState {
    assert!(
        reg.account_id == registry_pda_id(program_id),
        "registry account is not the registry PDA"
    );
    let registry: RegistryState = decode(&reg.account.data).expect("registry not initialized");
    assert_eq!(registry.initialized, 1, "registry not initialized");
    registry
}

/// Assert the pre-states slice has exactly `n` entries and return them.
fn exactly<const N: usize>(pre: &[AccountWithMetadata]) -> [&AccountWithMetadata; N] {
    pre.iter()
        .collect::<Vec<&AccountWithMetadata>>()
        .try_into()
        .expect("wrong number of accounts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lee_core::account::{Account, AccountId, Data, Nonce};
    use osm_core::set::by_path;

    const PROG: ProgramId = [1, 0, 0, 0, 0, 0, 0, 0];
    const OWNER: [u8; 32] = [7; 32];
    const REGISTRAR: [u8; 32] = [9; 32];

    fn default_account(id: AccountId) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: [0; 8],
                balance: 0,
                data: Data::default(),
                nonce: Nonce::default(),
            },
            is_authorized: false,
            account_id: id,
        }
    }

    fn signer(id: [u8; 32]) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: [0; 8],
                balance: 0,
                data: Data::default(),
                nonce: Nonce::default(),
            },
            is_authorized: true,
            account_id: AccountId::new(id),
        }
    }

    fn registry_pda() -> AccountId {
        AccountId::for_public_pda(&PROG, &PdaSeed::new(REGISTRY_SEED))
    }

    fn initialized_registry() -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                data: Data::try_from(
                    borsh::to_vec(&RegistryState {
                        owner: OWNER,
                        region_count: 0,
                        registration_count: 0,
                        initialized: 1,
                    })
                    .unwrap(),
                )
                .unwrap(),
                ..Account::default()
            },
            is_authorized: false,
            account_id: registry_pda(),
        }
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

    fn region_account(path: &str) -> AccountWithMetadata {
        default_account(region_pda_id(&PROG, path))
    }

    #[test]
    fn init_binds_owner() {
        let reg = default_account(registry_pda());
        let owner = signer(OWNER);
        let posts = execute(PROG, &[reg, owner], &Instruction::Init { owner: OWNER });
        let st: RegistryState = borsh::from_slice(posts[0].account().data.as_ref()).unwrap();
        assert_eq!(st.owner, OWNER);
        assert_eq!(st.initialized, 1);
        assert_eq!(st.region_count, 0);
    }

    #[test]
    fn register_creates_entry_from_embedded_table() {
        let reg = initialized_registry();
        let r = region_account("us/california");
        let who = signer(REGISTRAR);
        let posts = execute(
            PROG,
            &[reg, r, who],
            &Instruction::RegisterRegion {
                registration: registration("us/california", 100),
            },
        );
        let entry: RegionEntry = borsh::from_slice(posts[1].account().data.as_ref()).unwrap();
        assert_eq!(entry.region, "us/california");
        // parent/level come from the table, not the client.
        assert_eq!(entry.parent, "us");
        assert_eq!(entry.level, 1);
        assert_eq!(entry.mirrors.len(), 1);
        assert_eq!(entry.mirrors[0].registrar, REGISTRAR);
        let st: RegistryState = borsh::from_slice(posts[0].account().data.as_ref()).unwrap();
        assert_eq!(st.region_count, 1);
        assert_eq!(st.registration_count, 1);
    }

    #[test]
    fn second_registrar_appends_and_keeps_history() {
        let mut reg = initialized_registry();
        let r = region_account("germany");
        let a = signer(REGISTRAR);
        let posts = execute(
            PROG,
            &[reg.clone(), r.clone(), a],
            &Instruction::RegisterRegion {
                registration: registration("germany", 100),
            },
        );
        // Feed the post-state region account back in for the second registrar.
        let r_after = AccountWithMetadata {
            account: posts[1].account().clone(),
            is_authorized: false,
            account_id: region_pda_id(&PROG, "germany"),
        };
        reg.account = posts[0].account().clone();
        let b = signer([0xdd; 32]);
        let posts = execute(
            PROG,
            &[reg, r_after, b],
            &Instruction::RegisterRegion {
                registration: RegionRegistration {
                    region: "germany".into(),
                    cid: "cid-v2".into(),
                    checksum: [0xcd; 16],
                    version: 20260601,
                    timestamp: 200,
                },
            },
        );
        let entry: RegionEntry = borsh::from_slice(posts[1].account().data.as_ref()).unwrap();
        assert_eq!(entry.mirrors.len(), 2);
        // Timestamp-ordered.
        assert_eq!(entry.mirrors[0].timestamp, 100);
        assert_eq!(entry.mirrors[1].timestamp, 200);
        assert_eq!(entry.registrars().len(), 2);
        assert_eq!(entry.latest_mirror().unwrap().version, 20260601);
        let st: RegistryState = borsh::from_slice(posts[0].account().data.as_ref()).unwrap();
        assert_eq!(st.region_count, 1, "same region, no double count");
        assert_eq!(st.registration_count, 2);
    }

    #[test]
    #[should_panic(expected = "outside the predefined region set")]
    fn rejects_region_outside_the_closed_set() {
        let reg = initialized_registry();
        let r = region_account("narnia");
        let who = signer(REGISTRAR);
        execute(
            PROG,
            &[reg, r, who],
            &Instruction::RegisterRegion {
                registration: registration("narnia", 100),
            },
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
        let reg = initialized_registry();
        let r = region_account("us");
        let who = signer(REGISTRAR);
        execute(
            PROG,
            &[reg, r, who],
            &Instruction::RegisterRegion {
                registration: registration("us", 100),
            },
        );
    }

    #[test]
    fn registrar_claimed_on_first_touch_only() {
        // Rule-7 reproduction at the pure-execute level: the FIRST
        // registration of a still-default signer must request
        // Claim::Authorized (after inclusion the signer's nonce makes them
        // non-default, and an unclaimed re-registration would fail with
        // `NonDefaultAccountWithDefaultOwner`); once this program already
        // owns the account, the write is plain.
        let reg = initialized_registry();
        let r = region_account("germany");
        let fresh = signer(REGISTRAR);
        assert_eq!(
            fresh.account.program_owner, [0; 8],
            "precondition: first-time registrar is default"
        );
        let posts = execute(
            PROG,
            &[reg.clone(), r.clone(), fresh],
            &Instruction::RegisterRegion {
                registration: registration("germany", 100),
            },
        );
        assert_eq!(
            posts[2].required_claim(),
            Some(Claim::Authorized),
            "first registration must claim the signer"
        );

        // Second registration: program_owner non-default => plain write.
        let mut owned = signer(REGISTRAR);
        owned.account.program_owner = PROG;
        let posts = execute(
            PROG,
            &[reg, r, owned],
            &Instruction::RegisterRegion {
                registration: RegionRegistration {
                    region: "germany".into(),
                    cid: "cid-v2".into(),
                    checksum: [0xcd; 16],
                    version: 20260601,
                    timestamp: 200,
                },
            },
        );
        assert_eq!(
            posts[2].required_claim(),
            None,
            "already-owned signer is written plainly"
        );
    }

    #[test]
    #[should_panic(expected = "registrar must sign")]
    fn unsigned_registration_rejected() {
        let reg = initialized_registry();
        let r = region_account("germany");
        let who = default_account(AccountId::new(REGISTRAR));
        execute(
            PROG,
            &[reg, r, who],
            &Instruction::RegisterRegion {
                registration: registration("germany", 100),
            },
        );
    }

    #[test]
    #[should_panic(expected = "not the PDA for region")]
    fn wrong_region_account_rejected() {
        let reg = initialized_registry();
        let r = region_account("germany");
        let who = signer(REGISTRAR);
        execute(
            PROG,
            &[reg, r, who],
            &Instruction::RegisterRegion {
                registration: registration("france", 100),
            },
        );
    }

    #[test]
    fn batch_registers_multiple_regions() {
        let reg = initialized_registry();
        let who = signer(REGISTRAR);
        let paths = ["germany", "france", "us/california"];
        let mut pre = vec![reg, who];
        for p in paths {
            pre.push(region_account(p));
        }
        let regs: Vec<RegionRegistration> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| registration(p, 100 + i as u64))
            .collect();
        let posts = execute(
            PROG,
            &pre,
            &Instruction::RegisterRegionsBatch {
                registrations: regs,
            },
        );
        assert_eq!(posts.len(), 5);
        let st: RegistryState = borsh::from_slice(posts[0].account().data.as_ref()).unwrap();
        assert_eq!(st.region_count, 3);
        assert_eq!(st.registration_count, 3);
        for (i, p) in paths.iter().enumerate() {
            let e: RegionEntry = borsh::from_slice(posts[2 + i].account().data.as_ref()).unwrap();
            assert_eq!(e.region, *p);
        }
    }

    #[test]
    #[should_panic(expected = "need 1..=24")]
    fn batch_over_cap_rejected() {
        let reg = initialized_registry();
        let who = signer(REGISTRAR);
        let mut pre = vec![reg, who];
        let regs: Vec<RegionRegistration> = (0..=MAX_BATCH)
            .map(|i| {
                let p = REGIONS[i % REGIONS.len()].path;
                pre.push(region_account(p));
                registration(p, 100)
            })
            .collect();
        execute(
            PROG,
            &pre,
            &Instruction::RegisterRegionsBatch {
                registrations: regs,
            },
        );
    }

    #[test]
    fn mirror_history_capped_at_max() {
        let mut reg = initialized_registry();
        let mut r = region_account("kenya");
        let who = signer(REGISTRAR);
        for i in 0..(MAX_MIRRORS as u64 + 3) {
            let posts = execute(
                PROG,
                &[reg.clone(), r.clone(), who.clone()],
                &Instruction::RegisterRegion {
                    registration: RegionRegistration {
                        region: "kenya".into(),
                        cid: format!("cid-{i}"),
                        checksum: [0; 16],
                        version: 20260101,
                        timestamp: i + 1,
                    },
                },
            );
            reg.account = posts[0].account().clone();
            r.account = posts[1].account().clone();
        }
        let entry: RegionEntry = borsh::from_slice(r.account.data.as_ref()).unwrap();
        assert_eq!(entry.mirrors.len(), MAX_MIRRORS);
        // Oldest pruned; the latest is the newest registration.
        assert_eq!(entry.mirrors[0].timestamp, 4);
        assert_eq!(
            entry.latest_mirror().unwrap().timestamp,
            MAX_MIRRORS as u64 + 3
        );
    }
}
