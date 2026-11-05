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
//! `update`, `catalog`, `lookup`, `lookup_bulk`. Transactions come back as
//! JSON for the wallet to submit — the FFI never signs or submits (same
//! split as the CLI).

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::regions::{Level, Region, REGIONS};
use crate::registry::{
    build_init, build_register_region, build_register_regions_batch, OsmTxBuilt,
};
use crate::storage::{CodexStorage, MemoryStorage, Storage};
use crate::OsmClient;

/// The **image id** of the `osm-registry` guest bytecode, hex (big-endian
/// bytes of the `[u32; 8]` id). Kept in sync with `methods/osm-host`'s
/// `DOCUMENTED_ID_HEX`; the integration-test crate has a consistency test
/// asserting the embedded artifact still hashes to this.
///
/// This is the bytecode identity, not the program's address. Under the v0.3
/// model a deployed program lives at an account the deployer chose, and PDAs
/// and transactions key off that account id (`program_account_hex`), not off
/// this value.
pub const DOCUMENTED_IMAGE_ID_HEX: &str =
    "c272ec3c2fe93c809d0381533511aff676e4667cbefb52eb150cca30fba98e79";

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
    /// The deployed program's account id (hex). Empty until a deployment is
    /// recorded; tx-building ops require it, because v0.3 addresses a program
    /// by its chosen account, not by its image id.
    program_account_hex: String,
    /// LEZ sequencer JSON-RPC endpoint, read by the `lookup` op.
    sequencer_url: String,
    /// Last-known region PBF sizes (bytes). Cached so the app can render the
    /// picker instantly and refresh in the background.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    region_sizes: std::collections::BTreeMap<String, u64>,
    /// Last-known on-chain registration statuses (region -> status object,
    /// the exact shape the `lookup_bulk` op returns). Cached so the Regions
    /// tab renders instantly; `Re-check` refreshes it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    region_statuses: std::collections::BTreeMap<String, Value>,
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

/// Per-session configuration. Logos Core loads multiple modules into one
/// process, and each may call `open` with its own storage/cache config. A
/// single global state meant the second `open` silently overwrote the first,
/// so every op routed through whichever module opened last. Sessions fix
/// that: `open` accepts a `"session"` id, every other op accepts the same
/// optional id, and configs live side by side keyed by it. The C ABI is
/// unchanged; callers that never pass a session id get the original
/// single-state behavior (the most recently opened session).
static SESSIONS: Mutex<Option<HashMap<String, AppState>>> = Mutex::new(None);
static ACTIVE_SESSION: Mutex<String> = Mutex::new(String::new());

// ---------------------------------------------------------------------------
// Background jobs
//
// The RemoteObjects result channel silently drops the result of any op that
// runs longer than roughly a minute (observed: an 8-minute `host` completed
// fully server-side — bytes stored, catalog updated, core idle — while the
// task in the UI stayed "running" forever). It also serializes ops behind
// whatever is running, so one long op starves every other report.
//
// Long ops therefore run as jobs: the op returns a job id immediately (the
// dispatcher is free), the work continues on a std thread against the shared
// tokio runtime, and the UI polls the cheap `job_status` op — which always
// delivers, being milliseconds.
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Serialize)]
struct JobState {
    id: String,
    op: String,
    label: String,
    /// "running" | "done" | "error"
    state: String,
    started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
}

static JOBS: Mutex<Vec<JobState>> = Mutex::new(Vec::new());
static JOB_SEQ: AtomicU64 = AtomicU64::new(0);

fn job_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Register a job, spawn its work, and return the job id. The closure runs
/// on its own thread and may block freely; it must be `Send + 'static`.
fn spawn_job<F>(op: &str, label: String, work: F) -> String
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    let id = format!("job-{}", JOB_SEQ.fetch_add(1, Ordering::Relaxed));
    {
        let mut g = JOBS.lock().unwrap();
        // Keep the registry bounded: at most 12 finished jobs, oldest first.
        let finished = g.iter().filter(|j| j.state != "running").count();
        let mut to_drop = finished.saturating_sub(12);
        let mut i = 0;
        while i < g.len() && to_drop > 0 {
            if g[i].state != "running" {
                g.remove(i);
                to_drop -= 1;
            } else {
                i += 1;
            }
        }
        g.push(JobState {
            id: id.clone(),
            op: op.to_string(),
            label,
            state: "running".to_string(),
            started_at: job_now(),
            error: None,
            result: None,
        });
    }
    let done_id = id.clone();
    std::thread::spawn(move || {
        // A panic in the job thread must not leave the job "running"
        // forever — catch it and surface it as the job's error.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|p| Err(format!("job panicked: {p:?}")));
        if let Ok(mut g) = JOBS.lock() {
            if let Some(j) = g.iter_mut().find(|j| j.id == done_id) {
                match outcome {
                    Ok(v) => {
                        j.state = "done".to_string();
                        j.result = Some(v);
                    }
                    Err(e) => {
                        j.state = "error".to_string();
                        j.error = Some(e);
                    }
                }
            }
        }
    });
    id
}

/// Cheap status snapshot for one job — the op the UI polls.
fn op_job_status(args: &Value) -> *mut c_char {
    let Some(id) = str_arg(args, "job_id") else {
        return err("job_status requires job_id");
    };
    let g = JOBS.lock().unwrap();
    match g.iter().find(|j| j.id == id) {
        Some(j) => {
            let mut v = serde_json::to_value(j).unwrap_or_else(|_| json!({"ok": false}));
            // Vary the reply while running: the transport delivers results
            // via a `lastResult` property that no-ops on a byte-identical
            // value. While a job is running the serialized JobState is
            // otherwise identical poll-to-poll, so the second poll's reply
            // would never reach the UI and the poll loop would stall. Adding
            // elapsed (seconds since start) makes every running reply
            // distinct so the property sync always emits.
            if let Value::Object(ref mut map) = v {
                map.insert(
                    "elapsed".into(),
                    json!(job_now().saturating_sub(j.started_at)),
                );
            }
            ok(v)
        }
        None => err(format!("unknown job {id}")),
    }
}

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

/// Resolve the cache/state dir under the Logos data tree when the caller
/// gives no explicit path. Logos ui-host processes export `LOGOS_USER_DIR`
/// (the Basecamp per-user data root); defaulting there means a module that
/// calls `open({})` no longer drops `osm-cache/` and `osm-state.json` into the
/// host process's CWD. When the env var is unset (standalone CLI/tests), the
/// original CWD-relative defaults apply so behavior is unchanged outside Logos.
fn user_dir_base() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("LOGOS_USER_DIR") {
        return Some(PathBuf::from(v));
    }
    // Portable fallback when the host sets no data dir: a stable per-user
    // location instead of the process CWD (which a module host does not
    // keep stable). HOME covers unix, USERPROFILE covers Windows.
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".logos-osm"))
}

/// Resolve a path argument: explicit value wins, else `LOGOS_USER_DIR/<file>`,
/// else the bare filename (CWD-relative, the historical default).
fn resolve_path(explicit: Option<String>, default_file: &str) -> String {
    if let Some(p) = explicit {
        return p;
    }
    match user_dir_base() {
        Some(base) => base.join(default_file).display().to_string(),
        None => default_file.to_string(),
    }
}

/// Resolve which session an op runs against: an explicit `"session"` arg
/// wins, else the most recently opened session (the single-module default).
/// Returns `None` when no session has been opened yet.
fn resolve_session(args: &Value) -> Option<String> {
    if let Some(s) = str_arg(args, "session") {
        if !s.is_empty() {
            return Some(s);
        }
    }
    let active = ACTIVE_SESSION.lock().unwrap();
    if active.is_empty() {
        None
    } else {
        Some(active.clone())
    }
}

/// Run `f` against the session named in `args` (or the active one), holding
/// the sessions lock across the op.
fn with_state<R>(args: &Value, f: impl FnOnce(&mut AppState) -> anyhow::Result<R>) -> *mut c_char
where
    R: serde::Serialize,
{
    let Some(sid) = resolve_session(args) else {
        return err("osm not open: call open first");
    };
    let mut guard = SESSIONS.lock().unwrap();
    let sessions = guard.get_or_insert_with(HashMap::new);
    let Some(st) = sessions.get_mut(&sid) else {
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

fn account_from_hex(hex_str: &str) -> anyhow::Result<lee_core::account::AccountId> {
    let bytes = hex::decode(hex_str.trim()).map_err(|e| anyhow::anyhow!("account hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("account id must be 32 bytes, got {}", bytes.len()))?;
    Ok(lee_core::account::AccountId::new(arr))
}

/// The configured deployed program account, or an error telling the caller to
/// record one (the address a deploy produced).
fn require_program_account(st: &AppState) -> anyhow::Result<lee_core::account::AccountId> {
    if st.program_account_hex.trim().is_empty() {
        anyhow::bail!(
            "program_account_hex is not set: deploy the program, then open with the account \
             address the deployment returned"
        );
    }
    account_from_hex(&st.program_account_hex)
}

/// The wallet-submittable form of a built tx: the program account (target),
/// the shard selectors in guest order, the account that must sign, and the
/// borsh-encoded instruction.
fn tx_json(built: &OsmTxBuilt, signer: &lee_core::account::AccountId) -> Value {
    json!({
        "program_account_hex": hex::encode(built.program_account_id.value()),
        "accounts_hex": built
            .accounts
            .iter()
            .map(|a| hex::encode(a.value()))
            .collect::<Vec<_>>(),
        "signer_hex": hex::encode(signer.value()),
        "instruction_hex": hex::encode(&built.instruction),
        "encoding": "borsh",
    })
}

fn region_json(r: &Region) -> Value {
    json!({
        "path": r.path,
        "name": r.name,
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

/// Dispatch an op and tag the result JSON with the op name. Results are
/// delivered asynchronously (the host copies the return value into the
/// `lastResult` property), so a UI driving several ops at once cannot tell
/// which result belongs to which call. Embedding `"op"` in the payload makes
/// routing reliable regardless of completion order.
fn dispatch(name: &str, args: &Value) -> *mut c_char {
    let raw = dispatch_op(name, args);
    // SAFETY: `raw` was created by `CString::into_raw` in `to_json`/`err` and
    // is not null (to_json always returns a valid pointer).
    let owned = unsafe { CString::from_raw(raw) };
    let parsed: Value = serde_json::from_str(owned.to_str().unwrap_or("{}"))
        .unwrap_or_else(|_| json!({"ok": false, "error": "unparseable core result"}));
    let mut tagged = parsed;
    if let Value::Object(ref mut map) = tagged {
        map.entry("op").or_insert(json!(name));
    }
    to_json(tagged)
}

fn dispatch_op(name: &str, args: &Value) -> *mut c_char {
    match name {
        "version" => to_json(json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "name": env!("CARGO_PKG_NAME"),
        })),
        "open" => op_open(args),
        "status" => with_state(args, |st| {
            Ok(json!({
                "storage_url": st.storage_url,
                "geofabrik_url": st.geofabrik_url,
                "cache_dir": st.cache_dir,
                "memory_storage": st.memory,
                "program_account_hex": st.program_account_hex,
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
        "sizes" => op_sizes(args),
        "job_status" => op_job_status(args),
        "discover" => op_discover(args),
        "host" => op_host(args),
        "host_bulk" => op_host_bulk(args),
        "register" => op_register(args),
        "register_bulk" => op_register_bulk(args),
        "init" => op_init(args),
        "fetch" => op_fetch(args),
        "import" => op_import(args),
        "update" => op_update(args),
        "lookup" => op_lookup(args),
        "lookup_bulk" => op_lookup_bulk(args),
        "catalog" => with_state(args, |st| {
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
    // Default cache/state paths land under LOGOS_USER_DIR (the Basecamp data
    // tree) when the caller gives none and the env var is set, so a module
    // that calls open({}) no longer drops files into the host process CWD.
    // Outside Logos (no LOGOS_USER_DIR) the bare filenames apply, preserving
    // the CLI/tests behavior.
    let state_path = resolve_path(str_arg(args, "state_path"), "osm-state.json");
    let cache_dir = resolve_path(str_arg(args, "cache_dir"), "osm-cache");
    let loaded = std::fs::read(&state_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<AppState>(&b).ok());
    let mut st = loaded.unwrap_or_else(|| AppState {
        storage_url: str_arg(args, "storage_url")
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
        geofabrik_url: str_arg(args, "geofabrik_url")
            .unwrap_or_else(|| crate::geofabrik::DEFAULT_BASE.to_string()),
        cache_dir: cache_dir.clone(),
        memory: args
            .get("memory")
            .and_then(|m| m.as_bool())
            .unwrap_or(false),
        program_account_hex: String::new(),
        sequencer_url: str_arg(args, "sequencer_url")
            .unwrap_or_else(|| "https://testnet.lez.logos.co".to_string()),
        region_sizes: std::collections::BTreeMap::new(),
        region_statuses: std::collections::BTreeMap::new(),
        state_path: state_path.clone(),
    });
    // Refresh any provided config (lets the app re-point at live endpoints).
    if let Some(v) = str_arg(args, "storage_url") {
        st.storage_url = v;
    }
    if let Some(v) = str_arg(args, "geofabrik_url") {
        st.geofabrik_url = v;
    }
    // Only an explicit cache_dir arg overrides a loaded state's cache dir;
    // the resolved default applies just to freshly-created state.
    if let Some(v) = str_arg(args, "cache_dir") {
        st.cache_dir = v;
    }
    if let Some(v) = args.get("memory").and_then(|m| m.as_bool()) {
        st.memory = v;
    }
    if let Some(v) = str_arg(args, "sequencer_url") {
        st.sequencer_url = v;
    }
    if let Some(v) = str_arg(args, "program_account_hex") {
        // Validate early so a typo can't poison every later tx build.
        if let Err(e) = account_from_hex(&v) {
            return err(format!("program_account_hex: {e}"));
        }
        st.program_account_hex = v;
    }
    st.state_path = state_path;
    let _ = std::fs::create_dir_all(&st.cache_dir);
    let summary = json!({
        "storage_url": st.storage_url,
        "geofabrik_url": st.geofabrik_url,
        "cache_dir": st.cache_dir,
        "memory_storage": st.memory,
        "program_account_hex": st.program_account_hex,
        "catalog": client(&st).catalog().len(),
        // Cached region sizes so the app can render the picker before the
        // live `sizes` refresh completes (empty until the first fetch).
        "region_sizes": st.region_sizes,
        // Cached on-chain registration statuses, same deal: the Regions tab
        // renders badges from these before the first `lookup_bulk` refresh.
        "region_statuses": st.region_statuses,
    });
    if let Err(e) = st.save() {
        return err(format!("{e:#}"));
    }
    // Store under the caller's session id (default "default"), and make it
    // the active session so ops without an explicit session target it.
    let sid = str_arg(args, "session").unwrap_or_else(|| "default".to_string());
    {
        let mut guard = SESSIONS.lock().unwrap();
        guard
            .get_or_insert_with(HashMap::new)
            .insert(sid.clone(), st);
    }
    *ACTIVE_SESSION.lock().unwrap() = sid;
    ok(summary)
}

/// Clone the session state named in `args` out from under the lock, so async
/// ops (which block on the tokio runtime for seconds) never hold the sessions
/// mutex while they run. `AppState` is config-only (endpoints, cache dir,
/// program id), so a clone is cheap and races with a concurrent `open` are
/// impossible by construction.
fn state_snapshot(args: &Value) -> Option<AppState> {
    let sid = resolve_session(args)?;
    let guard = SESSIONS.lock().unwrap();
    guard.as_ref()?.get(sid.as_str()).cloned()
}

/// Live upstream cross-check of the closed region set (needs network).
fn op_discover(args: &Value) -> *mut c_char {
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let c = client(&st);
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
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    // Job mode: a real host runs minutes (region downloads are 50 MB–6 GB),
    // which outruns the result channel and starves every op queued behind
    // it. Return a job id immediately; the UI polls job_status.
    if args.get("job").and_then(|b| b.as_bool()).unwrap_or(false) {
        let job_region = region.clone();
        let job_st = st.clone();
        let job_reg = registrar;
        let id = spawn_job("host", format!("host {job_region}"), move || {
            let c = client(&job_st);
            let snap = rt()
                .block_on(c.host_region(&job_region))
                .map_err(|e| format!("host {job_region}: {e:#}"))?;
            c.catalog_record_hosted(&snap)
                .map_err(|e| format!("catalog: {e:#}"))?;
            let tx = job_reg.as_ref().and_then(|reg| {
                let account = require_program_account(&job_st).ok()?;
                let built = build_register_region(&account, reg, &snap.registration(None));
                Some(tx_json(&built, reg))
            });
            Ok(json!({
                "region": snap.region,
                "cid": snap.cid,
                "version": snap.version,
                "bytes": snap.bytes,
                "md5": hex::encode(snap.checksum),
                "path": snap.path.display().to_string(),
                "tx": tx,
            }))
        });
        return ok(json!({ "job_id": id, "op": "host", "region": region }));
    }
    let c = client(&st);
    let snap = match block_on(c.host_region(&region)) {
        Ok(s) => s,
        Err(e) => return err(format!("host {region}: {e:#}")),
    };
    if let Err(e) = c.catalog_record_hosted(&snap) {
        return err(format!("catalog: {e:#}"));
    }
    let program_account = match require_program_account(&st) {
        Ok(a) => a,
        Err(e) => return err(format!("{e:#}")),
    };
    let tx = registrar.as_ref().map(|reg| {
        let built = build_register_region(&program_account, reg, &snap.registration(None));
        tx_json(&built, reg)
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

/// Live extract sizes (HEAD Content-Length per region), for the app's
/// region picker and cumulative hosting estimates. Takes an optional
/// comma-separated `regions` subset (default: the whole frozen set) and
/// uses the session's Geofabrik base when open, the default otherwise.
fn op_sizes(args: &Value) -> *mut c_char {
    let wanted: Vec<String> = match str_arg(args, "regions") {
        Some(spec) if spec != "all" => spec
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => REGIONS.iter().map(|r| r.path.to_string()).collect(),
    };
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let base = state_snapshot(args)
        .map(|st| st.geofabrik_url)
        .unwrap_or_else(|| crate::geofabrik::DEFAULT_BASE.to_string());
    let g = crate::geofabrik::GeofabrikClient::with_base(&base);
    let sizes = match block_on(g.sizes(&wanted)) {
        Ok(s) => s,
        Err(e) => return err(format!("sizes: {e:#}")),
    };
    let total: u64 = sizes.iter().map(|(_, n)| *n).sum();
    let have: Vec<&str> = sizes.iter().map(|(id, _)| id.as_str()).collect();
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|r| !have.contains(r))
        .copied()
        .collect();
    let map: std::collections::BTreeMap<&str, u64> =
        sizes.iter().map(|(id, n)| (id.as_str(), *n)).collect();
    // Persist the fresh sizes into the session state so the next `open`
    // returns them (the app renders the picker from the cache instantly and
    // lets this live fetch refresh it). MERGE, not replace: a subset call
    // (`regions: "a,b"`) refreshes only those entries — the rest of the
    // cached map survives, so a subset fetch can no longer shrink the cache
    // (the picker kept losing sizes it already had). Skipped when nothing
    // changed.
    if let Some(mut st) = state_snapshot(args) {
        let mut changed = false;
        for (k, v) in map.iter().map(|(k, v)| (k.to_string(), *v)) {
            if st.region_sizes.insert(k, v) != Some(v) {
                changed = true;
            }
        }
        if changed {
            if let Err(e) = st.save() {
                return err(format!("{e:#}"));
            }
            if let Some(sid) = resolve_session(args) {
                let mut guard = SESSIONS.lock().unwrap();
                if let Some(m) = guard.as_mut() {
                    m.insert(sid, st);
                }
            }
        }
    }
    ok(json!({
        "sizes": map,
        "count": map.len(),
        "requested": wanted.len(),
        "missing": missing,
        "total_bytes": total,
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
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let c = client(&st);
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
    with_state(args, |st| {
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
        let program_account = require_program_account(st)?;
        let built = build_register_region(&program_account, &registrar, &snap.registration(None));
        Ok(tx_json(&built, &registrar))
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
    with_state(args, |st| {
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
        let program_account = require_program_account(st)?;
        let built = build_register_regions_batch(&program_account, &registrar, &regs);
        Ok(tx_json(&built, &registrar))
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
    with_state(args, |st| {
        let program_account = require_program_account(st)?;
        let built = build_init(&program_account, &owner);
        Ok(tx_json(&built, &owner))
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
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    // Job mode: fetching pulls the full region PBF from Logos Storage —
    // minutes for big regions, which outruns the result channel. Same
    // pattern as `host`: job id now, poll job_status for the outcome.
    if args.get("job").and_then(|b| b.as_bool()).unwrap_or(false) {
        let (jr, jc, jm) = (region.clone(), cid.clone(), checksum);
        let job_st = st.clone();
        let id = spawn_job("fetch", format!("fetch {jr}"), move || {
            let c = client(&job_st);
            let outcome = rt()
                .block_on(c.fetch_snapshot(&jr, &jc, &jm))
                .map_err(|e| format!("fetch {jr}: {e:#}"))?;
            let summary = rt()
                .block_on(c.import_local(&jr, &outcome.path))
                .map_err(|e| format!("import {jr}: {e:#}"))?;
            Ok(json!({
                "region": summary.region,
                "bytes": summary.bytes,
                "blobs": summary.blobs,
                "md5_verified": hex::encode(summary.md5),
                "path": outcome.path.display().to_string(),
            }))
        });
        return ok(json!({ "job_id": id, "op": "fetch", "region": region }));
    }
    let c = client(&st);
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
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let c = client(&st);
    let path = std::path::Path::new(&pbf);
    if store {
        let snap = match block_on(c.host_local(&region, path)) {
            Ok(s) => s,
            Err(e) => return err(format!("import {region}: {e:#}")),
        };
        if let Err(e) = c.catalog_record_hosted(&snap) {
            return err(format!("catalog: {e:#}"));
        }
        let program_account = match require_program_account(&st) {
            Ok(a) => a,
            Err(e) => return err(format!("{e:#}")),
        };
        let tx = registrar.as_ref().map(|reg| {
            let built = build_register_region(&program_account, reg, &snap.registration(None));
            tx_json(&built, reg)
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

/// Resolve a region to its on-chain registry entry: the CID, source URL,
/// checksum, version and mirrors recorded for it. This is the consumer path —
/// a Basecamp app that wants map data needs to turn a region into a CID, not
/// to host anything.
///
/// Reads the sequencer directly over JSON-RPC, so it needs no wallet.
fn op_lookup(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("lookup requires region");
    };
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let program_account = match require_program_account(&st) {
        Ok(a) => a,
        Err(e) => return err(format!("{e:#}")),
    };

    let pda = crate::registry::region_pda(&program_account, &region);
    let account = match block_on(async {
        let client = sequencer_service_rpc::SequencerClientBuilder::default()
            .build(st.sequencer_url.clone())
            .map_err(|e| anyhow::anyhow!("sequencer {}: {e}", st.sequencer_url))?;
        use sequencer_service_rpc::RpcClient as _;
        client
            .get_account(pda)
            .await
            .map_err(|e| anyhow::anyhow!("reading {region}: {e}"))
    }) {
        Ok(a) => a,
        Err(e) => return err(format!("{e:#}")),
    };

    let shard = account.data.shard(program_account);
    if shard.as_ref().is_empty() {
        return err(format!("{region} is not registered on chain"));
    }
    let entry: crate::registry::RegionEntry = match borsh::from_slice(shard.as_ref()) {
        Ok(e) => e,
        Err(e) => return err(format!("decoding {region}: {e}")),
    };
    let latest = entry.latest_mirror();
    ok(json!({
        "region": entry.region,
        "parent": entry.parent,
        "level": entry.level,
        "mirrors": entry.mirrors.len(),
        "registrars": entry.registrars().len(),
        "latest": latest.map(|m| json!({
            "cid": m.cid,
            "source_url": m.source_url,
            "checksum": hex::encode(m.checksum),
            "version": m.version,
            "timestamp": m.timestamp,
        })),
    }))
}

/// Concurrently read the on-chain registration status for many regions —
/// the bulk form of [`op_lookup`], shaped like [`op_sizes`] (optional comma
/// `regions` subset, `"all"`/absent = the whole frozen set; 8 reads in
/// flight). A region whose read fails keeps its `error` field instead of
/// failing the whole op (a partial answer beats none).
///
/// Returns `{ statuses: { <region>: { registered, latest, cid, error? } },
/// count, registered_count, error_count }` and MERGES the statuses into the
/// session state (same merge semantics as the sizes cache: fetched entries
/// update, the rest survive), so the next `open` serves them from disk.
fn op_lookup_bulk(args: &Value) -> *mut c_char {
    let wanted: Vec<String> = match str_arg(args, "regions") {
        Some(spec) if spec != "all" => spec
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => REGIONS.iter().map(|r| r.path.to_string()).collect(),
    };
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let program_account = match require_program_account(&st) {
        Ok(a) => a,
        Err(e) => return err(format!("{e:#}")),
    };
    let resolved = match block_on(crate::registry::resolve_regions_bulk(
        &st.sequencer_url,
        &program_account,
        &wanted,
    )) {
        Ok(r) => r,
        Err(e) => return err(format!("lookup_bulk: {e:#}")),
    };
    let mut statuses: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut registered_count = 0u64;
    let mut error_count = 0u64;
    for (region, res) in resolved {
        let entry = match res {
            // An on-chain entry exists: registered. (latest/cid come from the
            // newest mirror; an entry with no mirrors is degenerate but
            // honest to report as registered with no version.)
            Ok(Some(e)) => {
                registered_count += 1;
                let latest = e.latest_mirror();
                json!({
                    "registered": true,
                    "latest": latest.map(|m| m.version.to_string()),
                    "cid": latest.map(|m| m.cid.clone()),
                })
            }
            Ok(None) => json!({
                "registered": false,
                "latest": Value::Null,
                "cid": Value::Null,
            }),
            Err(e) => {
                error_count += 1;
                json!({
                    "registered": false,
                    "latest": Value::Null,
                    "cid": Value::Null,
                    "error": format!("{e:#}"),
                })
            }
        };
        statuses.insert(region, entry);
    }
    let result = json!({
        "statuses": statuses,
        "count": statuses.len(),
        "registered_count": registered_count,
        "error_count": error_count,
    });
    // Persist the fresh statuses into the session state (MERGE, not
    // replace — same contract as op_sizes: a subset call refreshes only the
    // fetched regions, so the cache can only grow more current).
    if let Some(mut st) = state_snapshot(args) {
        let mut changed = false;
        if let Value::Object(ref map) = result["statuses"] {
            for (k, v) in map {
                if st.region_statuses.insert(k.clone(), v.clone()) != Some(v.clone()) {
                    changed = true;
                }
            }
        }
        if changed {
            if let Err(e) = st.save() {
                return err(format!("{e:#}"));
            }
            if let Some(sid) = resolve_session(args) {
                let mut guard = SESSIONS.lock().unwrap();
                if let Some(m) = guard.as_mut() {
                    m.insert(sid, st);
                }
            }
        }
    }
    ok(result)
}

/// Check whether a newer snapshot is published upstream.
fn op_update(args: &Value) -> *mut c_char {
    let Some(region) = str_arg(args, "region") else {
        return err("update requires region");
    };
    let Some(st) = state_snapshot(args) else {
        return err("osm not open: call open first");
    };
    let c = client(&st);
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

    /// The program account the tests configure (hex, 32 bytes).
    const PROGRAM_ACCOUNT_HEX: &str =
        "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

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
            &json!({
                "state_path": state,
                "cache_dir": cache,
                "memory": true,
                // The address a deployment would have returned.
                "program_account_hex": PROGRAM_ACCOUNT_HEX,
            }),
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
            r["result"]["program_account_hex"].as_str().unwrap(),
            PROGRAM_ACCOUNT_HEX
        );
        assert_eq!(r["result"]["encoding"].as_str().unwrap(), "borsh");
        // The instruction is borsh bytes now; decode them back (the guest's
        // exact decode path) and check they name the owner we passed.
        let words = hex::decode(r["result"]["instruction_hex"].as_str().unwrap()).unwrap();
        let back: crate::registry::Instruction = borsh::from_slice(&words).unwrap();
        assert_eq!(
            back,
            crate::registry::Instruction::Init { owner: [0x07; 32] }
        );
        // bad program id rejected at open.
        let r = invoke(
            "open",
            &json!({ "state_path": state, "program_account_hex": "zz" }),
        );
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        // unknown op.
        let r = invoke_no_args("nope");
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    }

    #[test]
    fn program_account_round_trip() {
        // The configured program account parses back to the same 32 bytes.
        let hex_id = "0f".repeat(32);
        let a = account_from_hex(&hex_id).unwrap();
        assert_eq!(hex::encode(a.value()), hex_id);
        assert!(account_from_hex("0123").is_err());
    }

    /// `lookup_bulk` against an unreachable sequencer: the op still succeeds
    /// (a partial answer beats none — the `sizes` contract), every region
    /// carries its own error, the dispatch tags the result with the op name,
    /// and the statuses MERGE into the cached state (a subset fetch keeps the
    /// earlier entries). Runs on its own session so the parallel default-
    /// session test cannot race it.
    #[test]
    fn ffi_lookup_bulk_shape_and_state_cache() {
        let (state, cache) = fresh_state();
        // Before open: the not-open error.
        let r = invoke(
            "lookup_bulk",
            &json!({ "regions": "germany,france", "session": "bulk-test" }),
        );
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        // Open with no program account recorded -> the op names what to set.
        let r = invoke(
            "open",
            &json!({
                "state_path": state, "cache_dir": cache, "memory": true,
                "session": "bulk-test",
            }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        let r = invoke(
            "lookup_bulk",
            &json!({ "regions": "germany", "session": "bulk-test" }),
        );
        assert!(!r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
        assert!(
            r["error"].as_str().unwrap().contains("program_account_hex"),
            "{}",
            r["error"]
        );
        // With a program account but an unreachable sequencer
        // (127.0.0.1:9 — nothing listens): ok, per-region error statuses.
        let r = invoke(
            "open",
            &json!({
                "state_path": state, "cache_dir": cache, "memory": true,
                "program_account_hex": PROGRAM_ACCOUNT_HEX,
                "sequencer_url": "http://127.0.0.1:9/",
                "session": "bulk-test",
            }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        let r = invoke(
            "lookup_bulk",
            &json!({ "regions": "germany,france", "session": "bulk-test" }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        // dispatch() tags the result with the core op name.
        assert_eq!(r["op"].as_str().unwrap(), "lookup_bulk");
        let res = &r["result"];
        assert_eq!(res["count"].as_u64(), Some(2));
        assert_eq!(res["registered_count"].as_u64(), Some(0));
        assert_eq!(res["error_count"].as_u64(), Some(2));
        for region in ["germany", "france"] {
            let s = &res["statuses"][region];
            assert_eq!(s["registered"].as_bool(), Some(false), "{s}");
            assert!(
                s["error"].as_str().unwrap().contains(region),
                "error should name the region: {s}"
            );
        }
        // The statuses were cached: the next open returns them.
        let r = invoke(
            "open",
            &json!({
                "state_path": state, "cache_dir": cache, "memory": true,
                "program_account_hex": PROGRAM_ACCOUNT_HEX,
                "session": "bulk-test",
            }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        assert_eq!(
            r["result"]["region_statuses"]["germany"]["registered"].as_bool(),
            Some(false)
        );
        // MERGE: a subset fetch (kenya) keeps the previously cached entries.
        let r = invoke(
            "lookup_bulk",
            &json!({ "regions": "kenya", "session": "bulk-test" }),
        );
        assert!(
            r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            "{r}"
        );
        let r = invoke(
            "open",
            &json!({
                "state_path": state, "cache_dir": cache, "memory": true,
                "program_account_hex": PROGRAM_ACCOUNT_HEX,
                "session": "bulk-test",
            }),
        );
        let cached = r["result"]["region_statuses"].as_object().unwrap();
        for region in ["germany", "france", "kenya"] {
            assert!(cached.contains_key(region), "missing {region}: {cached:?}");
        }
    }
}
