// The C ABI is the one place this crate touches raw pointers (the Qt plugin
// passes us `*const c_char` / frees our `*mut c_char` results); the pointers'
// validity is the caller's side of that contract.
#![allow(unsafe_code)]

//! C ABI exposed to the Logos Core Qt plugin (`module/`).
//!
//! The plugin dlopens `liblogos_osm.so` and resolves:
//!   - `logos_osm_version`      -> JSON `{"ok":true,"version":"..."}`
//!   - `logos_osm_invoke`       -> dispatch an OSM op by name + JSON args
//!   - `logos_osm_free_string`  -> free a string returned by the above
//!
//! `invoke` routes to the Rust SDK ([`crate::OsmClient`]). The async SDK ops
//! (Geofabrik download, Logos Storage put/get) run on a private tokio runtime
//! so the C ABI stays synchronous — the Qt plugin calls from a worker thread.
//! Local state is just configuration (endpoints + cache dir + the program
//! binding); the import catalog itself lives in the cache dir, so the app
//! spans multiple invocations exactly like the CLI.
//!
//! Ops (see `dispatch`): `open`, `status`, `regions`, `discover`, `host`,
//! `host_bulk`, `register`, `register_bulk`, `init`, `fetch`, `import`,
//! `update`, `catalog`. Transactions come back as JSON for the wallet to
//! submit — the FFI never signs or submits (same split as the CLI).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::regions::{Level, Region, REGIONS};
use crate::registry::{
    build_init, build_register_region, build_register_regions_batch, to_identities, OsmTxBuilt,
};
use crate::storage::{CodexStorage, MemoryStorage, Storage};
use crate::OsmClient;

/// The pinned program id of the deployed `osm-registry` guest, hex
/// (big-endian bytes of the `[u32; 8]` id). Kept in sync with
/// `methods/osm-host`'s `DOCUMENTED_ID_HEX`; the integration-test crate has a
/// consistency test asserting the embedded artifact still hashes to this.
pub const DOCUMENTED_PROGRAM_ID_HEX: &str =
    "77ecdf2f92edfb9eb54c9ae3f5beca1f46b6f9a5d109667b462fd96c7d1c43f0";

/// Return a heap JSON string the caller must free with `logos_osm_free_string`.
fn to_json(v: Value) -> *mut c_char {
    let s = CString::new(v.to_string()).unwrap_or_else(|_| CString::new("null").unwrap());
    s.into_raw()
}

fn ok(obj: Value) -> *mut c_char {
    to_json(json!({ "ok": true, "result": obj }))
}
fn err(msg: impl Into<String>) -> *mut c_char {
    to_json(json!({ "ok": false, "error": msg.into() }))
}

/// Free a string previously returned by this FFI.
///
/// # Safety
/// `ptr` must be a non-null pointer previously returned by this FFI (or
/// null), and must not have been freed already.
#[no_mangle]
pub unsafe extern "C" fn logos_osm_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(CString::from_raw(ptr));
}

#[no_mangle]
pub extern "C" fn logos_osm_version() -> *mut c_char {
    to_json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "name": env!("CARGO_PKG_NAME"),
    }))
}

// ---------------------------------------------------------------------------
// Config state
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct AppState {
    storage_url: String,
    geofabrik_url: String,
    cache_dir: String,
    /// In-memory storage (offline demo): nothing persists.
    memory: bool,
    /// Program binding for built transactions (hex, overridable).
    program_id_hex: String,
    state_path: String,
}

impl AppState {
    fn save(&self) -> anyhow::Result<()> {
        let path = PathBuf::from(&self.state_path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| anyhow::anyhow!("encoding osm state: {e}"))?;
        std::fs::write(&path, bytes).map_err(|e| anyhow::anyhow!("writing osm state: {e}"))?;
        Ok(())
    }
}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Run an async op on the private runtime, blocking the caller.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    rt().block_on(f)
}

/// The storage backend the app selected at `open` (real node or in-memory).
enum AnyStorage {
    Memory(MemoryStorage),
    Codex(CodexStorage),
}

impl Storage for AnyStorage {
    async fn put_bytes(&self, data: bytes::Bytes) -> anyhow::Result<crate::storage::StoredObject> {
        match self {
            AnyStorage::Memory(s) => s.put_bytes(data).await,
            AnyStorage::Codex(s) => s.put_bytes(data).await,
        }
    }
    async fn put_file(
        &self,
        path: &std::path::Path,
    ) -> anyhow::Result<crate::storage::StoredObject> {
        match self {
            AnyStorage::Memory(s) => s.put_file(path).await,
            AnyStorage::Codex(s) => s.put_file(path).await,
        }
    }
    async fn get_bytes(&self, cid: &str) -> anyhow::Result<bytes::Bytes> {
        match self {
            AnyStorage::Memory(s) => s.get_bytes(cid).await,
            AnyStorage::Codex(s) => s.get_bytes(cid).await,
        }
    }
    async fn get_to_file(
        &self,
        cid: &str,
        dest: &std::path::Path,
    ) -> anyhow::Result<crate::geofabrik::DownloadOutcome> {
        match self {
            AnyStorage::Memory(s) => s.get_to_file(cid, dest).await,
            AnyStorage::Codex(s) => s.get_to_file(cid, dest).await,
        }
    }
}

/// Build a lifecycle client from the current state (cheap: config only).
fn client(st: &AppState) -> OsmClient<AnyStorage> {
    let storage = if st.memory {
        AnyStorage::Memory(MemoryStorage::new())
    } else {
        AnyStorage::Codex(CodexStorage::new(&st.storage_url))
    };
    OsmClient::new(storage, &st.cache_dir).with_geofabrik_base(&st.geofabrik_url)
}

/// Hold the state lock across a sync read/mutation, returning JSON.
fn with_state<R>(f: impl FnOnce(&mut AppState) -> anyhow::Result<R>) -> *mut c_char
where
    R: serde::Serialize,
{
    let mut guard = STATE.lock().unwrap();
    let Some(st) = guard.as_mut() else {
        return err("osm not open: call open first");
    };
    match f(st) {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => err(format!("{e:#}")),
    }
}

// ---------------------------------------------------------------------------
// Transaction JSON (for the wallet to submit — mirrors the CLI's TxCmd)
// ---------------------------------------------------------------------------

fn program_id_from_hex(hex_str: &str) -> anyhow::Result<lee_core::program::ProgramId> {
    let bytes = hex::decode(hex_str.trim()).map_err(|e| anyhow::anyhow!("program_id hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("program_id must be 32 bytes, got {}", bytes.len()))?;
    let mut words = [0u32; 8];
    for (i, w) in words.iter_mut().enumerate() {
        *w = u32::from_be_bytes([arr[i * 4], arr[i * 4 + 1], arr[i * 4 + 2], arr[i * 4 + 3]]);
    }
    Ok(words)
}

fn account_from_hex(hex_str: &str) -> anyhow::Result<lee_core::account::AccountId> {
    let bytes = hex::decode(hex_str.trim()).map_err(|e| anyhow::anyhow!("account hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("account id must be 32 bytes, got {}", bytes.len()))?;
    Ok(lee_core::account::AccountId::new(arr))
}

fn hex_words(words: &[u32]) -> String {
    let mut out = String::with_capacity(words.len() * 8);
    for w in words {
        out.push_str(&format!("{w:08x}"));
    }
    out
}

/// The wallet-submittable form of a built tx: account ids in guest order,
/// which of them sign, and the risc0-serde instruction words.
fn tx_json(st: &AppState, built: &OsmTxBuilt, signer: &lee_core::account::AccountId) -> Value {
    let ids = to_identities(built, signer);
    let signing: Vec<&str> = ids
        .iter()
        .map(|id| match id {
            wallet::AccountIdentity::Public(_) => "sign",
            _ => "read",
        })
        .collect();
    json!({
        "program_id_hex": st.program_id_hex,
        "accounts_hex": built
            .accounts
            .iter()
            .map(|a| hex::encode(a.value()))
            .collect::<Vec<_>>(),
        "signing": signing,
        "instruction_hex": hex_words(&built.instruction),
    })
}

fn region_json(r: &Region) -> Value {
    json!({
        "path": r.path,
        "continent": r.continent,
        "parent": r.parent,
        "level": match r.level { Level::Country => "country", Level::Subregion => "subregion" },
        "pbf_url": r.source_url(),
    })
}

// ---------------------------------------------------------------------------
// Op dispatch
// ---------------------------------------------------------------------------

/// Invoke an OSM operation. `name` and `args_json` are null-terminated UTF-8.
/// Returns a heap JSON string the caller frees with `logos_osm_free_string`.
///
/// # Safety
/// `name` must be a valid null-terminated UTF-8 C string. `args_json`, if
/// non-null, must be a valid null-terminated UTF-8 C string whose contents
/// parse as a JSON object.
#[no_mangle]
pub unsafe extern "C" fn logos_osm_invoke(
    name: *const c_char,
    args_json: *const c_char,
) -> *mut c_char {
    if name.is_null() {
        return err("null op name");
    }
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return err("non-utf8 op name"),
    };
    let args: Value = if args_json.is_null() {
        Value::Object(Default::default())
    } else {
        let raw = CStr::from_ptr(args_json);
        match raw.to_str() {
            Err(_) => return err("non-utf8 args"),
            Ok(s) => serde_json::from_str(s).unwrap_or_else(|_| Value::Object(Default::default())),
        }
    };
    dispatch(&name, &args)
}

fn dispatch(name: &str, args: &Value) -> *mut c_char {
    match name {
        "version" => to_json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")})),
        "open" => op_open(args),
        "status" => with_state(|st| {
            Ok(json!({
                "storage_url": st.storage_url,
                "geofabrik_url": st.geofabrik_url,
                "cache_dir": st.cache_dir,
                "memory_storage": st.memory,
                "program_id_hex": st.program_id_hex,
                "catalog": client(st).catalog().len(),
            }))
        }),
        "regions" => {
            let parent = str_arg(args, "parent");
            let out: Vec<Value> = REGIONS
                .iter()
                .filter(|r| parent.as_deref().is_none_or(|p| r.parent == Some(p)))
                .map(region_json)
                .collect();
            ok(json!({ "regions": out, "count": out.len() }))
        }
        "discover" => op_discover(),
        "host" => op_host(args),
        "host_bulk" => op_host_bulk(args),
        "register" => op_register(args),
        "register_bulk" => op_register_bulk(args),
        "init" => op_init(args),
        "fetch" => op_fetch(args),
        "import" => op_import(args),
        "update" => op_update(args),
        "catalog" => with_state(|st| {
            Ok(client(st)
                .catalog()
                .into_iter()
                .map(|c| {
                    json!({
                        "region": c.region,
                        "version": c.version,
                        "cid": c.cid,
                        "md5": hex::encode(c.md5),
                        "path": c.path.display().to_string(),
                        "imported_at": c.imported_at,
                    })
                })
                .collect::<Vec<_>>())
        }),
        other => err(format!("unknown op: {other}")),
    }
}

fn op_open(args: &Value) -> *mut c_char {
    let state_path = str_arg(args, "state_path").unwrap_or_else(|| "osm-state.json".to_string());
    let loaded = std::fs::read(&state_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<AppState>(&b).ok());
    let mut st = loaded.unwrap_or_else(|| AppState {
        storage_url: str_arg(args, "storage_url")
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
        geofabrik_url: str_arg(args, "geofabrik_url")
            .unwrap_or_else(|| crate::geofabrik::DEFAULT_BASE.to_string()),
        cache_dir: str_arg(args, "cache_dir").unwrap_or_else(|| "osm-cache".to_string()),
        memory: args
            .get("memory")
            .and_then(|m| m.as_bool())
            .unwrap_or(false),
        program_id_hex: DOCUMENTED_PROGRAM_ID_HEX.to_string(),
        state_path: state_path.clone(),
    });
    // Refresh any provided config (lets the app re-point at live endpoints).
    if let Some(v) = str_arg(args, "storage_url") {
        st.storage_url = v;
    }
    if let Some(v) = str_arg(args, "geofabrik_url") {
        st.geofabrik_url = v;
    }
    if let Some(v) = str_arg(args, "cache_dir") {
        st.cache_dir = v;
    }
    if let Some(v) = args.get("memory").and_then(|m| m.as_bool()) {
        st.memory = v;
    }
    if let Some(v) = str_arg(args, "program_id_hex") {
        // Validate early so a typo can't poison every later tx build.
        if let Err(e) = program_id_from_hex(&v) {
            return err(format!("program_id_hex: {e}"));
        }
        st.program_id_hex = v;
    }
    st.state_path = state_path;
    let _ = std::fs::create_dir_all(&st.cache_dir);
    let summary = json!({
        "storage_url": st.storage_url,
        "geofabrik_url": st.geofabrik_url,
        "cache_dir": st.cache_dir,
        "memory_storage": st.memory,
        "program_id_hex": st.program_id_hex,
        "catalog": client(&st).catalog().len(),
    });
    if let Err(e) = st.save() {
        return err(format!("{e:#}"));
    }
    *STATE.lock().unwrap() = Some(st);
    ok(summary)
}

/// Live upstream cross-check of the closed region set (needs network).
fn op_discover() -> *mut c_char {
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    let available = match block_on(c.discover()) {
        Ok(a) => a,
        Err(e) => return err(format!("discover: {e:#}")),
    };
    ok(json!({
        "available": available.len(),
        "total_set": REGIONS.len(),
        "regions": available
            .iter()
            .map(|i| json!({ "path": i.region.path, "pbf_url": i.pbf_url }))
            .collect::<Vec<_>>(),
    }))
}

/// Host one region: download from Geofabrik, verify the published MD5, store
/// in Logos Storage; optionally build the registration tx.
fn op_host(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("host requires region");
    };
    let registrar = match str_arg(args, "registrar_hex") {
        Some(h) => match account_from_hex(&h) {
            Ok(a) => Some(a),
            Err(e) => return err(format!("registrar_hex: {e}")),
        },
        None => None,
    };
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    let snap = match block_on(c.host_region(&region)) {
        Ok(s) => s,
        Err(e) => return err(format!("host {region}: {e:#}")),
    };
    if let Err(e) = c.catalog_record_hosted(&snap) {
        return err(format!("catalog: {e:#}"));
    }
    let program_id = match program_id_from_hex(&st.program_id_hex) {
        Ok(p) => p,
        Err(e) => return err(format!("{e}")),
    };
    let tx = registrar.as_ref().map(|reg| {
        let built = build_register_region(&program_id, reg, &snap.registration(None));
        tx_json(st, &built, reg)
    });
    ok(json!({
        "region": snap.region,
        "cid": snap.cid,
        "version": snap.version,
        "bytes": snap.bytes,
        "md5": hex::encode(snap.checksum),
        "path": snap.path.display().to_string(),
        "tx": tx,
    }))
}

/// Bulk host with per-region opt-out (a skipped region is reported, not an
/// error; per-region failures don't abort the run).
fn op_host_bulk(args: &Value) -> *mut c_char {
    let Some(spec) = str_arg(args, "regions") else {
        return err("host_bulk requires regions (comma-separated or all)");
    };
    let wanted: Vec<String> = if spec == "all" {
        REGIONS.iter().map(|r| r.path.to_string()).collect()
    } else {
        spec.split(',').map(str::trim).map(String::from).collect()
    };
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let opt_out: Vec<String> = str_arg(args, "opt_out")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let opt_out: Vec<&str> = opt_out.iter().map(String::as_str).collect();
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    let report = match block_on(c.host_regions_bulk(&wanted, &opt_out)) {
        Ok(r) => r,
        Err(e) => return err(format!("host_bulk: {e:#}")),
    };
    for s in &report.hosted {
        let _ = c.catalog_record_hosted(s);
    }
    ok(json!({
        "hosted": report
            .hosted
            .iter()
            .map(|s| json!({
                "region": s.region, "cid": s.cid,
                "version": s.version, "bytes": s.bytes,
                "md5": hex::encode(s.checksum),
            }))
            .collect::<Vec<_>>(),
        "skipped": report.skipped,
    }))
}

/// Registration tx for a region already in the local catalog.
fn op_register(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("register requires region");
    };
    let Some(reg_hex) = str_arg(args, "registrar_hex") else {
        return err("register requires registrar_hex (32 bytes hex)");
    };
    let registrar = match account_from_hex(&reg_hex) {
        Ok(a) => a,
        Err(e) => return err(format!("registrar_hex: {e}")),
    };
    with_state(|st| {
        let rec = client(st)
            .catalog()
            .into_iter()
            .find(|c| c.region == region)
            .ok_or_else(|| {
                anyhow::anyhow!("{region} is not in the local catalog (host it first)")
            })?;
        let snap = crate::HostedSnapshot {
            region: rec.region,
            cid: rec
                .cid
                .ok_or_else(|| anyhow::anyhow!("catalog record has no cid"))?,
            checksum: rec.md5,
            version: rec.version,
            bytes: 0,
            path: rec.path,
        };
        let program_id = program_id_from_hex(&st.program_id_hex)?;
        let built = build_register_region(&program_id, &registrar, &snap.registration(None));
        Ok(tx_json(st, &built, &registrar))
    })
}

/// Batch registration tx (bulk hosting on-chain).
fn op_register_bulk(args: &Value) -> *mut c_char {
    let Some(spec) = str_arg(args, "regions") else {
        return err("register_bulk requires regions (comma-separated)");
    };
    let Some(reg_hex) = str_arg(args, "registrar_hex") else {
        return err("register_bulk requires registrar_hex (32 bytes hex)");
    };
    let registrar = match account_from_hex(&reg_hex) {
        Ok(a) => a,
        Err(e) => return err(format!("registrar_hex: {e}")),
    };
    let wanted: Vec<String> = spec.split(',').map(str::trim).map(String::from).collect();
    with_state(|st| {
        let catalog = client(st).catalog();
        let mut regs = Vec::with_capacity(wanted.len());
        for region in &wanted {
            let rec = catalog
                .iter()
                .find(|c| &c.region == region)
                .ok_or_else(|| anyhow::anyhow!("{region} is not in the local catalog"))?;
            let snap = crate::HostedSnapshot {
                region: rec.region.clone(),
                cid: rec
                    .cid
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("catalog record has no cid"))?,
                checksum: rec.md5,
                version: rec.version,
                bytes: 0,
                path: rec.path.clone(),
            };
            regs.push(snap.registration(None));
        }
        let program_id = program_id_from_hex(&st.program_id_hex)?;
        let built = build_register_regions_batch(&program_id, &registrar, &regs);
        Ok(tx_json(st, &built, &registrar))
    })
}

/// The registry `Init` tx (deploy-time, once).
fn op_init(args: &Value) -> *mut c_char {
    let Some(owner_hex) = str_arg(args, "owner_hex") else {
        return err("init requires owner_hex (32 bytes hex)");
    };
    let owner = match account_from_hex(&owner_hex) {
        Ok(a) => a,
        Err(e) => return err(format!("owner_hex: {e}")),
    };
    with_state(|st| {
        let program_id = program_id_from_hex(&st.program_id_hex)?;
        let built = build_init(&program_id, &owner);
        Ok(tx_json(st, &built, &owner))
    })
}

/// Fetch a registered snapshot from Logos Storage (Geofabrik fallback),
/// verify the checksum, import locally.
fn op_fetch(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("fetch requires region");
    };
    let Some(cid) = str_arg(args, "cid") else {
        return err("fetch requires cid (from the registry)");
    };
    let Some(md5_hex) = str_arg(args, "md5_hex") else {
        return err("fetch requires md5_hex (published checksum, hex)");
    };
    let checksum: [u8; 16] = match hex::decode(md5_hex.trim()) {
        Ok(b) => match b.as_slice().try_into() {
            Ok(arr) => arr,
            Err(_) => return err(format!("md5 must be 16 bytes hex, got {} bytes", b.len())),
        },
        Err(e) => return err(format!("md5_hex: {e}")),
    };
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    let outcome = match block_on(c.fetch_snapshot(&region, &cid, &checksum)) {
        Ok(o) => o,
        Err(e) => return err(format!("fetch {region}: {e:#}")),
    };
    let summary = match block_on(c.import_local(&region, &outcome.path)) {
        Ok(s) => s,
        Err(e) => return err(format!("import {region}: {e:#}")),
    };
    ok(json!({
        "region": summary.region,
        "bytes": summary.bytes,
        "blobs": summary.blobs,
        "md5_verified": hex::encode(summary.md5),
        "path": outcome.path.display().to_string(),
    }))
}

/// Import a local PBF into the catalog (structure-validated).
fn op_import(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("import requires region");
    };
    let Some(pbf) = str_arg(args, "pbf") else {
        return err("import requires pbf (local file path)");
    };
    // `store: true` runs the FULL local-import workflow (like `host`, but
    // with user-provided bytes): verify the file against Geofabrik's
    // currently-published MD5, store it in Logos Storage, and — with
    // `registrar_hex` — return the registration tx.
    let store = args.get("store").and_then(Value::as_bool).unwrap_or(false);
    let registrar = match str_arg(args, "registrar_hex") {
        Some(h) => match account_from_hex(&h) {
            Ok(a) => Some(a),
            Err(e) => return err(format!("registrar_hex: {e}")),
        },
        None => None,
    };
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    let path = std::path::Path::new(&pbf);
    if store {
        let snap = match block_on(c.host_local(&region, path)) {
            Ok(s) => s,
            Err(e) => return err(format!("import {region}: {e:#}")),
        };
        if let Err(e) = c.catalog_record_hosted(&snap) {
            return err(format!("catalog: {e:#}"));
        }
        let program_id = match program_id_from_hex(&st.program_id_hex) {
            Ok(p) => p,
            Err(e) => return err(format!("{e}")),
        };
        let tx = registrar.as_ref().map(|reg| {
            let built = build_register_region(&program_id, reg, &snap.registration(None));
            tx_json(st, &built, reg)
        });
        ok(json!({
            "region": snap.region,
            "cid": snap.cid,
            "version": snap.version,
            "bytes": snap.bytes,
            "md5": hex::encode(snap.checksum),
            "workflow": "verified+stored",
            "tx": tx,
        }))
    } else {
        match block_on(c.import_local(&region, path)) {
            Ok(s) => ok(json!({
                "region": s.region, "bytes": s.bytes, "blobs": s.blobs,
                "md5": hex::encode(s.md5),
            })),
            Err(e) => err(format!("import: {e:#}")),
        }
    }
}

/// Check whether a newer snapshot is published upstream.
fn op_update(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("update requires region");
    };
    let guard = STATE.lock().unwrap();
    let Some(st) = guard.as_ref() else {
        return err("osm not open");
    };
    let c = client(st);
    match block_on(c.update_check(&region)) {
        Ok(crate::UpdateStatus::UpToDate { local, published }) => {
            ok(json!({ "status": "up_to_date", "local": local, "published": published }))
        }
        Ok(crate::UpdateStatus::UpdateAvailable { local, published }) => {
            ok(json!({ "status": "update_available", "local": local, "published": published }))
        }
        Ok(crate::UpdateStatus::NotImported { published }) => {
            ok(json!({ "status": "not_imported", "local": null, "published": published }))
        }
        Err(e) => err(format!("update: {e:#}")),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

#[cfg(test)]
mod tests {
    //! Exercises the C ABI directly. Network ops (host/fetch/discover/update)
    //! need live Geofabrik/Storage and are covered by the CLI + live tests;
    //! here we verify dispatch, state persistence, tx JSON, and error paths.
    use super::*;
    use std::ffi::CString;

    fn invoke(name: &str, args: &serde_json::Value) -> Value {
        let n = CString::new(name).unwrap();
        let a = CString::new(args.to_string()).unwrap();
        // SAFETY: both pointers are valid null-terminated UTF-8 CStrings.
        let ptr = unsafe { logos_osm_invoke(n.as_ptr(), a.as_ptr()) };
        let s = unsafe { CString::from_raw(ptr) }
            .to_string_lossy()
            .to_string();
        serde_json::from_str(&s).unwrap_or(Value::Null)
    }

    fn invoke_no_args(name: &str) -> Value {
        let n = CString::new(name).unwrap();
        // SAFETY: `n` is a valid CString; args is null (empty object).
        let ptr = unsafe { logos_osm_invoke(n.as_ptr(), std::ptr::null()) };
        let s = unsafe { CString::from_raw(ptr) }
            .to_string_lossy()
            .to_string();
        serde_json::from_str(&s).unwrap_or(Value::Null)
    }

    fn fresh_state() -> (String, String) {
        let dir = tempfile::tempdir().unwrap();
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        let base = dir.keep();
        (
            format!("{}/osm-ffi-{nano}.json", base.display()),
            format!("{}/cache-{nano}", base.display()),
        )
    }

    #[test]
    fn ffi_op_dispatch_and_persistence() {
        let (state, cache) = fresh_state();
        // Before open: status reports not open.
        let r = invoke_no_args("status");
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        // Open (in-memory storage; offline).
        let r = invoke(
            "open",
            &json!({ "state_path": state, "cache_dir": cache, "memory": true }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        // status reflects the config.
        let r = invoke_no_args("status");
        assert_eq!(r["result"]["memory_storage"].as_bool(), Some(true));
        assert_eq!(r["result"]["catalog"].as_u64(), Some(0));
        // regions: full set + parent filter.
        let r = invoke_no_args("regions");
        assert_eq!(r["result"]["count"].as_u64(), Some(REGIONS.len() as u64));
        let r = invoke("regions", &json!({ "parent": "us" }));
        assert_eq!(r["result"]["count"].as_u64(), Some(8));
        // catalog empty.
        let r = invoke_no_args("catalog");
        assert!(r["result"].as_array().unwrap().is_empty());
        // register without catalog -> error naming the region.
        let r = invoke(
            "register",
            &json!({ "region": "germany", "registrar_hex": "09".repeat(32) }),
        );
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        assert!(r["error"].as_str().unwrap().contains("germany"));
        // init builds a tx against the pinned program id.
        let r = invoke("init", &json!({ "owner_hex": "07".repeat(32) }));
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        assert_eq!(
            r["result"]["program_id_hex"].as_str().unwrap(),
            DOCUMENTED_PROGRAM_ID_HEX
        );
        assert_eq!(
            r["result"]["instruction_hex"].as_str().unwrap().len() % 8,
            0
        );
        // bad program id rejected at open.
        let r = invoke(
            "open",
            &json!({ "state_path": state, "program_id_hex": "zz" }),
        );
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        // unknown op.
        let r = invoke_no_args("nope");
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    }

    #[test]
    fn ffi_program_id_round_trip() {
        let pid = program_id_from_hex(DOCUMENTED_PROGRAM_ID_HEX).unwrap();
        // Round trip: words back to the same hex.
        let mut bytes = Vec::with_capacity(32);
        for w in pid {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        assert_eq!(hex::encode(bytes), DOCUMENTED_PROGRAM_ID_HEX);
        assert!(program_id_from_hex("0123").is_err());
    }
}
