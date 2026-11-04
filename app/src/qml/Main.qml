import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

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
    property int    pendingRow: -1        // Regions tab: row awaiting a lookup
    // Region picker state: the frozen set with live sizes (fetched after
    // startup), the picker's selection, and the locally hosted set (from the
    // catalog op) for the ever-valid "host all".
    property var    regionList: []          // [{path, name, level}]
    property var    regionSizes: ({})       // {path: bytes}
    property bool   sizesLoading: false
    property var    selectedRegions: []     // [path]
    property var    hostedRegions: []       // [path] from the catalog
    property string discoverState: "pending"   // pending | ok | error
    property int    regionsAvailable: 0
    property int    regionsTotal: 72
    property string discoverError: ""

    // Txns picker state (Host §03): the regions chosen for the register /
    // register_bulk builders. Own selection state, same selection method as
    // the §02 picker.
    property var txSelectedRegions: []     // [path]

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

    function regionDisplay(path) {
        const s = root.regionSizes[path]
        const size = (s !== undefined) ? ("  ·  " + fmtBytes(s)) : ""
        return path + size
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
    function toggleTxRegion(path) {
        root.txSelectedRegions = root.toggleIn(root.txSelectedRegions, path)
    }
    // Shared picker toggle: returns a fresh array, so reassigning the
    // selection property notifies every binding that reads it.
    function toggleIn(list, path) {
        const i = list.indexOf(path)
        if (i >= 0) list.splice(i, 1)
        else list.push(path)
        return list.slice()
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
            default:
                return "ok"
        }
    }

    // A result landed: resolve the OLDEST running task whose core op matches
    // the routed op (r.op when the core tags results, else the local
    // pendingOp tag, normalized back to the core name).
    function resolveTaskResult(op, r) {
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
        root.call("host", { region: root.hostCurrent })
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

    // Route a completed op's result. Consoles per tab; the two list-driving
    // ops (regions, lookup) also update the Regions tab's model.
    //
    // Ops arrive tagged by the core with the CORE's op name (r.op in the
    // router), while locally-dispatched tags ("regions-list", "catalog-list")
    // still apply for cores that predate op tagging — every arm accepts
    // both. The startup chain is serialized (each op fired from the previous
    // result) because firing an op while another is still in flight can
    // silently drop it in the RemoteObjects transport. Order (v1.3.2):
    // open (seeds the picker from sizes cached in the state file) →
    // discover → regions → sizes (live refresh; fires catalog even on
    // error so the chain never strands) → catalog (the slow LEZ RPC,
    // last).
    function onOpResult(op, r) {
        root.resolveTaskResult(op, r)
        const consoleText = root.pretty(r)

        if (op === "open") {
            if (r.ok) {
                // Seed the picker from the sizes cached in the state file so
                // it renders before the live `sizes` refresh lands.
                if (r.result && r.result.region_sizes) root.regionSizes = r.result.region_sizes
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
                root.hostedRegions = cat.map(c => c.region)
            } else {
                root.hostedRegions = []
            }
        } else if (op === "host") {
            // The sequential host loop advances from this result.
            root.onHostResult(r)
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
        if (root.currentTab === 0)      out.outputText  = consoleText
        else if (root.currentTab === 1) outR.outputText = consoleText
        else                            out2.outputText = consoleText

        if (op === "regions" || op === "regions-list") {
            regionModel.clear()
            if (r.ok && r.result && r.result.regions) {
                for (const g of r.result.regions) {
                    regionModel.append({
                        path: g.path, name: g.name, level: g.level,
                        status: "unknown", detail: ""
                    })
                }
            }
        } else if (op.indexOf("lookup:") === 0) {
            const row = root.pendingRow
            if (row >= 0 && row < regionModel.count) {
                if (r.ok && r.result) {
                    const latest = r.result.latest
                    regionModel.setProperty(row, "status", "hosted")
                    regionModel.setProperty(row, "detail",
                        latest ? String(latest.version) : "?")
                } else {
                    regionModel.setProperty(row, "status", "not-hosted")
                    regionModel.setProperty(row, "detail", "")
                }
                root.pendingRow = -1
            }
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
        onTriggered: root.taskTick++
    }

    // ---- Logos identity tokens (from logos.co live CSS) ------------------
    readonly property color cCanvas:   "#152521"                            // brand dark green — the canvas
    readonly property color cInk:      "#f5f5ef"                            // brand off-white
    readonly property color cAccent:   "#ffd328"                            // brand yellow — the one accent
    readonly property color cTan:      "#e2e0c9"                            // accent tan — secondary surfaces
    readonly property color cTeal:     "#5f797c"                            // steel teal — muted accents
    readonly property color cInfo:     "#c6ebf7"                            // light blue — info, links
    readonly property color cBrown:    "#a18863"                            // brown — "not hosted" status

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
            onClicked: root.currentTab = btn.tabIndex
        }
    }

    component SectionHead: RowLayout {
        id: head
        property string numeral: ""
        property string heading: ""
        spacing: 12
        Layout.fillWidth: true
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
    // the default child.
    component FieldCell: ColumnLayout {
        id: cell
        property string label: ""
        default property alias fieldData: slot.data
        Layout.fillWidth: true
        Layout.preferredWidth: 1     // equal-weight columns in grids/rows
        spacing: 6
        Text {
            text: cell.label
            font.family: root.fSans
            font.pixelSize: 11
            font.letterSpacing: 1.0
            font.weight: Font.Medium
            color: root.cMuted
        }
        ColumnLayout { id: slot; Layout.fillWidth: true; spacing: 0 }
    }

    // Every machine value (URL, hex id, path, region slug) is mono.
    component MonoField: TextField {
        id: control
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
            border.color: control.activeFocus ? Qt.rgba(0.776, 0.922, 0.969, 0.55)
                        : (control.hovered ? Qt.rgba(0.961, 0.961, 0.941, 0.26)
                                           : Qt.rgba(0.961, 0.961, 0.941, 0.14))
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
                        text: root.osmVersion.length ? root.osmVersion : "—"
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.cInfo
                        elide: Text.ElideRight
                        Layout.maximumWidth: 380
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.runDiscover()
                    ToolTip.visible: containsMouse
                    ToolTip.text: "Re-run the live Geofabrik cross-check"
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
                    y: -(height + 8)
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
                    SectionHead { numeral: "01"; heading: "Connect" }
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
                            MonoField { id: storageUrl; text: "http://127.0.0.1:8080" }
                        }
                        FieldCell {
                            label: "GEOFABRIK BASE URL"
                            MonoField { id: geofabrikUrl; text: "https://download.geofabrik.de" }
                        }
                        FieldCell {
                            label: "CACHE DIR"
                            MonoField { id: cacheDir; text: "osm-cache" }
                        }
                        FieldCell {
                            label: "REGISTRAR (HEX, 32B)"
                            MonoField { id: registrar; placeholderText: "registrar account id" }
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

                    // -- Host --
                    SectionHead { numeral: "02"; heading: "Host (download → verify MD5 → Logos Storage)" }

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

                        // The picker button + popup with the region list.
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: (root.selectedRegions.length === 0)
                                  ? (root.sizesLoading ? "Select regions  (sizes loading…)" : "Select regions ▾")
                                  : (root.selectedRegions.length + " selected ▾")
                            onClicked: regionPicker.open()
                            Popup {
                                id: regionPicker
                                x: 0
                                y: -(height + 8)
                                width: 520
                                height: 400
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
                                        TextField {
                                            id: pickerSearch
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
                                            onTextChanged: pickerModel.update()
                                        }
                                        Text {
                                            text: root.sizesLoading ? "sizes…" : root.regionList.length + " regions"
                                            font.family: root.fSans
                                            font.pixelSize: 10
                                            color: root.cMuted
                                        }
                                    }
                                    ListView {
                                        id: pickerList
                                        Layout.fillWidth: true
                                        Layout.fillHeight: true
                                        clip: true
                                        ScrollBar.vertical: ConsoleScrollBar { }
                                        model: ListModel { id: pickerModel
                                            function update() {
                                                clear()
                                                const needle = (pickerSearch.text || "").toLowerCase()
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
                                            width: pickerList.width
                                            height: 34
                                            color: pickerMouse.containsMouse ? root.cPanelHi : "transparent"
                                            RowLayout {
                                                anchors.fill: parent
                                                anchors.leftMargin: 10
                                                anchors.rightMargin: 10
                                                spacing: 8
                                                Text {
                                                    text: root.selectedRegions.indexOf(model.path) >= 0 ? "×" : "·"
                                                    color: root.selectedRegions.indexOf(model.path) >= 0 ? root.cAccent : root.cMuted
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
                                                id: pickerMouse
                                                anchors.fill: parent
                                                hoverEnabled: true
                                                cursorShape: Qt.PointingHandCursor
                                                onClicked: root.toggleRegion(model.path)
                                            }
                                        }
                                        Connections {
                                            target: root
                                            function onRegionListChanged() { pickerModel.update() }
                                            function onRegionSizesChanged() { pickerModel.update() }
                                        }
                                        Component.onCompleted: pickerModel.update()
                                    }
                                }
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

                    // -- On-chain tx builders (wallet submits) --
                    SectionHead { numeral: "03"; heading: "LEZ transactions (the wallet submits these)" }

                    // Tx-region selection: the same method as the Host §02
                    // picker (own selection state, chips + popup), replacing
                    // the old free-text REGION field. "Register tx" takes the
                    // single selected region; "Batch tx" takes them all.
                    Flow {
                        id: txChips
                        Layout.fillWidth: true
                        spacing: 6
                        visible: root.txSelectedRegions.length > 0
                        Repeater {
                            model: root.txSelectedRegions
                            delegate: Rectangle {
                                radius: 4
                                color: Qt.rgba(0.961, 0.961, 0.941, 0.08)
                                border.width: 1
                                border.color: root.cHair
                                width: txChipRow.implicitWidth + 16
                                height: 26
                                RowLayout {
                                    id: txChipRow
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
                                            onClicked: root.toggleTxRegion(modelData)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12

                        // The txns picker button + popup (same construction as
                        // the §02 region picker).
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: (root.txSelectedRegions.length === 0)
                                  ? (root.sizesLoading ? "Select regions  (sizes loading…)" : "Select regions ▾")
                                  : (root.txSelectedRegions.length + " selected ▾")
                            onClicked: txRegionPicker.open()
                            Popup {
                                id: txRegionPicker
                                x: 0
                                y: -(height + 8)
                                width: 520
                                height: 400
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
                                        TextField {
                                            id: txPickerSearch
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
                                            onTextChanged: txPickerModel.update()
                                        }
                                        Text {
                                            text: root.sizesLoading ? "sizes…" : root.regionList.length + " regions"
                                            font.family: root.fSans
                                            font.pixelSize: 10
                                            color: root.cMuted
                                        }
                                    }
                                    ListView {
                                        id: txPickerList
                                        Layout.fillWidth: true
                                        Layout.fillHeight: true
                                        clip: true
                                        ScrollBar.vertical: ConsoleScrollBar { }
                                        model: ListModel { id: txPickerModel
                                            function update() {
                                                clear()
                                                const needle = (txPickerSearch.text || "").toLowerCase()
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
                                            width: txPickerList.width
                                            height: 34
                                            color: txPickerMouse.containsMouse ? root.cPanelHi : "transparent"
                                            RowLayout {
                                                anchors.fill: parent
                                                anchors.leftMargin: 10
                                                anchors.rightMargin: 10
                                                spacing: 8
                                                Text {
                                                    text: root.txSelectedRegions.indexOf(model.path) >= 0 ? "×" : "·"
                                                    color: root.txSelectedRegions.indexOf(model.path) >= 0 ? root.cAccent : root.cMuted
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
                                                id: txPickerMouse
                                                anchors.fill: parent
                                                hoverEnabled: true
                                                cursorShape: Qt.PointingHandCursor
                                                onClicked: root.toggleTxRegion(model.path)
                                            }
                                        }
                                        Connections {
                                            target: root
                                            function onRegionListChanged() { txPickerModel.update() }
                                            function onRegionSizesChanged() { txPickerModel.update() }
                                        }
                                        Component.onCompleted: txPickerModel.update()
                                    }
                                }
                            }
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            enabled: root.txSelectedRegions.length === 1
                            opacity: enabled ? 1.0 : 0.4
                            text: "Register tx"
                            onClicked: root.call("register", {
                                region: root.txSelectedRegions[0],
                                registrar_hex: registrar.text
                            })
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            enabled: root.txSelectedRegions.length > 0
                            opacity: enabled ? 1.0 : 0.4
                            text: "Batch tx (region,cid pairs via catalog)"
                            onClicked: root.call("register_bulk", {
                                regions: root.txSelectedRegions.join(","),
                                registrar_hex: registrar.text
                            })
                        }
                        Item { Layout.fillWidth: true }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "OWNER (HEX, 32B)"
                            MonoField { id: ownerHex; placeholderText: "owner (hex, 32B)" }
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Init tx"
                            onClicked: root.call("init", { owner_hex: ownerHex.text })
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
            // The closed set, with each region's on-chain status. The set is
            // static (the prize froze it), so the list renders without a
            // network call; the per-region status is read when a row is
            // clicked, because a status costs one chain read.
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
                        font.family: root.fMono
                        font.pixelSize: 11
                        color: root.cMuted
                        Layout.alignment: Qt.AlignBottom
                        Layout.bottomMargin: 2
                    }
                    Item { Layout.fillWidth: true }
                    PButton {
                        text: "List regions"
                        onClicked: root.call("regions")
                    }
                }

                Hint {
                    text: "Click a row to read its registry entry from the chain. Hosted regions show the latest registered version."
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
                        text: "The closed region set is frozen by the prize.\nPress Load set to list every region."
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
                            model: ListModel { id: regionModel }
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
                                        color: model.status === "hosted" ? root.cAccent
                                             : (model.status === "not-hosted" ? root.cBrown
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
                                        text: model.status === "hosted"
                                              ? ("v" + model.detail)
                                              : (model.status === "not-hosted" ? "not hosted" : "not probed")
                                        font.family: model.status === "hosted" ? root.fMono : root.fSans
                                        font.pixelSize: 12
                                        color: model.status === "hosted" ? root.cInfo
                                             : (model.status === "not-hosted" ? Qt.rgba(0.631, 0.525, 0.388, 0.9) : root.cFaint)
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
                                    visible: !ListView.view || index < ListView.view.count - 1
                                }
                                MouseArea {
                                    id: rowMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        root.pendingRow = index
                                        root.call("lookup", { region: model.path },
                                                  "lookup:" + model.path)
                                    }
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

                    SectionHead { numeral: "01"; heading: "Fetch a registered snapshot (storage → verify → import)" }
                    GridLayout {
                        columns: 2
                        Layout.fillWidth: true
                        columnSpacing: 16
                        rowSpacing: 12
                        FieldCell {
                            label: "REGION"
                            MonoField { id: fRegion; text: "germany" }
                        }
                        FieldCell {
                            label: "CID"
                            MonoField { id: fCid; placeholderText: "from the registry" }
                        }
                        FieldCell {
                            label: "PUBLISHED MD5 (HEX)"
                            MonoField { id: fMd5; placeholderText: "from the registry" }
                        }
                    }
                    RowLayout {
                        spacing: 12
                        PButton {
                            text: "Fetch + import"
                            onClicked: root.call("fetch", {
                                region: fRegion.text.trim(),
                                cid: fCid.text.trim(),
                                md5_hex: fMd5.text.trim()
                            })
                        }
                        Item { Layout.fillWidth: true }
                    }

                    SectionHead { numeral: "02"; heading: "Import a local PBF" }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "REGION"
                            MonoField { id: impRegion; placeholderText: "region"; text: "germany" }
                        }
                        FieldCell {
                            label: "PBF FILE"
                            MonoField { id: impPath; placeholderText: "/path/to/region-latest.osm.pbf" }
                        }
                    }
                    RowLayout {
                        spacing: 12
                        SButton {
                            text: "Import (validate + catalog)"
                            onClicked: root.call("import", {
                                region: impRegion.text.trim(),
                                pbf: impPath.text.trim()
                            })
                        }
                        PButton {
                            // The full local-import workflow: verify the file
                            // against Geofabrik's published MD5, store in Logos
                            // Storage, and print the registration tx.
                            text: "Verify + store + register tx"
                            onClicked: root.call("import", {
                                region: impRegion.text.trim(),
                                pbf: impPath.text.trim(),
                                store: true,
                                registrar_hex: registrar.text.trim()
                            })
                        }
                        Item { Layout.fillWidth: true }
                    }

                    SectionHead { numeral: "03"; heading: "Update check" }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "REGION"
                            MonoField { id: updRegion; placeholderText: "region"; text: "germany" }
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Check for newer snapshot"
                            onClicked: root.call("update", {
                                region: updRegion.text.trim()
                            })
                        }
                    }
                    RowLayout {
                        spacing: 12
                        SButton {
                            text: "Local catalog"
                            onClicked: root.call("catalog")
                        }
                        Item { Layout.fillWidth: true }
                    }

                    OutputBox {
                        id: out2
                        Layout.fillWidth: true
                        Layout.preferredHeight: 260
                        Layout.bottomMargin: 8
                    }
                }
            }
        }
    }
}
