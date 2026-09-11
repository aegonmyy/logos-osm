#!/usr/bin/env bash
# scripts/smoke_lgx.sh — prove the packaged OSM module is loadable + callable.
#
# Evidence for the app/module deliverable: the .lgx bundle's payload
# (osm_plugin.so) is NOT a stub. This script extracts it, then drives the
# common module-impl C ABI exactly the way the Logos host (logos_host) would:
# dlopen, negotiate protocol version, enumerate methods, dispatch. The
# generated glue (OsmCdylibPlugin) forwards into OsmImpl, which dlopens the
# Rust core (liblogos_osm.so) over the FFI contract — so a real op result
# round-trips C++ -> Rust and back.
#
# Usage:
#   ./scripts/smoke_lgx.sh                 # builds the .lgx + Rust cdylib if missing
#   OSM_PLUGIN=path/to/osm_plugin.so LOGOS_OSM_FFI_PATH=path/to/liblogos_osm.so \
#     ./scripts/smoke_lgx.sh   # skip the build, point at existing artifacts
#   SMOKE_NO_FFI=1 ./scripts/smoke_lgx.sh  # no Rust core: asserts fail-closed health
set -euo pipefail
cd "$(dirname "$0")/.."

PLUGIN="${OSM_PLUGIN:-}"
FFI="${LOGOS_OSM_FFI_PATH:-}"
if [[ -n "${SMOKE_NO_FFI:-}" ]]; then FFI=""; fi
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ -z "$PLUGIN" ]]; then
  echo ">> building osm-lgx (nix)..."
  nix build .#osm-lgx --out-link "$WORK/osm-lgx" >/dev/null
  LGX="$WORK/osm-lgx/logos-osm-module-lib.lgx"
  echo ">> extracting $LGX"
  tar xzf "$LGX" -C "$WORK"
  PLUGIN="$WORK/variants/linux-amd64-dev/osm_plugin.so"
fi

if [[ -z "$FFI" && -z "${SMOKE_NO_FFI:-}" ]]; then
  echo ">> building Rust osm core (cargo release)..."
  cargo build -p logos-osm --release >/dev/null
  FFI="$PWD/target/release/liblogos_osm.so"
fi

echo ">> plugin: $PLUGIN"
test -f "$PLUGIN" || { echo "plugin missing"; exit 1; }
if [[ -z "$FFI" ]]; then
  echo ">> ffi:    (none — SMOKE_NO_FFI: asserting fail-closed health)"
else
  echo ">> ffi:    $FFI"
  test -f "$FFI" || { echo "ffi lib missing"; exit 1; }
fi

python3 - "$PLUGIN" "$FFI" <<'PY'
import ctypes, os, sys

plugin, ffi = sys.argv[1], sys.argv[2]
if ffi:
    os.environ["LOGOS_OSM_FFI_PATH"] = ffi

h = ctypes.CDLL(plugin, mode=ctypes.RTLD_LOCAL)
h.logos_module_get_protocol_version.restype = ctypes.c_char_p
h.logos_module_get_methods.restype = ctypes.c_void_p
h.logos_module_dispatch.restype = ctypes.c_void_p
h.logos_module_dispatch.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
h.logos_module_string_free.argtypes = [ctypes.c_void_p]

def owned(p):
    if not p:
        return None
    s = ctypes.cast(p, ctypes.c_char_p).value.decode()
    h.logos_module_string_free(p)
    return s

proto = h.logos_module_get_protocol_version().decode()
methods = owned(h.logos_module_get_methods())
print("protocol_version:", proto)
print("methods:", methods)

# Dispatch results are JSON values; a std::string return arrives
# double-encoded (a JSON string wrapping the module's JSON), so unwrap once.
import json
def unwrap(r):
    try:
        inner = json.loads(r)
        return inner if isinstance(inner, str) else r
    except ValueError:
        return r

assert proto == "0.5.0", f"protocol version {proto!r}"
assert "osmVersionJson" in methods and "invokeOpJson" in methods, "method surface"
hl = unwrap(owned(h.logos_module_dispatch(b"health", b"[]")))

if not ffi:
    # No Rust core on PATH: dispatch must still work and FAIL CLOSED.
    assert '"ok":false' in hl and "not loaded" in hl, \
        f"expected fail-closed health without the Rust core, got: {hl}"
    print("\nSMOKE OK (no-FFI): module loads, ABI negotiates, health fails closed.")
    sys.exit(0)

v = unwrap(owned(h.logos_module_dispatch(b"osmVersionJson", b"[]")))
assert '"ok":true' in v, f"osmVersionJson not ok: {v}"
assert '"ok":true' in hl, f"health not ok: {hl}"
st = unwrap(owned(h.logos_module_dispatch(b"invokeOpJson", b'["status", "{}"]')))
# status before open is an EXPECTED structured error from the Rust core — proves
# the C++ -> Rust FFI bridge is live, not a stub.
assert '"osm not open' in st, f"expected 'osm not open' from live core, got: {st}"
# NOTE: the outer array's second element must be a JSON *string* containing
# the args object — so the inner quotes are escaped in the wire bytes.
# (Python consumes one level of \", hence \\" here.)
rg = unwrap(owned(h.logos_module_dispatch(
    b"invokeOpJson", b'["regions", "{\\"parent\\": \\"us\\"}"]')))
assert '"count":8' in rg, f"expected 8 US subregions from live core, got: {rg}"
print("\nSMOKE OK: module loads, ABI negotiates, Rust core dispatches.")
PY
