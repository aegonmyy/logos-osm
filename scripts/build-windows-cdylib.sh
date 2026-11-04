#!/usr/bin/env bash
# scripts/build-windows-cdylib.sh — build logos_osm.dll by cross-compiling
# from Linux with the posix-thread mingw toolchain.
#
# Why this recipe exists (each line is a wall that was hit):
#   - rapidsnark v0.0.8 has no shared library for Windows and prebuilts only
#     for the GNU toolchain -> we cross x86_64-pc-windows-gnu (not msvc) with
#     the static-rapidsnark feature, enabled for windows via osm-sdk's
#     cfg(windows) dependency; the pinned rev predates windows support, so a
#     patched vendored copy lives at vendor/rapidsnark.
#   - Ubuntu's DEFAULT mingw is the win32-threads model, whose libstdc++ has
#     broken std::mutex/condition_variable -> RocksDB fails to compile. The
#     -posix suffixed compilers fix it.
#   - The LEZ stack's rapidsnark download script has no windows-gnu mapping
#     at our pinned rev -> pre-download the Logos fork's windows static
#     archives and point RAPIDSNARK_LIB_DIR at them.
#   - logos-blockchain-circuits needs LBC_ROOT_DIR with the windows-x86_64
#     prebuilt artifact, and its .lib files must be renamed to lib*.a for
#     mingw's linker.
#   - the Logos fork's static rapidsnark is built against the mcfgthread
#     runtime -> link the official static libmcfgthread (not in Ubuntu's
#     mingw).
#   - risc0-zkvm 3.0.5 does not compile for windows at all (unix-only
#     DefaultProver) -> vendored patch at vendor/risc0-zkvm compiles the
#     module out on windows.
#
# Output: target/x86_64-pc-windows-gnu/release/logos_osm.dll
# Then: cp it to module/vendor/x86_64-windows/ , commit, and dispatch the
# release workflow so the .lgx bundles it.

set -euo pipefail
cd "$(dirname "$0")/.."

WORK="${WINDOWS_CDYLIB_WORK:-/tmp/osm-win-cdylib}"
mkdir -p "$WORK"

# 1. Toolchain (posix-thread model is REQUIRED for rocksdb's std::mutex).
if ! command -v x86_64-w64-mingw32-g++-posix >/dev/null; then
  sudo apt-get install -y -q mingw-w64
fi

# 2. rapidsnark windows statics (Logos fork release).
RSN_DIR="$WORK/rapidsnark-win"
if [ ! -f "$RSN_DIR/lib/librapidsnark.a" ]; then
  mkdir -p "$RSN_DIR"
  curl -sL -o "$RSN_DIR/rs.tar.gz" \
    "https://github.com/logos-blockchain/logos-blockchain-rapidsnark/releases/download/v0.0.8-logos.1-rc.4/rapidsnark-windows-x86_64-v0.0.8-logos.1-rc.4.tar.gz"
  tar xzf "$RSN_DIR/rs.tar.gz" -C "$RSN_DIR"
fi

# 3. logos-blockchain-circuits windows artifact (.lib -> lib*.a renames).
LBC_DIR="$WORK/lbc-root/logos-blockchain-circuits-v0.5.3-windows-x86_64"
if [ ! -f "$LBC_DIR/lib/libgmp.a" ]; then
  mkdir -p "$WORK/lbc-root"
  curl -sL -o "$WORK/lbc.tar.gz" \
    "https://github.com/logos-blockchain/logos-blockchain-circuits/releases/download/v0.5.3/logos-blockchain-circuits-v0.5.3-windows-x86_64.tar.gz"
  tar xzf "$WORK/lbc.tar.gz" -C "$WORK/lbc-root"
  for c in pol poq poc signature; do
    if [ -f "$LBC_DIR/$c/$c.lib" ]; then mv "$LBC_DIR/$c/$c.lib" "$LBC_DIR/$c/lib$c.a"; fi
  done
fi

# 3b. mcfgthread runtime: the Logos fork's windows static rapidsnark is
# built against the mcfgthread threading runtime, whose _MCF_* symbols are
# not in Ubuntu's mingw. Download the official static runtime and place it
# in the rapidsnark lib dir (the link-search path).
if [ ! -f "$RSN_DIR/lib/libmcfgthread.a" ]; then
  curl -sL -o "$RSN_DIR/mcfg.zip" \
    "https://github.com/lhmouse/mcfgthread/releases/download/v2.5-ga.1/x86-64_mingw-w64_gcc.zip"
  (cd "$RSN_DIR" && unzip -oq mcfg.zip && cp lib/libmcfgthread.a lib/ 2>/dev/null || cp mcfg-lib/libmcfgthread.a lib/ 2>/dev/null) || {
    # the zip extracts to ./lib — handle both shapes
    find "$RSN_DIR" -name libmcfgthread.a -exec cp {} "$RSN_DIR/lib/" \;
  }
fi

# 4. Build.
RAPIDSNARK_LIB_DIR="$RSN_DIR/lib" \
LBC_ROOT_DIR="$LBC_DIR" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix \
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix \
CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix \
AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
RUSTFLAGS="-C link-arg=-L$RSN_DIR/lib -C link-arg=-lmcfgthread -C link-arg=-lssp -C link-arg=-lkernel32 -C link-arg=-lntdll" \
cargo build -p logos-osm --release --target x86_64-pc-windows-gnu

DLL="target/x86_64-pc-windows-gnu/release/logos_osm.dll"
test -f "$DLL"
echo "built: $DLL ($(stat -c%s "$DLL") bytes)"
