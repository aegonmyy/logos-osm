import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The consumer example's view.
//
// One dropdown of regions, one button, one result. The point is the shape:
// this module depends on the OSM SDK and nothing else, and every answer it
// shows came from a single SDK call.
Item {
    id: root

    readonly property var bk: logos.module("osm_consumer")
    readonly property string lastErr: bk ? bk.lastErr : ""
    readonly property var regionList: {
        if (!bk || !bk.regionsJson) return []
        try { return JSON.parse(bk.regionsJson) } catch (e) { return [] }
    }
    readonly property var resolved: {
        if (!bk || !bk.resolvedJson) return null
        try { return JSON.parse(bk.resolvedJson) } catch (e) { return null }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 10

        Label {
            text: "Map Regions"
            font.pixelSize: 20
            font.bold: true
        }
        Label {
            text: "Resolve a region to the CID the registry records for it. "
                  + "Read-only: this module hosts nothing and registers nothing."
            color: "#666"
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            ComboBox {
                id: picker
                Layout.preferredWidth: 260
                model: root.regionList.map(function (r) { return r.path })
                enabled: root.regionList.length > 0
            }
            Button {
                text: "Resolve"
                enabled: picker.currentIndex >= 0
                onClicked: {
                    if (!root.bk) return
                    root.bk.resolveRegion(picker.currentText)
                }
            }
        }

        Label {
            visible: root.lastErr.length > 0
            text: root.lastErr
            color: "#c0392b"
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }

        // The resolved entry: the CID the SDK returned, plus the audit
        // fields the registry stores beside it.
        Frame {
            visible: root.resolved !== null
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                spacing: 4
                Label { text: root.resolved ? root.resolved.region : ""; font.bold: true }
                Label { text: "mirrors: " + (root.resolved ? root.resolved.mirrors : 0) }
                Label { text: "registrars: " + (root.resolved ? root.resolved.registrars : 0) }
                Label {
                    visible: root.resolved && root.resolved.latest
                    text: root.resolved && root.resolved.latest
                          ? "cid: " + root.resolved.latest.cid : ""
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
                Label {
                    visible: root.resolved && root.resolved.latest
                    text: root.resolved && root.resolved.latest
                          ? "version: " + root.resolved.latest.version : ""
                }
            }
        }

        Item { Layout.fillHeight: true }
    }
}
