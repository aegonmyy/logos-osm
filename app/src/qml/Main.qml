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
    property string discoverState: "pending"   // pending | ok | error
    property int    regionsAvailable: 0
    property int    regionsTotal: 72
    property string discoverError: ""

    // Auto-init: open the SDK session on startup; discover follows once the
    // session is up. The user never thinks about "Open" — the fields above
    // are applied automatically, and editing them re-fires on the next
    // restart (state persists under the module data tree).
    function openArgs() {
        const args = {
            // Absolute path: the module host's CWD is not stable (it can
            // even be a deleted dir), so relative state paths break.
            state_path: "/home/codespace/.local/share/Logos/LogosBasecamp/module_data/osm/osm-state.json",
            storage_url: storageUrl.text,
            geofabrik_url: geofabrikUrl.text,
            cache_dir: cacheDir.text,
            // The registry this app talks to on the live LEZ testnet: the
            // OSM guest program deployed 2026-10-30.
            program_account_hex: "b2fa65027d9b2540d4ac2aa4e0a38040ac03604c2201930592c547f3d65ecb43"
        }
        if (registrar.text.length) args.registrar_hex = registrar.text
        return args
    }

    function runDiscover() {
        root.discoverState = "pending"
        root.call("discover")
    }

    Component.onCompleted: {
        root.call("open", root.openArgs())
    }

    function call(op, args, tag) {
        if (!root.bk) return
        root.pendingOp = tag || op
        root.bk.invokeOp(op, JSON.stringify(args || {}))
    }

    function pretty(obj) { return JSON.stringify(obj, null, 2) }

    // Route a completed op's result. Consoles per tab; the two list-driving
    // ops (regions, lookup) also update the Regions tab's model.
    function onOpResult(op, r) {
        const consoleText = root.pretty(r)

        if (op === "open") {
            if (r.ok) root.runDiscover()
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
        }
        if (root.currentTab === 0)      out.outputText  = consoleText
        else if (root.currentTab === 1) outR.outputText = consoleText
        else                            out2.outputText = consoleText

        if (op === "regions") {
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
            root.onOpResult(root.pendingOp, r)
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
            TextArea {
                id: ta
                Layout.fillWidth: true
                Layout.fillHeight: true
                readOnly: true
                wrapMode: TextArea.Wrap
                // Explicit scrollbar: the hosting page is a Flickable that
                // eats wheel events, so overflowing results need a draggable
                // bar to climb.
                ScrollBar.vertical: ConsoleScrollBar { }
                font.family: root.fMono
                font.pixelSize: 12
                color: root.cTan
                selectionColor: Qt.rgba(0.961, 0.961, 0.941, 0.22)
                selectedTextColor: root.cTan
                background: Item { }
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
                    ToolTip.visible: hovered
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
                    ToolTip.visible: hovered
                    ToolTip.text: root.discoverState === "error" ? root.discoverError : "Live Geofabrik cross-check — click to refresh"
                    hoverEnabled: true
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
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "REGION"
                            MonoField { id: hostRegion; placeholderText: "region (e.g. germany, us/california)" }
                        }
                        PButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Host + registration tx"
                            onClicked: {
                                const args = { region: hostRegion.text.trim() }
                                if (registrar.text.length) args.registrar_hex = registrar.text
                                root.call("host", args)
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "OPT-OUT REGIONS"
                            MonoField { id: optOut; placeholderText: "opt-out regions (comma)" }
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Host bulk (all, small set first!)"
                            onClicked: root.call("host_bulk", { regions: "all" })
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Bulk with opt-out"
                            onClicked: root.call("host_bulk", {
                                regions: "all",
                                opt_out: optOut.text.split(",").map(s => s.trim()).filter(s => s.length).join(",")
                            })
                        }
                    }

                    // -- On-chain tx builders (wallet submits) --
                    SectionHead { numeral: "03"; heading: "LEZ transactions (the wallet submits these)" }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 12
                        FieldCell {
                            label: "REGION"
                            MonoField { id: txRegion; placeholderText: "region" }
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Register tx"
                            onClicked: root.call("register", {
                                region: txRegion.text.trim(),
                                registrar_hex: registrar.text
                            })
                        }
                        SButton {
                            Layout.alignment: Qt.AlignBottom
                            text: "Batch tx (region,cid pairs via catalog)"
                            onClicked: root.call("register_bulk", {
                                regions: txRegion.text.trim(),
                                registrar_hex: registrar.text
                            })
                        }
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
                                    visible: index < ListView.view.count - 1
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
