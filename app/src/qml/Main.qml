import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    // Backend: the OsmApp replica (RemoteObjects). Its invokeOp(name,
    // argsJson) forwards to the core osm module, which dlopens
    // liblogos_osm.so and runs the Rust SDK.
    readonly property var    bk:          logos.module("osm_app")
    readonly property string osmVersion:  bk ? bk.osmVersion : ""
    readonly property string lastErr:     bk ? bk.lastErr    : ""

    // Call an OSM op and return the parsed JSON object, or {ok:false}.
    function call(op, args) {
        if (!root.bk) return { ok: false, error: "osm_app module not loaded" }
        const s = root.bk.invokeOp(op, JSON.stringify(args || {}))
        try { return JSON.parse(s) } catch (e) { return { ok: false, error: s } }
    }

    function pretty(obj) { return JSON.stringify(obj, null, 2) }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 10

        // ---- Header ----
        RowLayout {
            spacing: 8
            Label { text: "OpenStreetMap on Logos"; font.pixelSize: 22; font.bold: true }
            Item { Layout.fillWidth: true }
            Label { text: "Core:"; color: "#555" }
            Label { text: root.osmVersion.length ? root.osmVersion : "—" }
        }
        Label {
            visible: root.lastErr.length > 0
            color: "#c0392b"
            wrapMode: Text.Wrap
            text: root.lastErr
            Layout.fillWidth: true
        }

        TabBar {
            id: tabs
            Layout.fillWidth: true
            TabButton { text: "Host / Registrar" }
            TabButton { text: "Regions" }
            TabButton { text: "Consumer" }
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: tabs.currentIndex

            // ============ HOST / REGISTRAR ============
            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                ColumnLayout {
                    width: parent.width
                    spacing: 12

                    // -- Open (config) --
                    Label { text: "Connect"; font.bold: true }
                    GridLayout {
                        columns: 2
                        Layout.fillWidth: true
                        Label { text: "Storage (Codex) URL:" }
                        TextField { id: storageUrl; Layout.fillWidth: true; text: "http://127.0.0.1:8080" }
                        Label { text: "Geofabrik base URL:" }
                        TextField { id: geofabrikUrl; Layout.fillWidth: true; text: "https://download.geofabrik.de" }
                        Label { text: "Cache dir:" }
                        TextField { id: cacheDir; Layout.fillWidth: true; text: "osm-cache" }
                        Label { text: "Registrar (hex, 32B):" }
                        TextField { id: registrar; Layout.fillWidth: true; placeholderText: "registrar account id" }
                    }
                    RowLayout {
                        Button {
                            text: "Open"
                            onClicked: {
                                const args = {
                                    state_path: "osm-state.json",
                                    storage_url: storageUrl.text,
                                    geofabrik_url: geofabrikUrl.text,
                                    cache_dir: cacheDir.text
                                }
                                if (registrar.text.length) args.registrar_hex = registrar.text
                                out.text = root.pretty(root.call("open", args))
                            }
                        }
                        Button {
                            text: "Discover (live cross-check)"
                            onClicked: out.text = root.pretty(root.call("discover"))
                        }
                        Button {
                            text: "Region set"
                            onClicked: out.text = root.pretty(root.call("regions"))
                        }
                    }

                    // -- Host --
                    Label { text: "Host (download → verify MD5 → Logos Storage)"; font.bold: true }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField { id: hostRegion; placeholderText: "region (e.g. germany, us/california)"; Layout.fillWidth: true }
                        Button {
                            text: "Host + registration tx"
                            onClicked: {
                                const args = { region: hostRegion.text.trim() }
                                if (registrar.text.length) args.registrar_hex = registrar.text
                                out.text = root.pretty(root.call("host", args))
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Button {
                            text: "Host bulk (all, small set first!)"
                            onClicked: out.text = root.pretty(root.call("host_bulk", { regions: "all" }))
                        }
                        TextField { id: optOut; placeholderText: "opt-out regions (comma)" }
                        Button {
                            text: "Bulk with opt-out"
                            onClicked: out.text = root.pretty(root.call("host_bulk", {
                                regions: "all",
                                opt_out: optOut.text.split(",").map(s => s.trim()).filter(s => s.length).join(",")
                            }))
                        }
                    }

                    // -- On-chain tx builders (wallet submits) --
                    Label { text: "LEZ transactions (the wallet submits these)"; font.bold: true }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField { id: txRegion; placeholderText: "region"; Layout.fillWidth: true }
                        Button {
                            text: "Register tx"
                            onClicked: out.text = root.pretty(root.call("register", {
                                region: txRegion.text.trim(),
                                registrar_hex: registrar.text
                            }))
                        }
                        Button {
                            text: "Batch tx (region,cid pairs via catalog)"
                            onClicked: out.text = root.pretty(root.call("register_bulk", {
                                regions: txRegion.text.trim(),
                                registrar_hex: registrar.text
                            }))
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField { id: ownerHex; placeholderText: "owner (hex, 32B)"; Layout.fillWidth: true }
                        Button {
                            text: "Init tx"
                            onClicked: out.text = root.pretty(root.call("init", { owner_hex: ownerHex.text }))
                        }
                    }

                    // -- Output --
                    Label { text: "Result"; font.bold: true }
                    TextArea {
                        id: out
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Layout.preferredHeight: 220
                        readOnly: true
                        font.family: "Monospace"
                        wrapMode: TextArea.Wrap
                    }
                }
            }

            // ============ REGIONS ============
            // The closed set, with each region's on-chain status. The set is
            // static (the prize froze it), so the list renders without a
            // network call; the per-region status is read when a row is
            // clicked, because a status costs one chain read.
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    Label {
                        text: "Regions (" + regionModel.count + " in the set)"
                        font.bold: true
                    }
                    Item { Layout.fillWidth: true }
                    Button {
                        text: "Load set"
                        onClicked: {
                            const r = root.call("regions")
                            regionModel.clear()
                            if (r.ok && r.result && r.result.regions) {
                                for (const g of r.result.regions) {
                                    regionModel.append({
                                        path: g.path,
                                        name: g.name,
                                        level: g.level,
                                        status: "unknown",
                                        detail: ""
                                    })
                                }
                            } else {
                                outR.text = root.pretty(r)
                            }
                        }
                    }
                }

                Label {
                    text: "Click a region to read its registry entry from the chain."
                    color: "#666"
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: ListModel { id: regionModel }
                    delegate: Rectangle {
                        width: ListView.view.width
                        height: 34
                        color: index % 2 === 0 ? "transparent" : "#00000008"
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 6
                            anchors.rightMargin: 6
                            spacing: 8
                            Label {
                                text: model.status === "hosted" ? "●"
                                      : (model.status === "not-hosted" ? "○" : "·")
                                color: model.status === "hosted" ? "#2e7d32" : "#9e9e9e"
                                Layout.preferredWidth: 14
                            }
                            Label {
                                text: model.path
                                Layout.preferredWidth: 200
                                elide: Text.ElideRight
                            }
                            Label {
                                text: model.status === "hosted"
                                      ? ("v" + model.detail)
                                      : (model.status === "not-hosted" ? "not hosted" : "")
                                color: "#555"
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }
                        }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const r = root.call("lookup", { region: model.path })
                                if (r.ok && r.result) {
                                    const latest = r.result.latest
                                    regionModel.setProperty(index, "status", "hosted")
                                    regionModel.setProperty(index, "detail",
                                        latest ? String(latest.version) : "?")
                                    outR.text = root.pretty(r)
                                } else {
                                    regionModel.setProperty(index, "status", "not-hosted")
                                    regionModel.setProperty(index, "detail", "")
                                    outR.text = r.error || "not registered"
                                }
                            }
                        }
                    }
                }

                Label { text: "Result"; font.bold: true }
                TextArea {
                    id: outR
                    Layout.fillWidth: true
                    Layout.preferredHeight: 120
                    readOnly: true
                    font.family: "Monospace"
                    wrapMode: TextArea.Wrap
                }
            }

            // ============ CONSUMER ============
            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                ColumnLayout {
                    width: parent.width
                    spacing: 12

                    Label { text: "Fetch a registered snapshot (storage → verify → import)"; font.bold: true }
                    GridLayout {
                        columns: 2
                        Layout.fillWidth: true
                        Label { text: "Region:" }
                        TextField { id: fRegion; Layout.fillWidth: true; text: "germany" }
                        Label { text: "CID:" }
                        TextField { id: fCid; Layout.fillWidth: true; placeholderText: "from the registry" }
                        Label { text: "Published MD5 (hex):" }
                        TextField { id: fMd5; Layout.fillWidth: true; placeholderText: "from the registry" }
                    }
                    Button {
                        text: "Fetch + import"
                        onClicked: out2.text = root.pretty(root.call("fetch", {
                            region: fRegion.text.trim(),
                            cid: fCid.text.trim(),
                            md5_hex: fMd5.text.trim()
                        }))
                    }

                    Label { text: "Import a local PBF"; font.bold: true }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField { id: impRegion; placeholderText: "region"; text: "germany" }
                        TextField { id: impPath; placeholderText: "/path/to/region-latest.osm.pbf"; Layout.fillWidth: true }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Button {
                            text: "Import (validate + catalog)"
                            onClicked: out2.text = root.pretty(root.call("import", {
                                region: impRegion.text.trim(),
                                pbf: impPath.text.trim()
                            }))
                        }
                        Button {
                            // The full local-import workflow: verify the file
                            // against Geofabrik's published MD5, store in Logos
                            // Storage, and print the registration tx.
                            text: "Verify + store + register tx"
                            onClicked: out2.text = root.pretty(root.call("import", {
                                region: impRegion.text.trim(),
                                pbf: impPath.text.trim(),
                                store: true,
                                registrar_hex: registrar.text.trim()
                            }))
                        }
                    }

                    Label { text: "Update check"; font.bold: true }
                    RowLayout {
                        Layout.fillWidth: true
                        TextField { id: updRegion; placeholderText: "region"; text: "germany" }
                        Button {
                            text: "Check for newer snapshot"
                            onClicked: out2.text = root.pretty(root.call("update", {
                                region: updRegion.text.trim()
                            }))
                        }
                    }

                    Button {
                        text: "Local catalog"
                        onClicked: out2.text = root.pretty(root.call("catalog"))
                    }

                    Label { text: "Result"; font.bold: true }
                    TextArea {
                        id: out2
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        Layout.preferredHeight: 220
                        readOnly: true
                        font.family: "Monospace"
                        wrapMode: TextArea.Wrap
                    }
                }
            }
        }
    }
}
