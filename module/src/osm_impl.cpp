#include "osm_impl.h"

#include <string>

#include "osm_ffi_client.h"

std::string OsmImpl::health()
{
    const std::string version = m_ffi.version();
    if (version.find("\"ok\":true") != std::string::npos) {
        return "{\"ok\":true}";
    }
    return "{\"ok\":false,\"error\":\"osm core not loaded\"}";
}

std::string OsmImpl::osmVersionJson()
{
    return m_ffi.version();
}

std::string OsmImpl::invokeOpJson(const std::string& name, const std::string& argsJson)
{
    return m_ffi.invoke(name, argsJson);
}
