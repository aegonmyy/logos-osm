#include "osm_ffi_client.h"

#include <dlfcn.h>

#include <cstdlib>

namespace {
const char* kLib    = "liblogos_osm.so";
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

    m_lib = ::dlopen(libPath.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!m_lib) {
        const char* dlErr = ::dlerror();
        m_lastErr = "cannot load " + libPath + ": " + (dlErr ? dlErr : "unknown dlopen error");
        return false;
    }

    m_version = reinterpret_cast<NoArgFn>(::dlsym(m_lib, "logos_osm_version"));
    m_free    = reinterpret_cast<FreeFn>(::dlsym(m_lib, "logos_osm_free_string"));
    m_invoke  = reinterpret_cast<InvokeFn>(::dlsym(m_lib, "logos_osm_invoke"));

    if (!m_version || !m_free || !m_invoke) {
        m_lastErr = "missing symbols in " + libPath;
        ::dlclose(m_lib);
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
        ::dlclose(m_lib);
    }
}
