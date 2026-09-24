#include "osm_ffi_client.h"

#include <cstdlib>

// Platform-agnostic dynamic loading. POSIX (Linux/macOS) uses dlfcn.h
// (dlopen/dlsym/dlclose); Windows uses the Win32 loader
// (LoadLibraryA/GetProcAddress/FreeLibrary). The OSM core is a cdylib
// (liblogos_osm.so on Linux, .dylib on macOS, logos_osm.dll on Windows),
// so the same FFI surface is reached through whichever loader the host
// provides. The function-pointer typedefs in the header are identical on
// both — a C ABI is a C ABI — so only the open/resolve/close calls differ.
#ifdef _WIN32
#  include <windows.h>
#else
#  include <dlfcn.h>
#endif

namespace {
// The default library name the loader searches when LOGOS_OSM_FFI_PATH is
// unset. The extension is platform-specific; the env override (an absolute
// path) is the reliable cross-platform path.
#ifdef _WIN32
const char* kLib    = "logos_osm.dll";
#else
const char* kLib    = "liblogos_osm.so";
#endif
const char* kEnvKey = "LOGOS_OSM_FFI_PATH";

// Minimal JSON error string (quotes escaped) — mirrors the Rust core's
// {"ok":false,"error":...} contract on the C side of a load failure.
std::string errJson(const std::string& message)
{
    std::string escaped;
    escaped.reserve(message.size());
    for (char c : message) {
        if (c == '"') {
            escaped += '\'';
        } else {
            escaped += c;
        }
    }
    return "{\"ok\":false,\"error\":\"" + escaped + "\"}";
}
} // namespace

bool OsmFfiClient::load()
{
    if (m_loaded) {
        return true;
    }

    const char* envPath = std::getenv(kEnvKey);
    const std::string libPath = (envPath && *envPath) ? envPath : kLib;

#ifdef _WIN32
    m_lib = reinterpret_cast<void*>(::LoadLibraryA(libPath.c_str()));
    if (!m_lib) {
        m_lastErr = "cannot load " + libPath + ": LoadLibraryA failed (err "
                    + std::to_string(::GetLastError()) + ")";
        return false;
    }
    m_version = reinterpret_cast<NoArgFn>(::GetProcAddress(
        reinterpret_cast<HMODULE>(m_lib), "logos_osm_version"));
    m_free    = reinterpret_cast<FreeFn>(::GetProcAddress(
        reinterpret_cast<HMODULE>(m_lib), "logos_osm_free_string"));
    m_invoke  = reinterpret_cast<InvokeFn>(::GetProcAddress(
        reinterpret_cast<HMODULE>(m_lib), "logos_osm_invoke"));
#else
    m_lib = ::dlopen(libPath.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!m_lib) {
        const char* dlErr = ::dlerror();
        m_lastErr = "cannot load " + libPath + ": " + (dlErr ? dlErr : "unknown dlopen error");
        return false;
    }

    m_version = reinterpret_cast<NoArgFn>(::dlsym(m_lib, "logos_osm_version"));
    m_free    = reinterpret_cast<FreeFn>(::dlsym(m_lib, "logos_osm_free_string"));
    m_invoke  = reinterpret_cast<InvokeFn>(::dlsym(m_lib, "logos_osm_invoke"));
#endif

    if (!m_version || !m_free || !m_invoke) {
        m_lastErr = "missing symbols in " + libPath;
#ifdef _WIN32
        ::FreeLibrary(reinterpret_cast<HMODULE>(m_lib));
#else
        ::dlclose(m_lib);
#endif
        m_lib = nullptr;
        return false;
    }

    m_loaded = true;
    return true;
}

std::string OsmFfiClient::version()
{
    if (!load()) {
        return errJson(m_lastErr);
    }
    char* result = m_version();
    if (!result) {
        return errJson("version returned null");
    }
    std::string text(result);
    m_free(result);
    return text;
}

std::string OsmFfiClient::invoke(const std::string& name, const std::string& argsJson)
{
    if (!load()) {
        return errJson(m_lastErr);
    }
    char* result = m_invoke(name.c_str(), argsJson.c_str());
    if (!result) {
        return errJson("invoke returned null");
    }
    std::string text(result);
    m_free(result);
    return text;
}

OsmFfiClient::~OsmFfiClient()
{
    if (m_lib) {
#ifdef _WIN32
        ::FreeLibrary(reinterpret_cast<HMODULE>(m_lib));
#else
        ::dlclose(m_lib);
#endif
    }
}
