#pragma once

#include <string>
#include "logos_module_context.h"
#include "osm_ffi_client.h"

/**
 * @brief The OpenStreetMap distribution core module.
 *
 * Universal authoring model: this class IS the module's API. Its public
 * methods are callable by other modules and from the CLI (`logoscore -c`),
 * and the Qt plugin glue (the *Plugin/*Interface classes, Q_PLUGIN_METADATA,
 * initLogos wiring) is generated from this header by logos-module-builder
 * (interface: "universal").
 *
 * All methods are JSON-out — callers parse the returned std::string as a
 * JSON object containing at least {"ok": true|false}. The OSM lifecycle
 * logic lives in the Rust core (liblogos_osm.so), which this module loads
 * via the C ABI (osm_ffi_client).
 *
 * Deriving LogosModuleContext gives modules() / typed events / onContextReady
 * (unused here — the module is a thin FFI bridge). Module code is Qt-free.
 */
class OsmImpl : public LogosModuleContext
{
public:
    /// Liveness: reports whether the Rust OSM core is loadable (JSON).
    std::string health();

    /// Version of the Rust OSM core (JSON).
    std::string osmVersionJson();

    /// Invoke an OSM operation by name with JSON arguments; returns the JSON
    /// result. Operations: open, status, regions, discover, host, host_bulk,
    /// register, register_bulk, init, fetch, import, update, catalog (the
    /// full FFI dispatch surface — see osm-sdk/src/ffi.rs).
    std::string invokeOpJson(const std::string& name, const std::string& argsJson);

private:
    OsmFfiClient m_ffi;
};
