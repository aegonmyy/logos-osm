import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The bundled guide documents (markdown as string constants). Lives in
// guide/GuideDocs.js next to this file — the module builder packages the
// whole qml tree, so it ships inside the .lgx (see the Guide tab).
import "guide/GuideDocs.js" as GuideDocs

/*
 * OpenStreetMap on Logos — data-distribution control panel.
 *
 * Styled to the logos.co identity (tokens taken from the live site):
 * a dark green canvas, off-white ink, one yellow accent used with
 * discipline, tan/teal/light-blue supporting tones, serif section
 * headings, sans UI text, and mono for every machine value.
 *
 * The backend contract is unchanged: logos.module("osm_app") and
 * invokeOp(op, argsJson) exactly as before.
 */

Item {
    id: root

    // ---- Backend ---------------------------------------------------------
    // The OsmApp replica (RemoteObjects). Its invokeOp(name, argsJson)
    // forwards to the core osm module, which dlopens liblogos_osm.so and
    // runs the Rust SDK.
    readonly property var    bk:          logos.module("osm_app")
    readonly property string osmVersion:  bk ? bk.osmVersion : ""
    readonly property string lastErr:     bk ? bk.lastErr    : ""

    property int currentTab: 0

    // ---- Async op dispatch ----------------------------------------------
    // `bk` is a Qt RemoteObjects DYNAMIC replica: a QML method call on it
    // fires the op and returns a default value immediately — the real result
    // comes back as the `lastResult` PROPERTY (the plugin sets it before
    // returning, and property sync is the channel that works). So: call()
    // fires the op and records what we asked for; the Connections block
    // below parses lastResult when it changes and routes it.
    property string pendingOp: ""
    // Region picker state: the frozen set with live sizes (fetched after
    // startup), the picker's selection, and the locally hosted set (from the
    // catalog op) for the ever-valid "host all".
    property var    regionList: []          // [{path, name, level}]
    property var    regionSizes: ({})       // {path: bytes}
    property bool   sizesLoading: false
    property var    selectedRegions: []     // [path]
    property var    hostedRegions: []       // [path] from the catalog
    // Full catalog records (region, version, cid, md5, path) from the last
    // `catalog` result — the local `path` is what "use cached file" fills.
    property var    catalogRecords: []
    property string discoverState: "pending"   // pending | ok | error
    property int    regionsAvailable: 0
    property int    regionsTotal: 72
    property string discoverError: ""

    // On-chain registration statuses (Regions tab): merged map
    // {region: {registered, latest, cid, error?}} from `lookup_bulk` and the
    // state-file cache the open result seeds. Registered-first sort + the
    // All/Registered/Not filter render from this.
    property var    regionStatuses: ({})
    property string regionFilter: "all"     // all | registered | not

    // Storage reachability, from the open result: unknown | ok | fail.
    // Drives the dot next to the STORAGE label in Host §01.
    property string storageReach: "unknown"

    // Consumer tab: one single-select region per section, its own picker.
    property string fetchRegion: ""
    property string importRegion: ""
    property string updateRegion: ""
    // The §01 auto-fill chain state: none | pending | ok | unregistered |
    // unreachable — drives the CID caption and field fills.
    property string fetchFillState: "none"
    // Correlation for the §01 auto-fill: the region whose lookup is in
    // flight (results arrive tagged only with the core op name "lookup").
    property string fetchLookupRegion: ""

    // Guide tab: the selected document id (see guide/GuideDocs.js).
    property string guideDoc: "how-it-works"

    // ---- Ongoing tasks (the tasks panel) ---------------------------------
    // Every call() registers a task; onOpResult resolves the oldest running
    // task whose op matches the result. Completed tasks are pruned to the
    // last ~8. Running rows tick off one shared 1 Hz counter (taskTick),
    // not per-row Timers.
    property var tasks: []            // [{id, op, label, args, startedAt, state, summary}]
    property int taskSeq: 0
    property int taskTick: 0          // 1 Hz heartbeat while anything runs
    property int runningTasks: root.tasks.filter(t => t.state === "running").length
    property int tasksErrorCount: root.tasks.filter(t => t.state === "error").length
    property string tasksChipText: {
        if (root.tasks.length === 0) return "idle"
        const parts = []
        if (root.runningTasks > 0) parts.push(root.runningTasks + " running")
        const done = root.tasks.length - root.runningTasks - root.tasksErrorCount
        if (done > 0) parts.push(done + " done")
        if (root.tasksErrorCount > 0)
            parts.push(root.tasksErrorCount + (root.tasksErrorCount > 1 ? " errors" : " error"))
        return parts.length > 0 ? parts.join(" · ") : "idle"
    }

    // The sequential host-loop driver (startHostRun / hostNext /
    // onHostResult): the three Host §02 buttons fire the single-region
    // `host` op one at a time instead of one host_bulk, so progress shows
    // per region. hostLoopActive also guards against a second run while one
    // is in flight (the transport drops ops fired concurrently).
    property bool hostLoopActive: false
    property var hostQueue: []          // [path] still to host this run
    property int hostTotal: 0           // queue length when the run started
    property int hostDoneCount: 0
    property int hostBytes: 0           // bytes stored so far this run
    property string hostCurrent: ""     // region whose host op is in flight
    property int hostTaskId: -1         // the grouped task's id

    // Background jobs: long ops (host, fetch) outrun the RemoteObjects
    // result channel — anything past roughly a minute is silently dropped —
    // and while one runs, every op queued behind it starves. The core runs
    // them as jobs instead: the op returns a job id instantly, and this
    // side polls the cheap `job_status` op (milliseconds, always delivers).
    // jobPolls maps job_id -> {taskId, host} while a poll is outstanding.
    property var jobPolls: ({})
    // A poll is "in flight" until its result lands — but the result channel
    // (the `lastResult` property sync) SILENTLY DROPS a value that is byte-
    // identical to the previous one (the generated setLastResult short-
    // circuits on equalStrings and never emits lastResultChanged). While a
    // job is running, every job_status reply is identical, so after the first
    // poll the second's result never reaches onJobStatus and the boolean
    // would stick true forever — killing the poll loop and leaving the task
    // "running" long after the job finished server-side.
    //
    // Track a deadline instead: if no result has re-armed us within a few
    // ticks, assume the reply was dropped/deduped and re-fire. The Rust side
    // also varies the running result (elapsed_secs) so the normal path still
    // delivers; the deadline is defense in depth against any dropped frame.
    property bool jobStatusInFlight: false
    property int jobStatusPollDeadline: 0   // taskTick at which to re-arm

    // Auto-init: open the SDK session on startup; discover follows once the
    // session is up. The user never thinks about "Open" — the fields above
    // are applied automatically, and editing them re-fires on the next
    // restart (state persists under the module data tree).
    function openArgs() {
        const args = {
            // No state_path: the module host's CWD is not stable (it can be
            // a deleted dir) and no hardcoded absolute path is portable
            // across machines, so the core resolves a per-user location
            // (LOGOS_USER_DIR, else ~/.logos-osm, else the CWD). Same for
            // the cache dir: only pass it when the user typed something
            // other than the placeholder default.
            storage_url: storageUrl.text,
            geofabrik_url: geofabrikUrl.text,
            // The registry this app talks to on the live LEZ testnet: the
            // OSM guest program deployed 2026-10-30.
            program_account_hex: "b2fa65027d9b2540d4ac2aa4e0a38040ac03604c2201930592c547f3d65ecb43"
        }
        const cache = cacheDir.text.trim()
        if (cache.length > 0 && cache !== "osm-cache") args.cache_dir = cache
        if (registrar.text.length) args.registrar_hex = registrar.text
        return args
    }

    function runDiscover() {
        root.discoverState = "pending"
        root.call("discover")
    }

    // The compact CORE chip version: parse the version JSON the backend
    // surfaces (osmVersion holds the raw JSON string).
    function coreVersion() {
        try {
            const j = JSON.parse(root.osmVersion)
            return j.version || ""
        } catch (e) { return "" }
    }

    // The local catalog record for a region (or null) — "use cached file"
    // fills the PBF FILE field from its `path`.
    function catalogRecordFor(region) {
        if (!region) return null
        for (let i = 0; i < root.catalogRecords.length; i++)
            if (root.catalogRecords[i].region === region) return root.catalogRecords[i]
        return null
    }
    function cachedPathFor(region) {
        const rec = root.catalogRecordFor(region)
        return (rec && rec.path) ? rec.path : ""
    }

    function fmtBytes(n) {
        if (n >= 1e9) return (n / 1e9).toFixed(1) + " GB"
        if (n >= 1e6) return (n / 1e6).toFixed(0) + " MB"
        return (n / 1e3).toFixed(0) + " KB"
    }
    function bytesOf(paths) {
        let t = 0
        for (const p of paths) if (root.regionSizes[p] !== undefined) t += root.regionSizes[p]
        return t
    }
    function toggleRegion(path) {
        root.selectedRegions = root.toggleIn(root.selectedRegions, path)
    }
    // Shared picker toggle: returns a fresh array, so reassigning the
    // selection property notifies every binding that reads it.
    function toggleIn(list, path) {
        const i = list.indexOf(path)
        if (i >= 0) list.splice(i, 1)
        else list.push(path)
        return list.slice()
    }

    // ---- Regions tab: statuses, sort, filter --------------------------------
    // Rows for the Regions model: the frozen list joined with the merged
    // on-chain statuses, filtered by the active toggle, registered first
    // (stable within each group by path).
    function regionRows() {
        let rows = []
        for (let i = 0; i < root.regionList.length; i++) {
            const g = root.regionList[i]
            const s = root.regionStatuses[g.path]
            rows.push({
                path: g.path, name: g.name, level: g.level,
                registered: s ? (s.registered === true) : false,
                latest: (s && s.latest) ? String(s.latest) : "",
                error: (s && s.error) ? String(s.error) : ""
            })
        }
        if (root.regionFilter === "registered")
            rows = rows.filter(r => r.registered)
        else if (root.regionFilter === "not")
            rows = rows.filter(r => !r.registered)
        rows.sort((a, b) => {
            if (a.registered !== b.registered) return a.registered ? -1 : 1
            return a.path < b.path ? -1 : (a.path > b.path ? 1 : 0)
        })
        return rows
    }

    function rebuildRegionModel() {
        regionModel.clear()
        const rows = root.regionRows()
        for (let i = 0; i < rows.length; i++) regionModel.append(rows[i])
    }

    function setRegionFilter(f) {
        if (root.regionFilter === f) return
        root.regionFilter = f
        root.rebuildRegionModel()
    }

    // Registered count across the whole set (the header line, not the
    // filtered view).
    function registeredShown() {
        let n = 0
        for (const k in root.regionStatuses)
            if (root.regionStatuses[k] && root.regionStatuses[k].registered === true) n++
        return n
    }

    // The Regions tab auto-checks the registry the first time it is opened
    // in a session with no statuses yet (the open result may have seeded
    // them from the state-file cache — then the user can Re-check).
    function maybeCheckStatuses() {
        if (root.currentTab !== 1) return
        if (Object.keys(root.regionStatuses).length > 0) return
        root.call("lookup_bulk", {})
    }

    // ---- Consumer tab: per-section single-select + the §01 fill chain -----
    function setFetchRegion(path) {
        root.fetchRegion = path
        // Clear the previous region's values; the lookup that follows fills
        // them (or explains why it can't).
        fCid.text = ""
        fMd5.text = ""
        root.fetchLookupRegion = ""
        if (path === "") {
            root.fetchFillState = "none"
            return
        }
        root.fetchFillState = "pending"
        root.fetchLookupRegion = path
        root.call("lookup", { region: path })
    }
    function setImportRegion(path) {
        root.importRegion = path
    }
    function setUpdateRegion(path) {
        root.updateRegion = path
    }
    Component.onCompleted: {
        root.call("open", root.openArgs())
    }

    function call(op, args, tag) {
        if (!root.bk) return
        root.pendingOp = tag || op
        const a = args || {}
        // Every fired op becomes a task the panel tracks to completion.
        root.addTask(op, root.taskLabel(op, a), a)
        root.bk.invokeOp(op, JSON.stringify(a))
    }

    function pretty(obj) { return JSON.stringify(obj, null, 2) }

    // ---- Task bookkeeping ---------------------------------------------------
    function addTask(op, label, args) {
        const t = {
            id: ++root.taskSeq,
            op: op,             // core op name ("host-run" for the loop's group task)
            label: label,
            args: args || {},   // kept for summaries the result doesn't echo
            startedAt: Date.now(),
            state: "running",   // running | done | error
            summary: ""
        }
        root.tasks = root.tasks.concat([t])
        return t.id
    }

    function taskIndex(id) {
        for (let i = 0; i < root.tasks.length; i++)
            if (root.tasks[i].id === id) return i
        return -1
    }

    // Patch one task: new object + array reassignment so the panel re-renders.
    function updateTask(id, patch) {
        const i = root.taskIndex(id)
        if (i < 0) return
        const arr = root.tasks.slice()
        arr[i] = Object.assign({}, root.tasks[i], patch)
        root.tasks = arr
        if (patch.state !== undefined && patch.state !== "running") root.pruneTasks()
    }

    // Keep the last ~8 completed tasks (running ones always stay).
    function pruneTasks() {
        const completedAt = []
        for (let i = 0; i < root.tasks.length; i++)
            if (root.tasks[i].state !== "running") completedAt.push(i)
        const excess = completedAt.length - 8
        if (excess <= 0) return
        const drop = completedAt.slice(0, excess)     // oldest completed first
        root.tasks = root.tasks.filter((t, i) => drop.indexOf(i) < 0)
    }

    function fmtElapsed(startedAt) {
        const s = Math.max(0, Math.floor((Date.now() - startedAt) / 1000))
        const m = Math.floor(s / 60)
        const r = s % 60
        return (m < 10 ? "0" : "") + m + ":" + (r < 10 ? "0" : "") + r
    }

    function taskLabel(op, args) {
        const region = args && args.region ? args.region : ""
        switch (op) {
            case "host":     return region ? "host " + region : "host"
            case "fetch":    return region ? "fetch " + region : "fetch"
            case "import":   return region ? "import " + region : "import"
            case "update":   return region ? "update " + region : "update"
            case "lookup":   return region ? "lookup " + region : "lookup"
            case "lookup_bulk": {
                const n = args && args.regions && args.regions !== "all"
                              ? String(args.regions).split(",").filter(s => s.trim().length > 0).length
                              : root.regionList.length > 0 ? root.regionList.length : root.regionsTotal
                return "registry check (" + n + " regions)"
            }
            case "register": return region ? "register " + region : "register"
            case "register_bulk": return "register batch"
            case "host_bulk":     return "host bulk"
            case "sizes": {
                const n = root.regionList.length > 0 ? root.regionList.length : root.regionsTotal
                return "sizes (" + n + " regions)"
            }
            default: return op
        }
    }

    // One-line summary of a finished task; the row elides on overflow.
    function taskSummary(op, r, t) {
        const res = r.result || {}
        switch (op) {
            case "host":
                return (res.region || "") + " · " + String(res.cid || "").substring(0, 10)
                     + (res.bytes !== undefined ? "… · " + root.fmtBytes(res.bytes) : "")
            case "sizes":
                return (res.count !== undefined ? res.count : "?") + " regions · "
                     + (res.total_bytes !== undefined ? root.fmtBytes(res.total_bytes) : "?")
            case "fetch": {
                const region = res.region || (t.args && t.args.region) || ""
                const cid = (t.args && t.args.cid) || ""
                return cid ? region + " · " + cid : region
            }
            case "lookup_bulk": {
                const reg = res.registered_count !== undefined ? res.registered_count : "?"
                const total = res.count !== undefined ? res.count : "?"
                const unreachable = res.error_count !== undefined ? res.error_count : 0
                return reg + "/" + total + " registered"
                     + (unreachable > 0 ? " · " + unreachable + " unreachable" : "")
            }
            default:
                return "ok"
        }
    }

    // A result landed: resolve the OLDEST running task whose core op matches
    // the routed op (r.op when the core tags results, else the local
    // pendingOp tag, normalized back to the core name).
    function resolveTaskResult(op, r) {
        // A job dispatch ack is not the task's outcome — the task resolves
        // from job_status when the background work finishes.
        if (r.ok && r.result && r.result.job_id) return
        let core = op
        if (core === "regions-list") core = "regions"
        else if (core === "catalog-list") core = "catalog"
        else if (core.indexOf("lookup:") === 0) core = "lookup"
        let idx = -1
        for (let i = 0; i < root.tasks.length; i++) {
            const t = root.tasks[i]
            if (t.state === "running" && t.op === core) { idx = i; break }
        }
        if (idx < 0) return
        const t = root.tasks[idx]
        if (r.ok) {
            root.updateTask(t.id, { state: "done", summary: root.taskSummary(core, r, t) })
        } else {
            root.updateTask(t.id, { state: "error", summary: r.error || "failed" })
        }
    }

    // ---- The sequential host loop -------------------------------------------
    // host_bulk reports nothing until every region finishes, so the Host §02
    // buttons compute their region list exactly as their labels do and fire
    // the single-region `host` op one at a time — strictly sequential (each
    // next fires from the previous result; the RemoteObjects transport
    // drops ops fired while another is in flight) — and every result ticks
    // the grouped task. The core's host_bulk op is untouched (the CLI uses
    // it); the loop IS the progress mechanism.
    function startHostRun(regions) {
        if (root.hostLoopActive || regions.length === 0) return
        root.hostQueue = regions.slice()
        root.hostTotal = regions.length
        root.hostDoneCount = 0
        root.hostBytes = 0
        root.hostCurrent = ""
        root.hostLoopActive = true
        root.hostTaskId = root.addTask("host-run",
            "host " + regions.length + (regions.length > 1 ? " regions" : " region"))
        root.hostNext()
    }

    function hostNext() {
        if (root.hostQueue.length === 0) return
        root.hostCurrent = root.hostQueue[0]
        root.hostQueue = root.hostQueue.slice(1)
        // Job mode: a real host runs minutes and its direct result never
        // survives the transport; the loop advances from job_status polls.
        root.call("host", { region: root.hostCurrent, job: true })
    }

    // ---- Job polling --------------------------------------------------------
    // One job_status per tick while polls are outstanding. The op is instant
    // server-side, but its result may be dropped by the transport: the
    // `lastResult` property sync silently no-ops on a byte-identical value
    // (see jobStatusInFlight above), and a frame colliding with another op in
    // flight can drop too. Either way the deadline re-arms the loop within a
    // few ticks instead of stalling forever.
    function pollJobs() {
        const ids = Object.keys(root.jobPolls)
        if (ids.length === 0 || !root.bk) return
        // Re-arm if a previous poll's result never landed (dropped/deduped).
        if (root.jobStatusInFlight && root.taskTick >= root.jobStatusPollDeadline)
            root.jobStatusInFlight = false
        if (root.jobStatusInFlight) return
        root.jobStatusInFlight = true
        // Re-arm after ~3 ticks even if nothing arrives — the job_status op
        // is sub-millisecond server-side, so 3 s is a generous drop timeout.
        root.jobStatusPollDeadline = root.taskTick + 3
        root.bk.invokeOp("job_status", JSON.stringify({ job_id: ids[0] }))
    }

    function runningTaskFor(coreOp) {
        for (let i = 0; i < root.tasks.length; i++)
            if (root.tasks[i].state === "running" && root.tasks[i].op === coreOp)
                return root.tasks[i]
        return null
    }

    // A job_status result: resolve the attached task when the job finished;
    // host jobs also advance the sequential loop.
    function onJobStatus(r) {
        root.jobStatusInFlight = false
        if (!r.ok || !r.result || !r.result.id) return
        const j = r.result
        const entry = root.jobPolls[j.id]
        if (!entry) return
        if (j.state === "running") return   // still working; next tick re-polls
        const next = Object.assign({}, root.jobPolls)
        delete next[j.id]
        root.jobPolls = next
        const task = root.tasks.find(x => x.id === entry.taskId)
        if (j.state === "done") {
            if (task) root.updateTask(task.id, {
                state: "done",
                summary: root.taskSummary(j.op, { ok: true, result: j.result }, task)
            })
            if (entry.host) root.onHostJobDone(j.result || {})
        } else {
            if (task) root.updateTask(task.id, { state: "error", summary: j.error || "failed" })
            if (entry.host) root.onHostJobError(j.error || "host failed")
        }
    }

    function onHostJobDone(res) {
        if (!root.hostLoopActive) return
        root.hostDoneCount++
        root.hostBytes += (res.bytes || 0)
        root.updateTask(root.hostTaskId, {
            summary: "hosting " + root.hostDoneCount + "/" + root.hostTotal
                   + " · " + (res.region || root.hostCurrent) + " ✓ · "
                   + root.fmtBytes(root.hostBytes) + " so far"
        })
        if (root.hostDoneCount >= root.hostTotal) {
            root.hostLoopActive = false
            root.updateTask(root.hostTaskId, {
                state: "done",
                summary: root.hostDoneCount + " hosted · " + root.fmtBytes(root.hostBytes)
            })
            // The run changed the catalog; refresh it for Consumer's "use
            // cached file". job_status just returned, so the dispatcher is
            // free and the call is safe.
            root.call("catalog", {}, "catalog-list")
        } else {
            root.hostNext()
        }
    }

    function onHostJobError(msg) {
        root.hostLoopActive = false
        root.updateTask(root.hostTaskId, {
            state: "error",
            summary: root.hostCurrent + ": " + msg
        })
    }

    // Clear the task panel: stop the host loop and all job polling, drop
    // completed rows. In-flight jobs keep running server-side (their bytes
    // land regardless); their results are simply not shown. Also unsticks
    // the incidental UI the dropped results would have reset.
    function clearTasks() {
        root.jobPolls = ({})
        root.jobStatusInFlight = false
        root.jobStatusPollDeadline = 0
        root.hostLoopActive = false
        root.hostQueue = []
        root.tasks = []
        if (root.sizesLoading) root.sizesLoading = false
        if (root.discoverState === "pending") root.discoverState = "ok"
    }

    // Runs from onOpResult for op "host" while a run is active; the
    // per-region task was already resolved by resolveTaskResult.
    function onHostResult(r) {
        if (!root.hostLoopActive) return
        if (r.ok && r.result) {
            root.hostDoneCount++
            root.hostBytes += (r.result.bytes || 0)
            root.updateTask(root.hostTaskId, {
                summary: "hosting " + root.hostDoneCount + "/" + root.hostTotal
                       + " · " + (r.result.region || root.hostCurrent) + " ✓ · "
                       + root.fmtBytes(root.hostBytes) + " so far"
            })
            if (root.hostDoneCount >= root.hostTotal) {
                root.hostLoopActive = false
                root.updateTask(root.hostTaskId, {
                    state: "done",
                    summary: root.hostDoneCount + " hosted · " + root.fmtBytes(root.hostBytes)
                })
                // The run just changed the catalog (the host op records each
                // hosted region): refresh it so "use cached file" in Consumer
                // §02 can offer what this run produced. Nothing else is in
                // flight (the last host result is this one), so the op is
                // safe to fire now.
                root.call("catalog", {}, "catalog-list")
            } else {
                root.hostNext()
            }
        } else {
            // A region failed: stop the run here. The per-region task row
            // carries the full error; the group carries region + message.
            root.hostLoopActive = false
            root.updateTask(root.hostTaskId, {
                state: "error",
                summary: root.hostCurrent + ": " + (r.error || "host failed")
            })
        }
    }

    // Route a completed op's result. Consoles per tab; the list-driving ops
    // (regions, lookup_bulk) also update the Regions tab's model, and the
    // fetch-region lookup drives the Consumer §01 auto-fill.
    //
    // Ops arrive tagged by the core with the CORE's op name (r.op in the
    // router), while locally-dispatched tags ("regions-list", "catalog-list")
    // still apply for cores that predate op tagging — every arm accepts
    // both. The startup chain is serialized (each op fired from the previous
    // result) because firing an op while another is still in flight can
    // silently drop it in the RemoteObjects transport. Order (v1.3.2):
    // open (seeds the picker from sizes/statuses cached in the state file) →
    // discover → regions → sizes (live refresh; fires catalog even on
    // error so the chain never strands) → catalog (the slow LEZ RPC,
    // last).
    function onOpResult(op, r) {
        root.resolveTaskResult(op, r)
        const consoleText = root.pretty(r)

        if (op === "open") {
            // The reachability dot tracks every open, ok or not.
            root.storageReach = r.ok ? "ok" : "fail"
            if (r.ok) {
                // Seed the picker from the sizes cached in the state file so
                // it renders before the live `sizes` refresh lands.
                if (r.result && r.result.region_sizes) root.regionSizes = r.result.region_sizes
                // Same for the on-chain statuses: the Regions tab's badges
                // render from the cache until the first Re-check.
                if (r.result && r.result.region_statuses) {
                    root.regionStatuses = r.result.region_statuses
                    root.rebuildRegionModel()
                }
                // The auto-open worked: fold Connect away (a manual click can
                // re-open it; a failed open leaves it expanded).
                if (secConnect) secConnect.expanded = false
                root.runDiscover()
            }
        } else if (op === "regions" || op === "regions-list") {
            if (r.ok && r.result && r.result.regions) {
                root.regionList = r.result.regions.map(g => ({
                    path: g.path, name: g.name, level: g.level }))
            }
            // Sizes here (not the catalog): the picker gets live numbers
            // before the slow LEZ catalog RPC runs at the end of the chain.
            root.sizesLoading = true
            root.call("sizes", {}, "sizes")
        } else if (op === "sizes") {
            if (r.ok && r.result && r.result.sizes) root.regionSizes = r.result.sizes
            root.sizesLoading = false
            // Catalog fires unconditionally (ok or not) so a failed sizes
            // op never strands the chain; if sizes is dropped in transport
            // the REGIONS chip refresh recovers the whole chain.
            root.call("catalog", {}, "catalog-list")
        } else if (op === "catalog" || op === "catalog-list") {
            // The core returns the catalog as a bare list under "result";
            // accept an object wrapper too in case that shape returns.
            const cat = Array.isArray(r.result)
                ? r.result
                : (r.result && r.result.catalog) || null
            if (r.ok && cat) {
                root.catalogRecords = cat
                root.hostedRegions = cat.map(c => c.region)
            } else {
                root.catalogRecords = []
                root.hostedRegions = []
            }
        } else if (op === "host") {
            if (r.ok && r.result && r.result.job_id) {
                // Job dispatch ack: attach the poll to the running task.
                const t = root.runningTaskFor("host")
                if (t) {
                    const next = Object.assign({}, root.jobPolls)
                    next[r.result.job_id] = { taskId: t.id, host: true }
                    root.jobPolls = next
                }
            } else {
                // Synchronous host result (old core, or a direct call).
                root.onHostResult(r)
            }
        } else if (op === "fetch") {
            if (r.ok && r.result && r.result.job_id) {
                const t = root.runningTaskFor("fetch")
                if (t) {
                    const next = Object.assign({}, root.jobPolls)
                    next[r.result.job_id] = { taskId: t.id, host: false }
                    root.jobPolls = next
                }
            }
        } else if (op === "job_status") {
            root.onJobStatus(r)
        } else if (op === "lookup_bulk") {
            if (r.ok && r.result && r.result.statuses) {
                // MERGE (mirrors the core's state-file merge): fetched
                // entries update, the rest survive.
                const merged = {}
                for (const k in root.regionStatuses) merged[k] = root.regionStatuses[k]
                for (const k in r.result.statuses) merged[k] = r.result.statuses[k]
                root.regionStatuses = merged
                root.rebuildRegionModel()
            }
        } else if (op === "lookup") {
            root.onFetchLookupResult(r)
        } else if (op === "discover") {
            if (r.ok && r.result) {
                root.regionsAvailable = r.result.available !== undefined ? r.result.available : 0
                root.regionsTotal = r.result.total_set !== undefined ? r.result.total_set : 72
                root.discoverState = "ok"
                root.discoverError = ""
            } else {
                root.discoverState = "error"
                root.discoverError = r.error || "discover failed"
            }
            if (r.ok) root.call("regions", {}, "regions-list")
        }
        // The 1 Hz job_status polls are bookkeeping, not results — keep
        // them out of the output consoles.
        if (op !== "job_status") {
            if (root.currentTab === 0)      out.outputText  = consoleText
            else if (root.currentTab === 1) outR.outputText = consoleText
            else                            out2.outputText = consoleText
        }

        if (op === "regions" || op === "regions-list") {
            // The badges join the frozen list with whatever statuses are
            // already in hand (possibly none on a cold start).
            root.rebuildRegionModel()
        }
    }

    // The Consumer §01 auto-fill: the lookup fired by setFetchRegion lands
    // here (correlated by region — results arrive tagged only with the core
    // op name). Fill CID + MD5 on success; explain why not otherwise; the
    // fields stay editable either way.
    function onFetchLookupResult(r) {
        if (root.fetchLookupRegion === "") return
        const want = root.fetchLookupRegion
        const resultRegion = (r.result && r.result.region) || ""
        const errorText = r.error || ""
        // A result for a different region (a Regions-tab row click that was
        // in flight) is not ours: leave the chain pending.
        if (resultRegion !== want && errorText.indexOf(want) < 0) return
        root.fetchLookupRegion = ""
        if (r.ok && r.result) {
            const latest = r.result.latest
            if (latest && latest.cid) {
                fCid.text = latest.cid
                fMd5.text = latest.checksum || ""
                root.fetchFillState = "ok"
            } else {
                root.fetchFillState = "unregistered"
            }
        } else if (errorText.indexOf("is not registered") >= 0) {
            root.fetchFillState = "unregistered"
        } else {
            // Registry unreachable: leave the fields to be typed by hand.
            root.fetchFillState = "unreachable"
        }
    }

    Connections {
        target: root.bk
        function onLastResultChanged() {
            if (!root.bk || !root.bk.lastResult.length) return
            let r
            try { r = JSON.parse(root.bk.lastResult) }
            catch (e) { r = { ok: false, error: root.bk.lastResult } }
            // Results arrive as property syncs, not return values, so the op
            // that produced a result cannot be inferred from call order when
            // several are in flight. The core tags every result with "op";
            // fall back to the pending tag for cores that predate that.
            root.onOpResult(r.op || root.pendingOp, r)
        }
    }

    // ONE shared 1 Hz timer drives every running task row's elapsed readout
    // (cheap: it bumps a counter the elapsed bindings depend on).
    Timer {
        interval: 1000
        repeat: true
        running: root.runningTasks > 0
        onTriggered: {
            root.taskTick++
            root.pollJobs()
        }
    }

    // ---- Logos identity tokens (from logos.co live CSS) ------------------
    readonly property color cCanvas:   "#152521"                            // brand dark green — the canvas
    readonly property color cInk:      "#f5f5ef"                            // brand off-white
    readonly property color cAccent:   "#ffd328"                            // brand yellow — the one accent
    readonly property color cTan:      "#e2e0c9"                            // accent tan — secondary surfaces
    readonly property color cTeal:     "#5f797c"                            // steel teal — muted accents
    readonly property color cInfo:     "#c6ebf7"                            // light blue — info, links
    readonly property color cBrown:    "#a18863"                            // brown — "not hosted" status
    // Validation/reachability tones, kept quiet on the dark-green canvas:
    // amber for "fixable wrong", soft green for confirmed, soft red for
    // unreachable. Derived from the same saturation band as cTeal/cBrown.
    readonly property color cWarn:     "#c9a23f"                            // amber — invalid input, registry unreachable
    readonly property color cGood:     "#8fb573"                            // soft green — valid input, storage reachable
    readonly property color cBad:      "#c0654e"                            // soft red — storage unreachable

    // Derived translucent overlays of the same tones.
    readonly property color cPanel:    Qt.rgba(0.886, 0.878, 0.788, 0.05)   // tan at 5%  — panels
    readonly property color cPanelHi:  Qt.rgba(0.886, 0.878, 0.788, 0.09)   // tan at 9%  — hover fills
    readonly property color cHair:     Qt.rgba(0.961, 0.961, 0.941, 0.14)   // hairline
    readonly property color cHairSoft: Qt.rgba(0.961, 0.961, 0.941, 0.07)   // softer hairline
    readonly property color cMuted:    Qt.rgba(0.961, 0.961, 0.941, 0.55)   // muted ink
    readonly property color cFaint:    Qt.rgba(0.961, 0.961, 0.941, 0.34)   // faint ink
    readonly property color cConsole:  "#0b1512"                            // darker green — output console

    // ---- Typography -------------------------------------------------------
    // Public Sans (UI text), Fira Mono (machine values: hex ids, CIDs, MD5s,
    // URLs, JSON), and a serif for headings — the site's own rhythm. The
    // faces load from the app's own qml dir; if they cannot be resolved the
    // system equivalents take over.
    FontLoader { id: sansLoader; source: "fonts/PublicSans.ttf" }
    FontLoader { id: monoLoader; source: "fonts/FiraMono.ttf" }
    readonly property string fSans:  sansLoader.status === FontLoader.Ready ? sansLoader.name : "DejaVu Sans"
    readonly property string fMono:  monoLoader.status === FontLoader.Ready ? monoLoader.name : "DejaVu Sans Mono"
    readonly property string fSerif: "Liberation Serif"   // Georgia / Times New Roman stand-in

    // =====================================================================
    //  Inline components
    // =====================================================================

    component TabBtn: Item {
        id: btn
        property int tabIndex: 0
        property string label: ""
        readonly property bool active: root.currentTab === btn.tabIndex
        implicitWidth: tabLabel.implicitWidth + 8
        implicitHeight: 46
        Text {
            id: tabLabel
            anchors.horizontalCenter: parent.horizontalCenter
            y: 14
            text: btn.label
            font.family: root.fSans
            font.pixelSize: 14
            font.weight: btn.active ? Font.DemiBold : Font.Medium
            color: btn.active ? root.cAccent
                 : (tabMouse.containsMouse ? Qt.rgba(0.961, 0.961, 0.941, 0.92)
                                           : Qt.rgba(0.961, 0.961, 0.941, 0.60))
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
        Rectangle {
            anchors.bottom: parent.bottom
            anchors.horizontalCenter: parent.horizontalCenter
            width: tabLabel.implicitWidth + 4
            height: 2
            radius: 1
            color: root.cAccent
            opacity: btn.active ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
        MouseArea {
            id: tabMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                root.currentTab = btn.tabIndex
                // Opening the Regions tab kicks off the registry check when
                // the session has no statuses yet.
                if (btn.tabIndex === 1) root.maybeCheckStatuses()
            }
        }
    }

    // A section heading that toggles: the whole bar is clickable, the
    // chevron rotates ▸→▾, and the Section below animates its body. The
    // click handler lives in Section (the head just emits the signal).
    component SectionHead: Item {
        id: head
        property string numeral: ""
        property string heading: ""
        property bool expanded: true
        signal clicked()
        implicitWidth: headRow.implicitWidth
        implicitHeight: headRow.implicitHeight
        Layout.fillWidth: true
        RowLayout {
            id: headRow
            anchors.left: parent.left
            anchors.right: parent.right
            spacing: 12
            Text {
                text: "▸"
                font.family: root.fMono
                font.pixelSize: 12
                color: root.cTeal
                Layout.alignment: Qt.AlignBottom
                Layout.bottomMargin: 3
                rotation: head.expanded ? 90 : 0
                Behavior on rotation {
                    NumberAnimation { duration: 180; easing.type: Easing.InOutQuad }
                }
                transformOrigin: Item.Center
            }
            Text {
                text: head.numeral
                font.family: root.fMono
                font.pixelSize: 11
                color: root.cTeal
                Layout.alignment: Qt.AlignBottom
                Layout.bottomMargin: 4
            }
            Text {
                text: head.heading
                font.family: root.fSerif
                font.pixelSize: 19
                font.letterSpacing: -0.2
                color: root.cInk
            }
            Rectangle {
                Layout.fillWidth: true
                Layout.leftMargin: 4
                height: 1
                color: root.cHair
            }
        }
        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: head.clicked()
        }
    }

    // A collapsible section: the head (toggle) plus a body whose height
    // animates between 0 and its content height. Expansion state is plain
    // property state — manual toggles persist for the session.
    component Section: ColumnLayout {
        id: sec
        property string numeral: ""
        property string heading: ""
        property bool expanded: true
        default property alias bodyData: body.data
        spacing: 12
        SectionHead {
            numeral: sec.numeral
            heading: sec.heading
            expanded: sec.expanded
            onClicked: sec.expanded = !sec.expanded
        }
        Item {
            id: bodyClip
            Layout.fillWidth: true
            implicitHeight: sec.expanded ? body.implicitHeight : 0
            visible: implicitHeight > 0
            clip: true
            Behavior on implicitHeight {
                NumberAnimation { duration: 180; easing.type: Easing.InOutQuad }
            }
            ColumnLayout {
                id: body
                anchors.left: parent.left
                anchors.right: parent.right
                height: implicitHeight
                spacing: 12
            }
        }
    }

    component Hint: Text {
        font.family: root.fSans
        font.pixelSize: 12
        lineHeight: 1.5
        lineHeightMode: Text.ProportionalHeight
        wrapMode: Text.Wrap
        color: root.cMuted
        Layout.fillWidth: true
    }

    // A labeled slot that hosts one input field. The field is supplied as
    // the default child. Optional extras: a caption line under the field
    // (dynamic — an empty string hides it), a reachability dot after the
    // label, and an ⓘ tooltip on the label.
    component FieldCell: ColumnLayout {
        id: cell
        property string label: ""
        property string caption: ""          // "" hides the caption line
        property color captionColor: root.cMuted
        property string dot: ""              // "ok" | "down" | "" — after the label
        property string info: ""             // ⓘ tooltip text on the label
        default property alias fieldData: slot.data
        Layout.fillWidth: true
        Layout.preferredWidth: 1     // equal-weight columns in grids/rows
        spacing: 6
        RowLayout {
            spacing: 6
            Text {
                text: cell.label
                font.family: root.fSans
                font.pixelSize: 11
                font.letterSpacing: 1.0
                font.weight: Font.Medium
                color: root.cMuted
            }
            Rectangle {
                visible: cell.dot.length > 0
                width: 6; height: 6; radius: 3
                Layout.alignment: Qt.AlignVCenter
                color: cell.dot === "ok" ? root.cGood : root.cBad
                MouseArea {
                    id: dotMouse
                    anchors.fill: parent
                    hoverEnabled: true
                }
                ToolTip.visible: dotMouse.containsMouse
                ToolTip.text: cell.dot === "ok" ? "Storage reachable" : "Storage unreachable"
                ToolTip.delay: 200
            }
            Text {
                visible: cell.info.length > 0
                text: "ⓘ"
                font.family: root.fSans
                font.pixelSize: 11
                color: root.cMuted
                Layout.alignment: Qt.AlignVCenter
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.WhatsThisCursor
                    ToolTip.visible: containsMouse
                    ToolTip.text: cell.info
                    ToolTip.delay: 200
                }
            }
            Item { Layout.fillWidth: true }
        }
        ColumnLayout { id: slot; Layout.fillWidth: true; spacing: 0 }
        Text {
            visible: cell.caption.length > 0
            Layout.fillWidth: true
            Layout.topMargin: -2
            text: cell.caption
            font.family: root.fSans
            font.pixelSize: 11
            lineHeight: 1.4
            lineHeightMode: Text.ProportionalHeight
            wrapMode: Text.Wrap
            color: cell.captionColor
        }
    }

    // Every machine value (URL, hex id, path, region slug) is mono. Optional
    // validation states color the border: "invalid" amber, "valid" a quiet
    // green — otherwise the focus/hover hairline logic applies.
    component MonoField: TextField {
        id: control
        property string validation: "none"   // none | valid | invalid
        Layout.fillWidth: true
        Layout.alignment: Qt.AlignBottom
        font.family: root.fMono
        font.pixelSize: 12
        color: root.cInk
        placeholderTextColor: Qt.rgba(0.961, 0.961, 0.941, 0.50)
        selectionColor: Qt.rgba(1.0, 0.827, 0.157, 0.85)
        selectedTextColor: "#152521"
        leftPadding: 10
        rightPadding: 10
        topPadding: 8
        bottomPadding: 8
        background: Rectangle {
            implicitHeight: 34
            radius: 4
            color: Qt.rgba(0, 0, 0, 0.26)
            border.width: 1
            border.color: control.validation === "invalid" ? Qt.rgba(0.788, 0.635, 0.247, 0.85)
                        : control.validation === "valid" ? Qt.rgba(0.561, 0.710, 0.451, 0.55)
                        : (control.activeFocus ? Qt.rgba(0.776, 0.922, 0.969, 0.55)
                        : (control.hovered ? Qt.rgba(0.961, 0.961, 0.941, 0.26)
                                           : Qt.rgba(0.961, 0.961, 0.941, 0.14)))
            Behavior on border.color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
    }

    // Primary action — the yellow accent, one per section.
    component PButton: Button {
        id: control
        leftPadding: 18
        rightPadding: 18
        topPadding: 8
        bottomPadding: 8
        implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
        implicitHeight: 34
        contentItem: Text {
            text: control.text
            font.family: root.fSans
            font.pixelSize: 13
            font.weight: Font.DemiBold
            color: "#152521"
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }
        background: Rectangle {
            radius: 6
            color: control.pressed ? "#e0bd20"
                 : (control.hovered ? "#ffdc52" : root.cAccent)
            border.width: 1
            border.color: Qt.rgba(0, 0, 0, 0.18)
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
    }

    // Secondary action — steel teal outline on the canvas.
    component SButton: Button {
        id: control
        leftPadding: 16
        rightPadding: 16
        topPadding: 8
        bottomPadding: 8
        implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
        implicitHeight: 34
        contentItem: Text {
            text: control.text
            font.family: root.fSans
            font.pixelSize: 13
            font.weight: Font.Medium
            color: control.hovered ? root.cTan : Qt.rgba(0.886, 0.878, 0.788, 0.86)
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
        background: Rectangle {
            radius: 6
            color: control.pressed ? Qt.rgba(0.373, 0.475, 0.486, 0.28)
                 : (control.hovered ? Qt.rgba(0.373, 0.475, 0.486, 0.18) : "transparent")
            border.width: 1
            border.color: Qt.rgba(0.373, 0.475, 0.486, 0.85)
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
    }

    // The region-picker popup every picker shares (Host §02 is multi-select;
    // the three Consumer pickers are single-select). Search + the frozen set
    // with live sizes. Selection state lives with the caller — the popup
    // only reports clicks through `picked`; single-select mode closes on
    // the pick.
    component RegionPickPopup: Popup {
        id: rpp
        // Paths currently selected (multi mode; the glyph shows ×).
        property var selection: []
        // The selected path (single mode; the glyph shows ×, click closes).
        property string single: ""
        property bool singleSelect: false
        signal picked(string path)
        x: 0
        width: 520
        height: 400
        padding: 1
        // Placement: above the button when it fits, else below with a height
        // clamped to the window — on a 1280x800 screen the Host §02 button
        // sits ~315 px down, so a fixed 400 px popup hanging upwards had its
        // filter field (and the first rows) clipped above the screen top.
        // `anchorItem` is the button that opens the popup.
        property Item anchorItem: null
        function place() {
            // The popup's parent is the anchor button, so y is relative to it:
            // above = -(height + gap), below = +height + gap. Only the
            // button's absolute position decides whether "above" fits.
            const gap = 8
            const btn = rpp.anchorItem
            const winH = root.height > 0 ? root.height : 800
            const bottomAbs = btn ? btn.mapToItem(null, 0, btn.height).y
                                  : rpp.parent.height
            const above = bottomAbs - gap
            const below = winH - bottomAbs - gap
            const want = 400
            if (above >= want) {
                rpp.height = want
                rpp.y = -(want + gap)
            } else if (below >= Math.min(want, above)) {
                rpp.height = Math.max(180, Math.min(want, below))
                rpp.y = gap
            } else {
                rpp.height = Math.max(180, Math.min(want, above))
                rpp.y = -(rpp.height + gap)
            }
        }
        onOpened: rpp.place()
        background: Rectangle {
            radius: 8
            color: root.cConsole
            border.width: 1
            border.color: root.cHair
        }
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 10
            spacing: 8
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                TextField {
                    id: pickSearch
                    Layout.fillWidth: true
                    placeholderText: "filter…"
                    font.family: root.fMono
                    font.pixelSize: 11
                    color: root.cInk
                    background: Rectangle {
                        radius: 4
                        color: Qt.rgba(0,0,0,0.25)
                        border.width: 1
                        border.color: root.cHair
                    }
                    onTextChanged: pickModel.update()
                }
                Text {
                    text: root.sizesLoading ? "sizes…" : root.regionList.length + " regions"
                    font.family: root.fSans
                    font.pixelSize: 10
                    color: root.cMuted
                }
            }
            ListView {
                id: pickList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                ScrollBar.vertical: ConsoleScrollBar { }
                model: ListModel {
                    id: pickModel
                    function update() {
                        clear()
                        const needle = (pickSearch.text || "").toLowerCase()
                        for (const g of root.regionList) {
                            if (needle.length === 0
                                || g.path.toLowerCase().indexOf(needle) >= 0
                                || (g.name||"").toLowerCase().indexOf(needle) >= 0) {
                                append({ path: g.path, name: g.name, level: g.level })
                            }
                        }
                    }
                }
                delegate: Rectangle {
                    width: pickList.width
                    height: 34
                    color: pickMouse.containsMouse ? root.cPanelHi : "transparent"
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        spacing: 8
                        Text {
                            text: rpp.isSelected(model.path) ? "×" : "·"
                            color: rpp.isSelected(model.path) ? root.cAccent : root.cMuted
                            font.pixelSize: 13
                        }
                        Text {
                            text: model.path
                            font.family: root.fMono
                            font.pixelSize: 11
                            color: root.cInk
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }
                        Text {
                            text: root.regionSizes[model.path] !== undefined
                                  ? root.fmtBytes(root.regionSizes[model.path]) : ""
                            font.family: root.fMono
                            font.pixelSize: 10
                            color: root.cMuted
                        }
                    }
                    MouseArea {
                        id: pickMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            rpp.picked(model.path)
                            // Single-select: choosing IS the whole answer —
                            // close the popup.
                            if (rpp.singleSelect) rpp.close()
                        }
                    }
                }
                Connections {
                    target: root
                    function onRegionListChanged() { pickModel.update() }
                    function onRegionSizesChanged() { pickModel.update() }
                }
                Component.onCompleted: pickModel.update()
            }
        }
        function isSelected(path) {
            if (rpp.singleSelect) return path === rpp.single
            return rpp.selection.indexOf(path) >= 0
        }
    }

    // Output console — mono values on a deeper green.
    component OutputBox: Rectangle {
        id: box
        property alias outputText: ta.text
        radius: 8
        color: root.cConsole
        border.width: 1
        border.color: root.cHair
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 12
            anchors.topMargin: 10
            spacing: 8
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Rectangle { width: 6; height: 6; radius: 3; color: root.cTeal }
                Text {
                    text: "OUTPUT"
                    font.family: root.fSans
                    font.pixelSize: 11
                    font.letterSpacing: 1.4
                    font.weight: Font.Medium
                    color: root.cMuted
                }
                Item { Layout.fillWidth: true }
            }
            // The output needs a real scroll view: the hosting page is a
            // Flickable that eats wheel events over plain TextAreas, so the
            // box gets a ScrollView that takes the wheel itself plus the
            // draggable console bar.
            ScrollView {
                id: outScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                ScrollBar.vertical: ConsoleScrollBar { }
                TextArea {
                    id: ta
                    width: outScroll.availableWidth
                    readOnly: true
                    wrapMode: TextArea.Wrap
                    font.family: root.fMono
                    font.pixelSize: 12
                    color: root.cTan
                    selectionColor: Qt.rgba(0.961, 0.961, 0.941, 0.22)
                    selectedTextColor: root.cTan
                    background: Item { }
                }
            }
        }
        Text {
            visible: ta.text.length === 0
            anchors.fill: parent
            anchors.margins: 14
            anchors.topMargin: 36
            text: "No output yet — run an operation above."
            font.family: root.fSans
            font.pixelSize: 12
            color: root.cFaint
            wrapMode: Text.Wrap
        }
    }

    component ConsoleScrollBar: ScrollBar {
        implicitWidth: 10
        background: Item { }
        contentItem: Rectangle {
            radius: 4
            color: parent.pressed ? Qt.rgba(0.961, 0.961, 0.941, 0.34)
                 : (parent.hovered ? Qt.rgba(0.961, 0.961, 0.941, 0.22)
                                   : Qt.rgba(0.961, 0.961, 0.941, 0.14))
            Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
        }
    }

    // =====================================================================
    //  Page
    // =====================================================================

    Rectangle {
        anchors.fill: parent
        color: root.cCanvas
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 32
        anchors.rightMargin: 32
        anchors.topMargin: 20
        anchors.bottomMargin: 16
        spacing: 16

        // ---- Header ----
        RowLayout {
            Layout.fillWidth: true
            spacing: 16
            Rectangle {
                Layout.alignment: Qt.AlignTop
                Layout.topMargin: 7
                width: 3
                height: 34
                radius: 1
                color: root.cAccent
            }
            ColumnLayout {
                spacing: 3
                Text {
                    text: "OpenStreetMap on Logos"
                    font.family: root.fSerif
                    font.pixelSize: 30
                    font.letterSpacing: -0.3
                    color: root.cInk
                }
                Text {
                    text: "Regional OSM extracts on Logos Storage, registered on-chain."
                    font.family: root.fSans
                    font.pixelSize: 12
                    color: root.cMuted
                }
            }
            Item { Layout.fillWidth: true }
            Rectangle {
                Layout.alignment: Qt.AlignTop
                Layout.topMargin: 5
                radius: 4
                color: Qt.rgba(0.776, 0.922, 0.969, 0.06)
                border.width: 1
                border.color: Qt.rgba(0.776, 0.922, 0.969, 0.30)
                implicitHeight: coreRow.implicitHeight + 14
                implicitWidth: coreRow.implicitWidth + 24
                RowLayout {
                    id: coreRow
                    anchors.centerIn: parent
                    spacing: 10
                    Text {
                        text: "CORE"
                        font.family: root.fSans
                        font.pixelSize: 11
                        font.letterSpacing: 1.4
                        font.weight: Font.Medium
                        color: Qt.rgba(0.776, 0.922, 0.969, 0.75)
                    }
                    Text {
                        // Compact: the version from the core's version JSON,
                        // not the raw JSON (hover shows the full payload).
                        text: {
                            const v = root.coreVersion()
                            return v.length ? "✓ v" + v : "—"
                        }
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.cTan
                        elide: Text.ElideRight
                        Layout.maximumWidth: 380
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.runDiscover()
                    ToolTip.visible: containsMouse
                    ToolTip.text: root.osmVersion.length
                                   ? root.osmVersion
                                   : "core not loaded — click to re-run the Geofabrik cross-check"
                    ToolTip.delay: 300
                    hoverEnabled: true
                }
            }

            // ---- Regions availability chip (auto-discover; click to refresh) ----
            Rectangle {
                id: regionsChip
                objectName: "regionsChip"
                Layout.alignment: Qt.AlignTop
                Layout.topMargin: 5
                radius: 4
                color: Qt.rgba(0.961, 0.890, 0.788, 0.06)
                border.width: 1
                border.color: Qt.rgba(0.961, 0.890, 0.788, 0.30)
                implicitHeight: regionsChipRow.implicitHeight + 14
                implicitWidth: regionsChipRow.implicitWidth + 24
                RowLayout {
                    id: regionsChipRow
                    anchors.centerIn: parent
                    spacing: 10
                    Text {
                        text: "REGIONS"
                        font.family: root.fSans
                        font.pixelSize: 11
                        font.letterSpacing: 1.4
                        font.weight: Font.Medium
                        color: Qt.rgba(0.961, 0.890, 0.788, 0.75)
                    }
                    Text {
                        text: {
                            if (root.discoverState === "pending") return "checking…"
                            if (root.discoverState === "error") return "unavailable"
                            return root.regionsAvailable + "/" + root.regionsTotal
                        }
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.discoverState === "error" ? root.cBrown : root.cTan
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.runDiscover()
                    ToolTip.visible: containsMouse
                    ToolTip.text: root.discoverState === "error" ? root.discoverError : "Live Geofabrik cross-check — click to refresh"
                    hoverEnabled: true
                }
            }
        }

        // ---- Ongoing tasks chip (opens the tasks panel) ----
        // Sits below the CORE/REGIONS row, right-aligned, so it never
        // overlaps them when closed; the panel opens upward from the chip
        // (never over the tab bar below).
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            Rectangle {
                id: tasksChip
                objectName: "tasksChip"
                Layout.alignment: Qt.AlignTop
                radius: 4
                color: Qt.rgba(0.961, 0.827, 0.157, 0.05)
                border.width: 1
                border.color: Qt.rgba(0.961, 0.827, 0.157, 0.30)
                implicitHeight: tasksChipRow.implicitHeight + 14
                implicitWidth: tasksChipRow.implicitWidth + 24
                RowLayout {
                    id: tasksChipRow
                    anchors.centerIn: parent
                    spacing: 10
                    Text {
                        text: "TASKS"
                        font.family: root.fSans
                        font.pixelSize: 11
                        font.letterSpacing: 1.4
                        font.weight: Font.Medium
                        color: Qt.rgba(0.961, 0.827, 0.157, 0.75)
                    }
                    Text {
                        text: "◐"
                        font.pixelSize: 12
                        color: root.runningTasks > 0 ? root.cAccent : root.cMuted
                    }
                    Text {
                        text: root.tasksChipText
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.tasksErrorCount > 0 ? root.cBrown : root.cTan
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: tasksPanel.opened ? tasksPanel.close() : tasksPanel.open()
                    ToolTip.visible: containsMouse
                    ToolTip.text: "Ongoing tasks — every op the app fires, with its result"
                    hoverEnabled: true
                }
                Popup {
                    id: tasksPanel
                    x: tasksChip.width - width
                    // Open DOWNWARD: the chip lives in the header near the
                    // top of the window, so the picker's upward placement
                    // would fly off-screen.
                    y: tasksChip.height + 8
                    width: 440
                    height: 380
                    padding: 1
                    background: Rectangle {
                        radius: 8
                        color: root.cConsole
                        border.width: 1
                        border.color: root.cHair
                    }
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 8
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8
                            Rectangle { width: 6; height: 6; radius: 3; color: root.cTeal }
                            Text {
                                text: "TASKS"
                                font.family: root.fSans
                                font.pixelSize: 11
                                font.letterSpacing: 1.4
                                font.weight: Font.Medium
                                color: root.cMuted
                            }
                            Item { Layout.fillWidth: true }
                            SButton {
                                // Stops the host loop and job polling, drops
                                // completed rows. In-flight jobs keep running
                                // server-side; their results are not shown.
                                text: "Clear"
                                font.pixelSize: 9
                                topPadding: 2; bottomPadding: 2
                                leftPadding: 8; rightPadding: 8
                                onClicked: root.clearTasks()
                                ToolTip.visible: hovered
                                ToolTip.text: "Stop incomplete tasks and clear the list"
                            }
                            Text {
                                text: root.tasks.length + " tracked"
                                font.family: root.fSans
                                font.pixelSize: 10
                                color: root.cMuted
                            }
                        }
                        Item {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            ListView {
                                id: taskList
                                anchors.fill: parent
                                clip: true
                                ScrollBar.vertical: ConsoleScrollBar { }
                                // newest first, so the running rows stay on top
                                model: root.tasks.slice().reverse()
                                delegate: Rectangle {
                                    width: taskList.width
                                    height: modelData.summary.length > 0 ? 52 : 36
                                    color: taskMouse.containsMouse ? root.cPanelHi : "transparent"
                                    ColumnLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: 10
                                        anchors.rightMargin: 10
                                        anchors.topMargin: 7
                                        anchors.bottomMargin: 7
                                        spacing: 2
                                        RowLayout {
                                            Layout.fillWidth: true
                                            spacing: 8
                                            Text {
                                                text: modelData.state === "running" ? "◐"
                                                     : (modelData.state === "error" ? "✕" : "✓")
                                                font.pixelSize: 12
                                                color: modelData.state === "running" ? root.cAccent
                                                     : (modelData.state === "error" ? root.cBrown : root.cTeal)
                                            }
                                            Text {
                                                text: modelData.label
                                                font.family: root.fSans
                                                font.pixelSize: 12
                                                color: modelData.state === "error" ? root.cTan : root.cInk
                                                Layout.fillWidth: true
                                                elide: Text.ElideRight
                                            }
                                            Text {
                                                // Running rows tick: the shared 1s
                                                // timer bumps taskTick, which this
                                                // binding depends on.
                                                text: {
                                                    if (modelData.state !== "running") return ""
                                                    root.taskTick
                                                    return root.fmtElapsed(modelData.startedAt)
                                                }
                                                font.family: root.fMono
                                                font.pixelSize: 11
                                                color: root.cMuted
                                            }
                                        }
                                        Text {
                                            visible: modelData.summary.length > 0
                                            text: modelData.summary
                                            font.family: root.fMono
                                            font.pixelSize: 11
                                            color: modelData.state === "error" ? root.cBrown : root.cMuted
                                            Layout.fillWidth: true
                                            elide: Text.ElideRight
                                        }
                                    }
                                    Rectangle {
                                        anchors.bottom: parent.bottom
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        height: 1
                                        color: root.cHairSoft
                                        visible: index < taskList.count - 1
                                    }
                                    MouseArea {
                                        id: taskMouse
                                        anchors.fill: parent
                                        hoverEnabled: true
                                    }
                                }
                            }
                            Text {
                                visible: root.tasks.length === 0
                                anchors.centerIn: parent
                                text: "No tasks yet — every op the app fires\nshows up here with its result."
                                horizontalAlignment: Text.AlignHCenter
                                font.family: root.fSans
                                font.pixelSize: 12
                                color: root.cFaint
                            }
                        }
                    }
                }
            }
        }

        // ---- Module error banner ----
        Rectangle {
            visible: root.lastErr.length > 0
            Layout.fillWidth: true
            implicitHeight: errCol.implicitHeight + 20
            radius: 6
            color: Qt.rgba(0.631, 0.525, 0.388, 0.10)
            border.width: 1
            border.color: Qt.rgba(0.631, 0.525, 0.388, 0.35)
            ColumnLayout {
                id: errCol
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 10
                spacing: 4
                Text {
                    text: "MODULE ERROR"
                    font.family: root.fSans
                    font.pixelSize: 11
                    font.letterSpacing: 1.4
                    font.weight: Font.Medium
                    color: root.cBrown
                }
                Text {
                    Layout.fillWidth: true
                    text: root.lastErr
                    wrapMode: Text.Wrap
                    font.family: root.fMono
                    font.pixelSize: 11
                    color: root.cTan
                }
            }
        }

        // ---- Tabs ----
        Item {
            Layout.fillWidth: true
            implicitHeight: 46
            RowLayout {
                anchors.top: parent.top
                anchors.left: parent.left
                spacing: 8
                TabBtn { tabIndex: 0; label: "Host / Registrar" }
                TabBtn { tabIndex: 1; label: "Regions" }
                TabBtn { tabIndex: 2; label: "Consumer" }
                TabBtn { tabIndex: 3; label: "Guide" }
            }
            Rectangle {
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                height: 1
                color: root.cHair
            }
        }

        // Tab pages. Anchored to fill (StackLayout/ScrollView sizing is not
        // reliable here); only the visible page is laid out.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            // ============ HOST / REGISTRAR ============
            Flickable {
                id: hostFlick
                anchors.fill: parent
                visible: root.currentTab === 0
                clip: true
                contentWidth: width
                contentHeight: hostCol.implicitHeight
                ScrollBar.vertical: ConsoleScrollBar { }

                ColumnLayout {
                    id: hostCol
                    // kept clear of the overlay scrollbar
                    width: hostFlick.width - 24
                    spacing: 24

                    // -- Open (config) --
                    // Auto-collapses once the auto-open succeeds (see the
                    // open arm in onOpResult); a failed open leaves it open.
                    Section {
                        id: secConnect
                        objectName: "secConnect"
                        numeral: "01"
                        heading: "Connect"
                        Hint {
                            text: "Open wires the host: Logos Storage (Codex) for blobs, Geofabrik as the source, and a local cache directory."
                        }
                        GridLayout {
                            columns: 2
                            Layout.fillWidth: true
                            columnSpacing: 16
                            rowSpacing: 12
                            FieldCell {
                                label: "STORAGE (CODEX) URL"
                                // The dot is the open result: green = the
                                // storage node answered, red = it did not.
                                dot: root.storageReach === "ok" ? "ok"
                                   : (root.storageReach === "fail" ? "down" : "")
                                caption: (storageUrl.text.length === 0
                                          || storageUrl.text === "http://127.0.0.1:8080")
                                         ? "Your local Logos Storage node — leave as-is unless you're pointing at a remote one."
                                         : ""
                                MonoField { id: storageUrl; text: "http://127.0.0.1:8080" }
                            }
                            FieldCell {
                                label: "GEOFABRIK BASE URL"
                                caption: (geofabrikUrl.text.length === 0
                                          || geofabrikUrl.text === "https://download.geofabrik.de")
                                         ? "Where map data downloads from. The default is the public source."
                                         : ""
                                MonoField { id: geofabrikUrl; text: "https://download.geofabrik.de" }
                            }
                            FieldCell {
                                label: "CACHE DIR"
                                caption: (cacheDir.text.length === 0
                                          || cacheDir.text === "osm-cache")
                                         ? "Where downloaded map files are kept on disk."
                                         : ""
                                MonoField { id: cacheDir; text: "osm-cache" }
                            }
                            FieldCell {
                                label: "REGISTRAR (HEX, 32B)"
                                info: "A registrar is the public ID of the account that files registrations — anyone can be one. The app never sees private keys: it builds the transaction, your wallet signs it."
                                caption: {
                                    const t = registrar.text.trim()
                                    if (t.length === 0)
                                        return "The wallet account that signs your registrations — paste your account ID (64 hex characters). Find it in your wallet app under account details."
                                    return /^[0-9a-fA-F]{64}$/.test(t)
                                           ? "✓ 64 hex characters"
                                           : "needs 64 hex characters"
                                }
                                captionColor: {
                                    const t = registrar.text.trim()
                                    if (t.length === 0) return root.cMuted
                                    return /^[0-9a-fA-F]{64}$/.test(t) ? root.cGood : root.cWarn
                                }
                                MonoField {
                                    id: registrar
                                    placeholderText: "registrar account id"
                                    validation: {
                                        const t = registrar.text.trim()
                                        if (t.length === 0) return "none"
                                        return /^[0-9a-fA-F]{64}$/.test(t) ? "valid" : "invalid"
                                    }
                                }
                            }
                        }
                        RowLayout {
                            spacing: 12
                            SButton {
                                text: "List regions"
                                onClicked: root.call("regions")
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }

                    // -- Host --
                    Section {
                        id: secHost
                        numeral: "02"
                        heading: "Host (download → verify MD5 → Logos Storage)"

                    // Selected regions as pinned chips with × to cancel.
                    Flow {
                        id: selectedChips
                        Layout.fillWidth: true
                        spacing: 6
                        visible: root.selectedRegions.length > 0
                        Repeater {
                            model: root.selectedRegions
                            delegate: Rectangle {
                                radius: 4
                                color: Qt.rgba(0.961, 0.961, 0.941, 0.08)
                                border.width: 1
                                border.color: root.cHair
                                width: chipRow.implicitWidth + 16
                                height: 26
                                RowLayout {
                                    id: chipRow
                                    anchors.centerIn: parent
                                    spacing: 6
                                    Text {
                                        text: modelData
                                        font.family: root.fMono
                                        font.pixelSize: 11
                                        color: root.cTan
                                    }
                                    Text {
                                        text: "×"
                                        font.family: root.fSans
                                        font.pixelSize: 13
                                        color: root.cMuted
                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: root.toggleRegion(modelData)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12

                        // The picker button + the shared region popup.
                        SButton {
                            id: hostPickBtn
                            Layout.alignment: Qt.AlignBottom
                            text: (root.selectedRegions.length === 0)
                                  ? (root.sizesLoading ? "Select regions  (sizes loading…)" : "Select regions ▾")
                                  : (root.selectedRegions.length + " selected ▾")
                            onClicked: hostPicker.open()
                            RegionPickPopup {
                                id: hostPicker
                                anchorItem: hostPickBtn
                                selection: root.selectedRegions
                                onPicked: function(path) { root.toggleRegion(path) }
                            }
                        }
                        Item { Layout.fillWidth: true }
                    }

                    // Actions + cumulative estimates. Each fires the
                    // sequential host loop (single-region `host` ops, one at
                    // a time — see startHostRun) with exactly the region list
                    // its label computes.
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        PButton {
                            Layout.alignment: Qt.AlignBottom
                            enabled: root.selectedRegions.length > 0 && !root.hostLoopActive
                            opacity: enabled ? 1.0 : 0.4
                            text: {
                                const sel = root.selectedRegions
                                const bytes = root.bytesOf(sel)
                                return "Host selected  (" + sel.length + " · " +
                                       (bytes ? root.fmtBytes(bytes) : "—") + ")"
                            }
                            onClicked: root.startHostRun(root.selectedRegions)
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            enabled: root.selectedRegions.length > 0 && !root.hostLoopActive
                            opacity: enabled ? 1.0 : 0.4
                            text: {
                                const all = root.regionList.map(g => g.path)
                                const keep = all.filter(p => root.selectedRegions.indexOf(p) < 0)
                                const bytes = root.bytesOf(keep)
                                return "Host all except selected  (" + keep.length +
                                       (bytes ? " · " + root.fmtBytes(bytes) : "") + ")"
                            }
                            onClicked: root.startHostRun(
                                root.regionList.map(g => g.path)
                                    .filter(p => root.selectedRegions.indexOf(p) < 0))
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            enabled: !root.hostLoopActive
                            opacity: enabled ? 1.0 : 0.4
                            text: {
                                const all = root.regionList.map(g => g.path)
                                const todo = all.filter(p => root.hostedRegions.indexOf(p) < 0)
                                const bytes = root.bytesOf(todo)
                                return "Host all  (" + todo.length + " new · " +
                                       (bytes ? root.fmtBytes(bytes) : "—") + ")"
                            }
                            onClicked: root.startHostRun(
                                root.regionList.map(g => g.path)
                                    .filter(p => root.hostedRegions.indexOf(p) < 0))
                        }
                    }
                    }
                    // -- Output --
                    OutputBox {
                        id: out
                        Layout.fillWidth: true
                        Layout.preferredHeight: 260
                        Layout.bottomMargin: 8
                    }
                }
            }

            // ============ REGIONS ============
            // The closed set with each region's on-chain status. The set is
            // static (the prize froze it); the statuses come from one bulk
            // registry read (lookup_bulk) — cached in the state file, so the
            // badges often render before any network call — and clicking a
            // row still reads the full registry entry from the chain.
            ColumnLayout {
                anchors.fill: parent
                anchors.rightMargin: 24   // match the scrollable tabs' gutter
                visible: root.currentTab === 1
                spacing: 16

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16
                    Text {
                        text: "Region set"
                        font.family: root.fSerif
                        font.pixelSize: 19
                        font.letterSpacing: -0.2
                        color: root.cInk
                    }
                    Text {
                        text: regionModel.count + " in the set"
                              + (root.registeredShown() > 0
                                 ? " · " + root.registeredShown() + " registered" : "")
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.cMuted
                        Layout.alignment: Qt.AlignBottom
                        Layout.bottomMargin: 2
                    }
                    Item { Layout.fillWidth: true }
                    SButton {
                        text: "Re-check"
                        onClicked: root.call("lookup_bulk", {})
                    }
                    PButton {
                        text: "List regions"
                        onClicked: root.call("regions")
                    }
                }

                // The filter row: All · Registered · Not registered (a
                // three-state toggle over the same model).
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10
                    Repeater {
                        model: [ { id: "all", label: "All" },
                                 { id: "registered", label: "Registered" },
                                 { id: "not", label: "Not registered" } ]
                        delegate: RowLayout {
                            spacing: 10
                            Text {
                                text: modelData.label
                                font.family: root.fSans
                                font.pixelSize: 12
                                font.weight: root.regionFilter === modelData.id ? Font.DemiBold : Font.Medium
                                color: root.regionFilter === modelData.id ? root.cAccent : root.cMuted
                                MouseArea {
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: root.setRegionFilter(modelData.id)
                                }
                            }
                            Text {
                                // the separator dot between filter options
                                visible: index < 2
                                text: "·"
                                font.pixelSize: 12
                                color: root.cFaint
                            }
                        }
                    }
                    Item { Layout.fillWidth: true }
                }

                Hint {
                    text: "Badges come from one bulk registry read (cached; Re-check refreshes it). Click a row to print the full registry entry from the chain."
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    radius: 12
                    color: root.cPanel
                    border.width: 1
                    border.color: root.cHair
                    clip: true

                    Text {
                        visible: regionModel.count === 0
                        anchors.centerIn: parent
                        width: parent.width - 48
                        text: "The closed region set is frozen by the prize.\nPress List regions to load every region."
                        horizontalAlignment: Text.AlignHCenter
                        font.family: root.fSans
                        font.pixelSize: 12
                        lineHeight: 1.625
                        lineHeightMode: Text.ProportionalHeight
                        color: root.cFaint
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: 16
                            Layout.rightMargin: 16
                            Layout.topMargin: 10
                            Layout.bottomMargin: 8
                            spacing: 12
                            Item { Layout.preferredWidth: 7 }
                            Text {
                                text: "REGION"
                                font.family: root.fSans
                                font.pixelSize: 11
                                font.letterSpacing: 1.0
                                font.weight: Font.Medium
                                color: root.cMuted
                                Layout.preferredWidth: 220
                            }
                            Text {
                                text: "STATUS"
                                font.family: root.fSans
                                font.pixelSize: 11
                                font.letterSpacing: 1.0
                                font.weight: Font.Medium
                                color: root.cMuted
                                Layout.fillWidth: true
                            }
                        }
                        Rectangle { Layout.fillWidth: true; height: 1; color: root.cHair }

                        ListView {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: ListModel { id: regionModel; objectName: "regionModel" }
                            delegate: Rectangle {
                                width: ListView.view.width
                                height: 38
                                color: rowMouse.containsMouse ? root.cPanelHi : "transparent"
                                Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 16
                                    anchors.rightMargin: 16
                                    spacing: 12
                                    Rectangle {
                                        Layout.preferredWidth: 7
                                        Layout.preferredHeight: 7
                                        radius: 4
                                        color: model.registered ? root.cAccent
                                             : (model.error && model.error.length > 0
                                                ? root.cBrown
                                                : Qt.rgba(0.961, 0.961, 0.941, 0.28))
                                    }
                                    Text {
                                        text: model.path
                                        font.family: root.fMono
                                        font.pixelSize: 12
                                        color: root.cInk
                                        Layout.preferredWidth: 220
                                        elide: Text.ElideRight
                                    }
                                    Text {
                                        // registered: "◆ registered · v<latest>"
                                        // (the dot is amber/red when the read
                                        // failed — the error text says why);
                                        // otherwise a muted dash.
                                        text: model.registered
                                              ? ("◆ registered" + (model.latest
                                                   ? " · v" + model.latest : ""))
                                              : (model.error && model.error.length > 0
                                                 ? "read failed"
                                                 : "—")
                                        font.family: model.registered || (model.error && model.error.length > 0)
                                                      ? root.fMono : root.fSans
                                        font.pixelSize: 12
                                        color: model.registered ? root.cAccent
                                             : (model.error && model.error.length > 0
                                                ? Qt.rgba(0.631, 0.525, 0.388, 0.9) : root.cFaint)
                                        Layout.fillWidth: true
                                        elide: Text.ElideRight
                                        ToolTip.visible: rowMouse.containsMouse
                                        ToolTip.delay: 300
                                        ToolTip.text: (model.error && model.error.length > 0)
                                                      ? model.error
                                                      : (model.registered
                                                         ? "registered on-chain"
                                                         : "no registration on-chain")
                                    }
                                }
                                Rectangle {
                                    anchors.bottom: parent.bottom
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    height: 1
                                    color: root.cHairSoft
                                    visible: !ListView.view || index < ListView.view.count - 1
                                }
                                MouseArea {
                                    id: rowMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: root.call("lookup", { region: model.path })
                                }
                            }
                        }
                    }
                }

                OutputBox {
                    id: outR
                    Layout.fillWidth: true
                    Layout.preferredHeight: 132
                }
            }

            // ============ CONSUMER ============
            Flickable {
                id: consFlick
                anchors.fill: parent
                visible: root.currentTab === 2
                clip: true
                contentWidth: width
                contentHeight: consCol.implicitHeight
                ScrollBar.vertical: ConsoleScrollBar { }

                ColumnLayout {
                    id: consCol
                    // kept clear of the overlay scrollbar
                    width: consFlick.width - 24
                    spacing: 24

                    // Each section has its own single-select region picker:
                    // clicking a region in the popup sets that section's
                    // region and closes it; the pinned chip below the header
                    // shows region · size with × to clear.
                    Section {
                        id: secFetch
                        numeral: "01"
                        heading: "Fetch a registered snapshot (storage → verify → import)"

                        Rectangle {
                            visible: root.fetchRegion !== ""
                            radius: 4
                            color: Qt.rgba(0.961, 0.961, 0.941, 0.08)
                            border.width: 1
                            border.color: root.cHair
                            width: fetchChipRow.implicitWidth + 16
                            height: 26
                            RowLayout {
                                id: fetchChipRow
                                anchors.centerIn: parent
                                spacing: 6
                                Text {
                                    text: root.fetchRegion
                                          + (root.regionSizes[root.fetchRegion] !== undefined
                                             ? "  ·  " + root.fmtBytes(root.regionSizes[root.fetchRegion]) : "")
                                    font.family: root.fMono
                                    font.pixelSize: 11
                                    color: root.cTan
                                }
                                Text {
                                    text: "×"
                                    font.family: root.fSans
                                    font.pixelSize: 13
                                    color: root.cMuted
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: root.setFetchRegion("")
                                    }
                                }
                            }
                        }
                        RowLayout {
                            spacing: 12
                            SButton {
                            id: fetchPickBtn
                                text: (root.fetchRegion === "")
                                      ? (root.sizesLoading ? "Select region  (sizes loading…)" : "Select region ▾")
                                      : (root.fetchRegion + " ▾")
                                onClicked: fetchPicker.open()
                                RegionPickPopup {
                                    id: fetchPicker
                                anchorItem: fetchPickBtn
                                    singleSelect: true
                                    single: root.fetchRegion
                                    onPicked: function(path) { root.setFetchRegion(path) }
                                }
                            }
                            Item { Layout.fillWidth: true }
                        }

                        GridLayout {
                            columns: 2
                            Layout.fillWidth: true
                            columnSpacing: 16
                            rowSpacing: 12
                            FieldCell {
                                label: "CID"
                                caption: {
                                    switch (root.fetchFillState) {
                                        case "ok":            return "from registry ✓"
                                        case "pending":       return "reading the registry…"
                                        case "unregistered":  return "not on the registry yet — nothing to fetch"
                                        case "unreachable":   return "registry unreachable — enter manually"
                                        default:              return "auto-filled from the registry when you pick a region"
                                    }
                                }
                                captionColor: root.fetchFillState === "unreachable" ? root.cWarn
                                            : (root.fetchFillState === "ok" ? root.cTeal : root.cMuted)
                                MonoField {
                                    id: fCid
                                    objectName: "fCid"
                                    placeholderText: "from the registry"
                                    // A light presence check: non-empty is
                                    // enough for a CID.
                                    validation: fCid.text.trim().length > 0 ? "valid" : "none"
                                }
                            }
                            FieldCell {
                                label: "PUBLISHED MD5 (HEX)"
                                caption: "the registered checksum — auto-filled"
                                MonoField {
                                    id: fMd5
                                    objectName: "fMd5"
                                    placeholderText: "from the registry"
                                    validation: {
                                        const t = fMd5.text.trim()
                                        if (t.length === 0) return "none"
                                        return /^[0-9a-fA-F]{32}$/.test(t) ? "valid" : "invalid"
                                    }
                                }
                            }
                        }
                        RowLayout {
                            spacing: 12
                            PButton {
                                enabled: root.fetchRegion !== ""
                                opacity: enabled ? 1.0 : 0.4
                                text: "Fetch + import"
                                onClicked: root.call("fetch", {
                                    region: root.fetchRegion,
                                    cid: fCid.text.trim(),
                                    md5_hex: fMd5.text.trim(),
                                    // Job mode: a fetch pulls the full region
                                    // PBF — minutes — so its result comes via
                                    // job_status polls, not this call.
                                    job: true
                                })
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }

                    Section {
                        id: secImport
                        numeral: "02"
                        heading: "Import a local PBF"

                        Rectangle {
                            visible: root.importRegion !== ""
                            radius: 4
                            color: Qt.rgba(0.961, 0.961, 0.941, 0.08)
                            border.width: 1
                            border.color: root.cHair
                            width: importChipRow.implicitWidth + 16
                            height: 26
                            RowLayout {
                                id: importChipRow
                                anchors.centerIn: parent
                                spacing: 6
                                Text {
                                    text: root.importRegion
                                          + (root.regionSizes[root.importRegion] !== undefined
                                             ? "  ·  " + root.fmtBytes(root.regionSizes[root.importRegion]) : "")
                                    font.family: root.fMono
                                    font.pixelSize: 11
                                    color: root.cTan
                                }
                                Text {
                                    text: "×"
                                    font.family: root.fSans
                                    font.pixelSize: 13
                                    color: root.cMuted
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: root.setImportRegion("")
                                    }
                                }
                            }
                        }
                        RowLayout {
                            spacing: 12
                            SButton {
                            id: importPickBtn
                                text: (root.importRegion === "")
                                      ? (root.sizesLoading ? "Select region  (sizes loading…)" : "Select region ▾")
                                      : (root.importRegion + " ▾")
                                onClicked: importPicker.open()
                                RegionPickPopup {
                                    id: importPicker
                                anchorItem: importPickBtn
                                    singleSelect: true
                                    single: root.importRegion
                                    onPicked: function(path) { root.setImportRegion(path) }
                                }
                            }
                            Item { Layout.fillWidth: true }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            FieldCell {
                                label: "PBF FILE"
                                caption: "a .osm.pbf file on disk"
                                MonoField { id: impPath; placeholderText: "/path/to/region-latest.osm.pbf" }
                            }
                        }
                        RowLayout {
                            spacing: 12
                            SButton {
                                // The region's hosted snapshot is already on
                                // disk in the cache dir — one click fills the
                                // path from the local catalog.
                                visible: root.cachedPathFor(root.importRegion) !== ""
                                text: "use cached file"
                                onClicked: impPath.text = root.cachedPathFor(root.importRegion)
                            }
                            SButton {
                                enabled: root.importRegion !== "" && impPath.text.trim().length > 0
                                opacity: enabled ? 1.0 : 0.4
                                text: "Import (validate + catalog)"
                                onClicked: root.call("import", {
                                    region: root.importRegion,
                                    pbf: impPath.text.trim()
                                })
                            }
                            PButton {
                                // The full local-import workflow: verify the file
                                // against Geofabrik's published MD5, store in Logos
                                // Storage, and print the registration tx.
                                enabled: root.importRegion !== "" && impPath.text.trim().length > 0
                                opacity: enabled ? 1.0 : 0.4
                                text: "Verify + store + register tx"
                                onClicked: root.call("import", {
                                    region: root.importRegion,
                                    pbf: impPath.text.trim(),
                                    store: true,
                                    registrar_hex: registrar.text.trim()
                                })
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }

                    Section {
                        id: secUpdate
                        numeral: "03"
                        heading: "Update check"

                        Rectangle {
                            visible: root.updateRegion !== ""
                            radius: 4
                            color: Qt.rgba(0.961, 0.961, 0.941, 0.08)
                            border.width: 1
                            border.color: root.cHair
                            width: updateChipRow.implicitWidth + 16
                            height: 26
                            RowLayout {
                                id: updateChipRow
                                anchors.centerIn: parent
                                spacing: 6
                                Text {
                                    text: root.updateRegion
                                          + (root.regionSizes[root.updateRegion] !== undefined
                                             ? "  ·  " + root.fmtBytes(root.regionSizes[root.updateRegion]) : "")
                                    font.family: root.fMono
                                    font.pixelSize: 11
                                    color: root.cTan
                                }
                                Text {
                                    text: "×"
                                    font.family: root.fSans
                                    font.pixelSize: 13
                                    color: root.cMuted
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: root.setUpdateRegion("")
                                    }
                                }
                            }
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            SButton {
                            id: updatePickBtn
                                text: (root.updateRegion === "")
                                      ? (root.sizesLoading ? "Select region  (sizes loading…)" : "Select region ▾")
                                      : (root.updateRegion + " ▾")
                                onClicked: updatePicker.open()
                                RegionPickPopup {
                                    id: updatePicker
                                anchorItem: updatePickBtn
                                    singleSelect: true
                                    single: root.updateRegion
                                    onPicked: function(path) { root.setUpdateRegion(path) }
                                }
                            }
                            SButton {
                                Layout.alignment: Qt.AlignBottom
                                enabled: root.updateRegion !== ""
                                opacity: enabled ? 1.0 : 0.4
                                text: "Check for newer snapshot"
                                onClicked: root.call("update", {
                                    region: root.updateRegion
                                })
                            }
                            Item { Layout.fillWidth: true }
                        }
                        RowLayout {
                            spacing: 12
                            SButton {
                                text: "Local catalog"
                                onClicked: root.call("catalog")
                            }
                            Item { Layout.fillWidth: true }
                        }
                    }

                    OutputBox {
                        id: out2
                        Layout.fillWidth: true
                        Layout.preferredHeight: 260
                        Layout.bottomMargin: 8
                    }
                }
            }

            // ============ GUIDE ============
            // The bundled documentation: a doc list on the left, the rendered
            // page on the right. The markdown ships as string constants in
            // guide/GuideDocs.js (a pragma-library JS file inside the QML
            // dir, packaged with it) — the sandboxed engine can read files
            // from its own qml tree through the type loader, but QML
            // XMLHttpRequest is disabled for local files (Qt 6 default) and
            // the engine's network stack is deny-all in Basecamp, so runtime
            // file reads were not an option (see STATE.md).
            Item {
                anchors.fill: parent
                visible: root.currentTab === 3

                RowLayout {
                    anchors.fill: parent
                    spacing: 16

                    // -- Doc list --
                    Rectangle {
                        Layout.preferredWidth: 220
                        Layout.fillHeight: true
                        Layout.alignment: Qt.AlignTop
                        radius: 12
                        color: root.cPanel
                        border.width: 1
                        border.color: root.cHair
                        clip: true

                        ColumnLayout {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.top: parent.top
                            anchors.margins: 10
                            spacing: 8
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 8
                                Rectangle { width: 6; height: 6; radius: 3; color: root.cTeal }
                                Text {
                                    text: "GUIDE"
                                    font.family: root.fSans
                                    font.pixelSize: 11
                                    font.letterSpacing: 1.4
                                    font.weight: Font.Medium
                                    color: root.cMuted
                                }
                                Item { Layout.fillWidth: true }
                            }
                            ListView {
                                id: guideList
                                Layout.fillWidth: true
                                Layout.preferredHeight: contentItem.childrenRect.height
                                clip: true
                                spacing: 2
                                model: ListModel {
                                    id: guideModel
                                    Component.onCompleted: {
                                        // The doc list comes from the library,
                                        // so titles can't drift from the docs.
                                        if (typeof GuideDocs === "undefined") return
                                        for (const d of GuideDocs.list())
                                            append({ did: d.id, title: d.title })
                                    }
                                }
                                delegate: Rectangle {
                                    width: guideList.width
                                    height: 38
                                    radius: 6
                                    color: root.guideDoc === model.did ? root.cPanelHi
                                         : (guideMouse.containsMouse ? Qt.rgba(0.886, 0.878, 0.788, 0.05) : "transparent")
                                    Behavior on color { ColorAnimation { duration: 150; easing.type: Easing.InOutQuad } }
                                    Rectangle {
                                        visible: root.guideDoc === model.did
                                        anchors.left: parent.left
                                        anchors.leftMargin: 6
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 3; height: 18; radius: 1
                                        color: root.cAccent
                                    }
                                    Text {
                                        anchors.verticalCenter: parent.verticalCenter
                                        anchors.left: parent.left
                                        anchors.leftMargin: 16
                                        anchors.right: parent.right
                                        anchors.rightMargin: 8
                                        text: model.title
                                        font.family: root.fSans
                                        font.pixelSize: 12
                                        font.weight: root.guideDoc === model.did ? Font.DemiBold : Font.Medium
                                        color: root.guideDoc === model.did ? root.cTan : root.cMuted
                                        elide: Text.ElideRight
                                    }
                                    MouseArea {
                                        id: guideMouse
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: root.guideDoc = model.did
                                    }
                                }
                            }
                            Item { Layout.fillHeight: true }
                        }
                    }

                    // -- The rendered page --
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        radius: 12
                        color: root.cPanel
                        border.width: 1
                        border.color: root.cHair
                        clip: true

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 14
                            spacing: 10

                            // The active document's title.
                            Text {
                                Layout.fillWidth: true
                                text: (typeof GuideDocs !== "undefined" && GuideDocs.doc(root.guideDoc))
                                      ? GuideDocs.doc(root.guideDoc).title : ""
                                font.family: root.fSerif
                                font.pixelSize: 19
                                font.letterSpacing: -0.2
                                color: root.cInk
                                elide: Text.ElideRight
                            }

                            ScrollView {
                                id: guideScroll
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                ScrollBar.vertical: ConsoleScrollBar { }
                                Text {
                                    id: guideText
                                    objectName: "guideText"
                                    width: guideScroll.availableWidth
                                    text: (typeof GuideDocs !== "undefined" && GuideDocs.doc(root.guideDoc))
                                          ? GuideDocs.doc(root.guideDoc).body : ""
                                    textFormat: Text.MarkdownText
                                    wrapMode: Text.Wrap
                                    font.family: root.fSans
                                    font.pixelSize: 13
                                    lineHeight: 1.5
                                    lineHeightMode: Text.ProportionalHeight
                                    color: root.cInk
                                    onLinkActivated: function(link) { Qt.openUrlExternally(link) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
