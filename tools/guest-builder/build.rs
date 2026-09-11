//! Build the RISC0 guest for the riscv target and generate the embedded
//! artifact constants. Run deliberately — never from normal CI builds.

fn main() {
    risc0_build::embed_methods();
}
