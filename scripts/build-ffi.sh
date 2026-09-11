#!/usr/bin/env bash
#
# build-ffi.sh — build the OSM module's Rust core (liblogos_osm.so) so it
# loads inside the nix-built Logos Core runtime.
#
# The core is a cargo cdylib that links libstdc++ and libc. For the module to
# load it, those must be the SAME libraries the Logos Core runtime uses — i.e.
# built against the same nixpkgs the Logos module builder pins. This script
# builds it in a nix-shell from that exact nixpkgs revision.
#
# Output: target/ffi/debug/liblogos_osm.so
# Point the module at it with:  export LOGOS_OSM_FFI_PATH=<that path>

set -euo pipefail

# Keep this in sync with the `nixpkgs` node in flake.lock (the revision the
# Logos module builder / logoscore are built from — its glibc must match).
NIXPKGS_REV="e9f00bd893984bc8ce46c895c3bf7cac95331127"
NIXPKGS="https://github.com/NixOS/nixpkgs/archive/${NIXPKGS_REV}.tar.gz"

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$REPO_DIR/target/ffi"

nix-shell -I nixpkgs="$NIXPKGS" -p gcc pkg-config openssl --run "
  set -e
  export CC=\$(command -v gcc)
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=\$(command -v gcc)
  export CARGO_TARGET_DIR='$OUT_DIR'
  export RISC0_DEV_MODE=1
  cd '$REPO_DIR'
  cargo build -p logos-osm --lib
"

SO="$OUT_DIR/debug/liblogos_osm.so"
echo
echo "Built: $SO"
echo "Runtime libraries (should be nix-store paths matching the Logos Core runtime):"
ldd "$SO" | grep -iE "libc.so|libstdc" || true
echo
echo "Load it in Logos Core with:  export LOGOS_OSM_FFI_PATH=$SO"
