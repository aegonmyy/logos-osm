//! Real guest cycle-count profile for the OSM region-registry program.
//!
//! Runs each instruction's exact on-chain scenario through the local RISC0
//! executor (`ExecutorImpl`, no proving) and reports the session cycle count
//! for each — the numbers behind `docs/CU_COSTS.md`. Ignored by default: it
//! needs the `prove` feature build (already enabled in this crate's dev-deps)
//! and takes ~seconds per instruction once compiled.
//!
//! Run:
//!   cargo test -p osm-integration-tests --test cycle_profile -- --nocapture

use lee_core::account::{Account, AccountId, AccountWithMetadata, Data, Nonce};
use lee_core::program::{PdaSeed, ProgramId};
use risc0_zkvm::{ExecutorEnv, ExecutorImpl};

use osm_core::{
    region_seed, Instruction, Mirror, RegionEntry, RegionRegistration, RegistryState, MAX_BATCH,
};

/// The embedded program id (the exact binary the chain runs).
fn prog() -> ProgramId {
    osm_registry::osm_registry_id()
}

const REGISTRY_SEED: &[u8; 32] = b"/OSM/REGISTRY/V1/SEED/0000000000";
const REGISTRAR: [u8; 32] = [9; 32];

fn registry_pda() -> AccountId {
    AccountId::for_public_pda(&prog(), &PdaSeed::new(*REGISTRY_SEED))
}

fn region_account(path: &str) -> AccountId {
    AccountId::for_public_pda(&prog(), &PdaSeed::new(region_seed(path)))
}

fn default_account(id: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: [0u32; 8],
            balance: 0,
            data: Data::default(),
            nonce: Nonce::default(),
        },
        is_authorized: false,
        account_id: id,
    }
}

fn registrar_signer() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: [0u32; 8],
            balance: 0,
            data: Data::default(),
            nonce: Nonce::default(),
        },
        is_authorized: true,
        account_id: AccountId::new(REGISTRAR),
    }
}

fn registry_state(state: &RegistryState) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: prog(),
            balance: 0,
            data: Data::try_from(borsh::to_vec(state).unwrap()).unwrap(),
            nonce: Nonce::default(),
        },
        is_authorized: false,
        account_id: registry_pda(),
    }
}

fn region_entry(path: &str, entry: &RegionEntry) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: prog(),
            balance: 0,
            data: Data::try_from(borsh::to_vec(entry).unwrap()).unwrap(),
            nonce: Nonce::default(),
        },
        is_authorized: false,
        account_id: region_account(path),
    }
}

/// One measured execution: user cycles (the guest's own work, no po2 padding)
/// and total cycles (what a prover pays, including padding/segment overhead).
#[derive(Clone, Copy, Debug)]
struct Measured {
    user_cycles: u64,
    total_cycles: u64,
}

/// Run one guest session for `instruction` over `pre_states`; return cycles.
fn run_cycles(
    instruction: &Instruction,
    pre_states: &[AccountWithMetadata],
) -> anyhow::Result<Measured> {
    let words = risc0_zkvm::serde::to_vec(instruction)
        .map_err(|e| anyhow::anyhow!("encode instruction: {e:?}"))?;
    let env = ExecutorEnv::builder()
        .write(&prog())?
        .write(&Option::<ProgramId>::None)?
        .write(&pre_states.to_vec())?
        .write(&words)?
        .build()?;
    let mut exec = ExecutorImpl::from_elf(env, osm_registry::osm_registry_elf())?;
    let session = exec.run()?;
    Ok(Measured {
        user_cycles: session.user_cycles,
        total_cycles: session.total_cycles,
    })
}

fn report(label: &str, m: Measured) {
    println!(
        "{label:<32}: {:>10} user {:>10} total cycles",
        m.user_cycles, m.total_cycles
    );
}

fn registration(region: &str, version: u32, ts: u64) -> RegionRegistration {
    RegionRegistration {
        region: region.into(),
        cid: format!("cid-{region}"),
        checksum: [0x5a; 16],
        version,
        timestamp: ts,
    }
}

#[test]
#[ignore = "heavy local executor build; run explicitly (see docs/CU_COSTS.md)"]
fn cycle_profile_per_instruction() -> anyhow::Result<()> {
    let owner = [7u8; 32];

    // Init: registry PDA default + owner signer.
    let init_m = run_cycles(
        &Instruction::Init { owner },
        &[
            default_account(registry_pda()),
            AccountWithMetadata {
                account: Account {
                    program_owner: [0u32; 8],
                    balance: 0,
                    data: Data::default(),
                    nonce: Nonce::default(),
                },
                is_authorized: true,
                account_id: AccountId::new(owner),
            },
        ],
    )?;
    report("Init", init_m);

    let initialized = RegistryState {
        owner,
        region_count: 0,
        registration_count: 0,
        initialized: 1,
    };

    // RegisterRegion (single, fresh region).
    let fresh_m = run_cycles(
        &Instruction::RegisterRegion {
            registration: registration("germany", 20260821, 1_788_000_000),
        },
        &[
            registry_state(&initialized),
            default_account(region_account("germany")),
            registrar_signer(),
        ],
    )?;
    report("RegisterRegion (fresh)", fresh_m);

    // RegisterRegion (append: existing entry with 2 mirrors — the history
    // re-encode cost grows with retained mirrors).
    let existing = RegionEntry {
        region: "germany".into(),
        parent: String::new(),
        level: 0,
        mirrors: vec![
            Mirror {
                registrar: [1; 32],
                cid: "cid-1".into(),
                checksum: [0; 16],
                version: 20260701,
                timestamp: 100,
                hosted: true,
            },
            Mirror {
                registrar: [2; 32],
                cid: "cid-2".into(),
                checksum: [0; 16],
                version: 20260801,
                timestamp: 200,
                hosted: true,
            },
        ],
    };
    let append_m = run_cycles(
        &Instruction::RegisterRegion {
            registration: registration("germany", 20260821, 1_788_000_000),
        },
        &[
            registry_state(&initialized),
            region_entry("germany", &existing),
            registrar_signer(),
        ],
    )?;
    report("RegisterRegion (append, 2 mirrors)", append_m);

    // Batch of 10.
    let batch10: Vec<RegionRegistration> = (0..10)
        .map(|i| {
            registration(
                osm_core::set::REGIONS[i].path,
                20260821,
                1_788_000_000 + i as u64,
            )
        })
        .collect();
    let mut pre10 = vec![registry_state(&initialized), registrar_signer()];
    for r in &batch10 {
        pre10.push(default_account(region_account(&r.region)));
    }
    let batch10_m = run_cycles(
        &Instruction::RegisterRegionsBatch {
            registrations: batch10,
        },
        &pre10,
    )?;
    report("RegisterRegionsBatch (x10)", batch10_m);

    // Batch at the cap (24).
    let batch24: Vec<RegionRegistration> = (0..MAX_BATCH)
        .map(|i| {
            registration(
                osm_core::set::REGIONS[i].path,
                20260821,
                1_788_000_000 + i as u64,
            )
        })
        .collect();
    let mut pre24 = vec![registry_state(&initialized), registrar_signer()];
    for r in &batch24 {
        pre24.push(default_account(region_account(&r.region)));
    }
    let batch24_m = run_cycles(
        &Instruction::RegisterRegionsBatch {
            registrations: batch24,
        },
        &pre24,
    )?;
    report("RegisterRegionsBatch (x24, cap)", batch24_m);

    println!();
    println!(
        "per-region, single RegisterRegion: {} user / {} total cycles",
        fresh_m.user_cycles, fresh_m.total_cycles
    );
    println!(
        "per-region, batch of 24          : {} user / {} total cycles (÷24 = {} total/region)",
        batch24_m.user_cycles,
        batch24_m.total_cycles,
        batch24_m.total_cycles / MAX_BATCH as u64
    );
    Ok(())
}
