//! Real guest cycle-count profile for the OSM region-registry program.
//!
//! Runs each instruction's exact on-chain scenario through the local RISC0
//! executor (`ExecutorImpl`, no proving) and reports the session cycle count
//! for each — the numbers behind `docs/CU_COSTS.md`.
//!
//! ## v0.3 shape
//!
//! Under the v0.3 program model each instruction costs **two** guest
//! invocations: a `Plan` call that sees the instruction and the account
//! metadata, and one `Apply` call per shard the plan emits. This profile
//! measures both and reports them separately, then the total a transaction
//! pays (plan + every apply), which is the figure that maps to CU.
//!
//! Ignored by default: it needs the `prove` feature build (already enabled in
//! this crate's dev-deps) and takes ~seconds per instruction once compiled.
//!
//! Run:
//!   cargo test -p osm-integration-tests --test cycle_profile -- --nocapture

use anyhow::{Result, bail};
use lee::program::Program;
use lee_core::account::{AccountId, ShardData};
use lee_core::from_frame;
use lee_core::program::{
    AccountMeta, ApplyInput, GuestOutput, PdaSeed, PlanInput, ProgramId, ShardEffect,
};
use risc0_zkvm::{ExecutorEnv, ExecutorImpl};

use osm_core::{
    Instruction, MAX_BATCH, Mirror, RegionEntry, RegionRegistration, RegistryState, region_seed,
};

/// The embedded program id (the exact binary the chain runs).
fn prog() -> ProgramId {
    osm_registry::osm_registry_id()
}

const REGISTRY_SEED: &[u8; 32] = b"/OSM/REGISTRY/V1/SEED/0000000000";
const REGISTRAR: [u8; 32] = [9; 32];

/// The program's own account id, as the chain derives it.
fn program_account() -> AccountId {
    AccountId::from_builtin_program(prog())
}

fn registry_pda() -> AccountId {
    AccountId::for_public_pda(&program_account(), &PdaSeed::new(*REGISTRY_SEED))
}

fn region_account(path: &str) -> AccountId {
    AccountId::for_public_pda(&program_account(), &PdaSeed::new(region_seed(path)))
}

/// Account metadata for an account whose shard this program writes.
fn meta(id: AccountId) -> AccountMeta {
    AccountMeta::new(id, false, program_account())
}

/// Account metadata for the signing registrar.
fn registrar_signer() -> AccountMeta {
    AccountMeta::new(AccountId::new(REGISTRAR), true, program_account())
}

/// One fixture: the account metadata the plan sees, and the pre-data its shard
/// carries into the apply phase.
fn fixture(account: AccountMeta, data: ShardData) -> (AccountMeta, ShardData) {
    (account, data)
}

fn registry_fixture(state: &RegistryState) -> (AccountMeta, ShardData) {
    (
        meta(registry_pda()),
        ShardData::from(state),
    )
}

fn region_fixture(path: &str, entry: &RegionEntry) -> (AccountMeta, ShardData) {
    (
        meta(region_account(path)),
        ShardData::from(entry),
    )
}

fn empty_region_fixture(path: &str) -> (AccountMeta, ShardData) {
    (meta(region_account(path)), ShardData::default())
}

/// One measured execution: user cycles (the guest's own work, no po2 padding)
/// and total cycles (what a prover pays, including padding/segment overhead).
#[derive(Clone, Copy, Debug, Default)]
struct Measured {
    user_cycles: u64,
    total_cycles: u64,
}

impl Measured {
    fn add(self, other: Measured) -> Measured {
        Measured {
            user_cycles: self.user_cycles.saturating_add(other.user_cycles),
            total_cycles: self.total_cycles.saturating_add(other.total_cycles),
        }
    }
}

fn decode_journal<T: borsh::BorshDeserialize>(journal: &[u8]) -> Result<T> {
    let payload = from_frame(journal).ok_or_else(|| anyhow::anyhow!("malformed journal frame"))?;
    Ok(borsh::from_slice(payload)?)
}

/// Run the Plan phase: returns the measured cycles and the emitted effects.
fn run_plan(input: &PlanInput) -> Result<(Measured, Vec<ShardEffect>)> {
    let mut builder = ExecutorEnv::builder();
    Program::write_plan_inputs(input, &mut builder)?;
    let env = builder.build()?;
    let mut exec = ExecutorImpl::from_elf(env, osm_registry::osm_registry_elf())?;
    let session = exec.run()?;
    let measured = Measured {
        user_cycles: session.user_cycles,
        total_cycles: session.total_cycles,
    };
    let journal = session
        .journal
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("plan session produced no journal"))?;
    let output: GuestOutput = decode_journal(&journal.bytes)?;
    let GuestOutput::Plan(plan) = output else {
        bail!("plan phase produced an apply output");
    };
    Ok((measured, plan.effects))
}

/// Run one Apply phase: returns the measured cycles and the shard's post-data.
fn run_apply(input: &ApplyInput) -> Result<(Measured, Option<ShardData>)> {
    let mut builder = ExecutorEnv::builder();
    Program::write_apply_inputs(input, &mut builder)?;
    let env = builder.build()?;
    let mut exec = ExecutorImpl::from_elf(env, osm_registry::osm_registry_elf())?;
    let session = exec.run()?;
    let measured = Measured {
        user_cycles: session.user_cycles,
        total_cycles: session.total_cycles,
    };
    let journal = session
        .journal
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("apply session produced no journal"))?;
    let output: GuestOutput = decode_journal(&journal.bytes)?;
    let GuestOutput::Apply(apply) = output else {
        bail!("apply phase produced a plan output");
    };
    Ok((measured, apply.post_data))
}

/// Run a whole instruction the way the chain does: one Plan, then one Apply per
/// emitted effect. Returns (plan cycles, apply cycles, total cycles).
fn run_scenario(
    instruction: &Instruction,
    fixtures: Vec<(AccountMeta, ShardData)>,
) -> Result<(Measured, Measured)> {
    let mut shards: std::collections::HashMap<AccountId, ShardData> = fixtures
        .iter()
        .map(|(account, data)| (account.account_id, data.clone()))
        .collect();

    let input = PlanInput {
        self_account_id: program_account(),
        caller_account_id: None,
        accounts: fixtures.into_iter().map(|(account, _)| account).collect(),
        instruction_data: borsh::to_vec(instruction)?,
    };

    let (plan_m, effects) = run_plan(&input)?;

    let mut apply_m = Measured::default();
    for effect in effects {
        let apply_input = ApplyInput {
            self_account_id: program_account(),
            selector: effect.selector,
            pre_data: shards
                .get(&effect.selector.account_id)
                .cloned()
                .unwrap_or_default(),
            effect_data: effect.data,
        };
        let (m, post) = run_apply(&apply_input)?;
        apply_m = apply_m.add(m);
        if let Some(post_data) = post {
            shards.insert(effect.selector.account_id, post_data);
        }
    }
    Ok((plan_m, apply_m))
}

fn report(label: &str, plan: Measured, apply: Measured) {
    let total = plan.add(apply);
    println!(
        "{label:<34}: plan {:>9}u/{:>9}t  apply {:>9}u/{:>9}t  total {:>10} cycles",
        plan.user_cycles, plan.total_cycles, apply.user_cycles, apply.total_cycles, total.total_cycles
    );
}

fn registration(region: &str, version: u32, ts: u64) -> RegionRegistration {
    RegionRegistration {
        region: region.into(),
        cid: format!("cid-{region}"),
        source_url: osm_core::set::by_path(region)
            .map(|r| r.source_url().to_string())
            .unwrap_or_default(),
        checksum: [0x5a; 16],
        version,
        timestamp: ts,
    }
}

#[test]
#[ignore = "heavy local executor build; run explicitly (see docs/CU_COSTS.md)"]
fn cycle_profile_per_instruction() -> Result<()> {
    let owner = [7u8; 32];

    // Init: registry PDA default + owner signer.
    let (plan, apply) = run_scenario(
        &Instruction::Init { owner },
        vec![
            fixture(meta(registry_pda()), ShardData::default()),
            fixture(
                AccountMeta::new(AccountId::new(owner), true, program_account()),
                ShardData::default(),
            ),
        ],
    )?;
    report("Init", plan, apply);

    let initialized = RegistryState {
        owner,
        regions: Vec::new(),
        registration_count: 0,
        initialized: 1,
    };

    // RegisterRegion (single, fresh region).
    let (plan, apply) = run_scenario(
        &Instruction::RegisterRegion {
            registration: registration("germany", 20260821, 1_788_000_000),
        },
        vec![
            registry_fixture(&initialized),
            empty_region_fixture("germany"),
            fixture(registrar_signer(), ShardData::default()),
        ],
    )?;
    report("RegisterRegion (fresh)", plan, apply);

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
                source_url: "https://download.geofabrik.de/europe/germany-latest.osm.pbf".into(),
                checksum: [0; 16],
                version: 20260701,
                timestamp: 100,
                hosted: true,
            },
            Mirror {
                registrar: [2; 32],
                cid: "cid-2".into(),
                source_url: "https://download.geofabrik.de/europe/germany-latest.osm.pbf".into(),
                checksum: [0; 16],
                version: 20260801,
                timestamp: 200,
                hosted: true,
            },
        ],
    };
    let (plan, apply) = run_scenario(
        &Instruction::RegisterRegion {
            registration: registration("germany", 20260821, 1_788_000_000),
        },
        vec![
            registry_fixture(&initialized),
            region_fixture("germany", &existing),
            fixture(registrar_signer(), ShardData::default()),
        ],
    )?;
    report("RegisterRegion (append, 2 mirrors)", plan, apply);

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
    let mut fixtures10 = vec![
        registry_fixture(&initialized),
        fixture(registrar_signer(), ShardData::default()),
    ];
    for r in &batch10 {
        fixtures10.push(empty_region_fixture(&r.region));
    }
    let (plan10, apply10) = run_scenario(
        &Instruction::RegisterRegionsBatch {
            registrations: batch10,
        },
        fixtures10,
    )?;
    report("RegisterRegionsBatch (x10)", plan10, apply10);

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
    let mut fixtures24 = vec![
        registry_fixture(&initialized),
        fixture(registrar_signer(), ShardData::default()),
    ];
    for r in &batch24 {
        fixtures24.push(empty_region_fixture(&r.region));
    }
    let (plan24, apply24) = run_scenario(
        &Instruction::RegisterRegionsBatch {
            registrations: batch24,
        },
        fixtures24,
    )?;
    report("RegisterRegionsBatch (x24, cap)", plan24, apply24);

    let total24 = plan24.add(apply24);
    println!();
    println!(
        "per-region, batch of 24: {} total cycles (÷24 = {} total/region)",
        total24.total_cycles,
        total24.total_cycles / MAX_BATCH as u64
    );
    Ok(())
}
