//! Embed the committed OSM-registry guest artifact.
//!
//! The guest is NOT rebuilt here on purpose: a risc0 guest build is not
//! byte-reproducible across machines — dependency source paths (e.g.
//! `$CARGO_HOME/registry/src/...`) land in panic-location metadata, so the
//! same sources + pinned toolchain produce different ELF bytes (and a
//! different image id) on every checkout host. That drift bit the vault
//! build twice before it switched to committed artifacts (same lesson
//! applied here from day one).
//!
//! Instead, the exact artifact is committed at `osm_registry.bin` (user ELF +
//! risc0 kernel, as produced by risc0_build's `ProgramBinary`), and the
//! program id below is pinned to it. `program_id_is_stable_and_documented`
//! verifies bytes -> id -> docs at test time, so a stale artifact or id fails
//! a test instead of shipping.
//!
//! To rebuild deliberately (guest-source or toolchain change):
//!   cargo run -p osm-guest-builder --release
//! then update the id constant below together with README.md /
//! docs/CU_COSTS.md / STATE.md and the testnet test's expected id, and
//! redeploy the program (the id IS the program address on LEZ).

use std::{env, fs, path::Path};

/// The program id of the committed `osm_registry.bin` (big-endian words).
/// Hex: 77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0
const OSM_REGISTRY_ID: [u32; 8] = [
    2012012335, 2465069982, 3041696483, 4122921503, 1186396581, 3507054203, 1177540972, 2099004400,
];

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let bin = Path::new(&manifest_dir).join("osm_registry.bin");
    assert!(
        bin.exists(),
        "missing committed guest artifact: {}",
        bin.display()
    );
    println!("cargo:rerun-if-changed={}", bin.display());

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("methods.rs");
    fs::write(
        &dest,
        format!(
            "pub const OSM_REGISTRY_ELF: &[u8] = include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/osm_registry.bin\"));\npub const OSM_REGISTRY_ID: [u32; 8] = {OSM_REGISTRY_ID:?};\n"
        ),
    )
    .expect("writing generated methods.rs");
}
