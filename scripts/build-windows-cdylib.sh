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

# 2. rapidsnark windows statics (Logos fork release). The tarball extracts
# NESTED (rapidsnark-windows-x86_64-<ver>/...); flatten it so the statics
# land where RAPIDSNARK_LIB_DIR expects them ($RSN_DIR/lib/). Both the
# download and the flatten are idempotent — a re-extracted tarball restores
# the nested layout and gets re-flattened.
RSN_DIR="$WORK/rapidsnark-win"
if [ ! -f "$RSN_DIR/lib/librapidsnark.a" ] && [ ! -d "$RSN_DIR/rapidsnark-windows-x86_64-v0.0.8-logos.1-rc.4" ]; then
  mkdir -p "$RSN_DIR"
  curl -sL -o "$RSN_DIR/rs.tar.gz" \
    "https://github.com/logos-blockchain/logos-blockchain-rapidsnark/releases/download/v0.0.8-logos.1-rc.4/rapidsnark-windows-x86_64-v0.0.8-logos.1-rc.4.tar.gz"
  tar xzf "$RSN_DIR/rs.tar.gz" -C "$RSN_DIR"
fi
if [ ! -f "$RSN_DIR/lib/librapidsnark.a" ] && ls "$RSN_DIR"/rapidsnark-windows-x86_64-*/lib/librapidsnark.a >/dev/null 2>&1; then
  cp -r "$RSN_DIR"/rapidsnark-windows-x86_64-*/. "$RSN_DIR/"
  rm -rf "$RSN_DIR"/rapidsnark-windows-x86_64-*
fi

# 2b. gcc/cargo temp files: keep them off any small tmpfs (/tmp here is a
# 7.6G tmpfs with a user quota; assembler temps blew it on 2026-11-04).
# $WORK defaults under /tmp, so point TMPDIR at the repo's target dir which
# lives on the big disk.
export TMPDIR="$PWD/target/gcc-tmp"
mkdir -p "$TMPDIR"

# 3. logos-blockchain-circuits windows artifact. mingw's -l flags need
# lib<name>.a; the tarball ships <circuit>.lib. The rename runs outside the
# download branch on purpose: a re-extracted tarball restores the .lib
# names and must be re-renamed.
LBC_DIR="$WORK/lbc-root/logos-blockchain-circuits-v0.5.3-windows-x86_64"
if [ ! -f "$LBC_DIR/lib/libgmp.a" ]; then
  mkdir -p "$WORK/lbc-root"
  curl -sL -o "$WORK/lbc.tar.gz" \
    "https://github.com/logos-blockchain/logos-blockchain-circuits/releases/download/v0.5.3/logos-blockchain-circuits-v0.5.3-windows-x86_64.tar.gz"
  tar xzf "$WORK/lbc.tar.gz" -C "$WORK/lbc-root"
fi
for c in pol poq poc signature; do
  if [ -f "$LBC_DIR/$c/$c.lib" ] && [ ! -f "$LBC_DIR/$c/lib$c.a" ]; then
    mv "$LBC_DIR/$c/$c.lib" "$LBC_DIR/$c/lib$c.a"
  fi
done

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
