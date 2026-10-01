/* osm_ffi_smoke.c — prove the OSM core cdylib loads and dispatches on this
 * platform.
 *
 * Compiled standalone (cc/osm_ffi_smoke.c) and pointed at whatever the cargo
 * cdylib produced on the host (liblogos_osm.so on Linux, liblogos_osm.dylib
 * on macOS, logos_osm.dll on Windows — the last via the C++ client in
 * module/src instead, which already has the Win32 loader path). It resolves
 * the C ABI, calls `version` and `regions`, and exits nonzero unless the
 * answer comes back ok with the full closed region set. That is the
 * "runs on this platform" evidence for the platform matrix: a real op
 * round-trips dlopen -> Rust core -> JSON out, with no stubs involved.
 *
 * Usage: osm_ffi_smoke <path-to-cdylib>   (or set LOGOS_OSM_FFI_PATH)
 */

#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef char *(*invoke_fn)(const char *, const char *);
typedef void (*free_fn)(char *);
typedef char *(*version_fn)(void);

static int contains(const char *hay, const char *needle) {
    return strstr(hay, needle) != NULL;
}

int main(int argc, char **argv) {
    const char *path = argc > 1 ? argv[1] : getenv("LOGOS_OSM_FFI_PATH");
    if (!path || !*path) {
        fprintf(stderr, "usage: %s <path-to-cdylib> (or set LOGOS_OSM_FFI_PATH)\n", argv[0]);
        return 2;
    }

    void *lib = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (!lib) {
        const char *e = dlerror();
        fprintf(stderr, "smoke: cannot load %s: %s\n", path, e ? e : "unknown dlopen error");
        return 1;
    }
    version_fn version = (version_fn)dlsym(lib, "logos_osm_version");
    invoke_fn invoke = (invoke_fn)dlsym(lib, "logos_osm_invoke");
    free_fn free_str = (free_fn)dlsym(lib, "logos_osm_free_string");
    if (!version || !invoke || !free_str) {
        const char *e = dlerror();
        fprintf(stderr, "smoke: missing symbols in %s: %s\n", path, e ? e : "unknown");
        return 1;
    }

    char *v = version();
    printf("version: %s\n", v);
    int v_ok = contains(v, "\"ok\":true");
    free_str(v);

    char *out = invoke("regions", "{}");
    if (!out) {
        fprintf(stderr, "smoke: regions returned NULL\n");
        return 1;
    }
    printf("regions: %zu bytes; head: %.100s\n", strlen(out), out);
    int ok = v_ok && contains(out, "\"ok\":true") && contains(out, "\"count\":72");
    free_str(out);
    dlclose(lib);

    if (!ok) {
        fprintf(stderr, "smoke: expected ok:true with count:72\n");
        return 1;
    }
    printf("smoke: ok — cdylib loads and dispatches on this platform\n");
    return 0;
}
