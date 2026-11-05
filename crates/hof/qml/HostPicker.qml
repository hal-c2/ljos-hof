import QtQuick
import QtQuick.Layouts
import Hof.App
import Hof.Bricks

// Picks the machine a new tab or split runs on: this one, or a name from
// `[hosts]`. Typing narrows the list, Up and Down move, Enter picks, and
// Escape goes back to the pane without opening anything. Each host shows
// how it is reached, `user@address:port`, beside its name.
Rectangle {
    id: root

    property var hosts: []

    // What the pick is for: "tab", "right" or "down".
    property string purpose: "tab"
    property int chosen: 0

    // Each host's address, user and port, by name.
    readonly property var details: {
        const by = {};
        for (const h of JSON.parse(Settings.hostDetails || "[]"))
            by[h.name] = h;
        return by;
    }

    // This machine first, as the empty name, then the hosts that have
    // what was typed in their name or their address.
    readonly property var shown: {
        const typed = field.text.toLowerCase();
        const all = [""].concat(root.hosts);
        return all.filter(h => typed.length === 0 || (h.length === 0 ? "this machine" : h + " " + root.reach(h)).toLowerCase().includes(typed));
    }

    signal picked(string host, string purpose)
    signal closed

    // How a host is reached, as ssh would be told: `user@address:port`;
    // empty when that is only its name.
    function reach(name) {
        const h = root.details[name];
        if (!h)
            return "";
        const how = (h.user ? h.user + "@" : "") + (h.ssh || name) + (h.port ? ":" + h.port : "");
        return how === name ? "" : how;
    }

    function open(purpose) {
        root.purpose = purpose;
        field.text = "";
        root.chosen = 0;
        root.visible = true;
        field.forceActiveFocus();
    }

    function close() {
        if (!root.visible)
            return;
        root.visible = false;
        root.closed();
    }

    function pick() {
        if (root.chosen < 0 || root.chosen >= root.shown.length)
            return;
        const host = root.shown[root.chosen];
        root.close();
        root.picked(host, root.purpose);
    }

    onShownChanged: root.chosen = Math.min(root.chosen, Math.max(0, root.shown.length - 1))

    visible: false
    width: Math.min(parent.width - 32, 400)
    height: column.implicitHeight + 16
    radius: Theme.radius
    color: Theme.palette.color("surfaceOverlay", "#18181b")
    border.color: Theme.palette.color("border", "#27272a")
    Accessible.role: Accessible.Dialog
    Accessible.name: root.purpose === "tab" ? "New tab on" : "Split on"

    ColumnLayout {
        id: column

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 8
        spacing: 4

        Text {
            Layout.fillWidth: true
            Layout.leftMargin: 2
            text: root.Accessible.name
            color: Theme.palette.color("textMuted", "#a1a1aa")
            font.pixelSize: Math.round(12 * Theme.fontScale)
            font.weight: Font.Medium
        }

        ShellTextField {
            id: field

            objectName: "hostPickerField"
            Layout.fillWidth: true
            placeholderText: "Host"
            Accessible.name: "Host"

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Escape) {
                    root.close();
                    event.accepted = true;
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    root.pick();
                    event.accepted = true;
                } else if (event.key === Qt.Key_Down || (event.key === Qt.Key_Tab && !(event.modifiers & Qt.ShiftModifier))) {
                    root.chosen = (root.chosen + 1) % Math.max(1, root.shown.length);
                    event.accepted = true;
                } else if (event.key === Qt.Key_Up || event.key === Qt.Key_Backtab) {
                    root.chosen = (root.chosen + Math.max(1, root.shown.length) - 1) % Math.max(1, root.shown.length);
                    event.accepted = true;
                }
            }
        }

        Repeater {
            model: root.shown

            delegate: Rectangle {
                id: entry

                required property string modelData
                required property int index
                readonly property bool here: entry.modelData.length === 0

                objectName: "hostChoice:" + entry.modelData
                Layout.fillWidth: true
                implicitHeight: 32
                radius: Math.max(0, Theme.radius - 2)
                color: entry.index === root.chosen || area.containsMouse ? Theme.palette.color("accentSurface", "#27272a") : "transparent"
                Accessible.role: Accessible.ListItem
                Accessible.name: entry.here ? "This machine" : entry.modelData
                Accessible.description: entry.here ? "" : root.reach(entry.modelData)
                Accessible.selected: entry.index === root.chosen
                Accessible.onPressAction: {
                    root.chosen = entry.index;
                    root.pick();
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 8

                    ShellIcon {
                        name: entry.here ? "monitor" : "server"
                        size: 14
                        color: Theme.palette.color("iconMuted", "#a1a1aa")
                    }

                    Text {
                        text: entry.here ? "This machine" : entry.modelData
                        color: Theme.palette.color("text", "#e4e4e7")
                        font.pixelSize: Math.round(13 * Theme.fontScale)
                        elide: Text.ElideRight
                        Layout.fillWidth: entry.here
                    }

                    Text {
                        visible: !entry.here
                        text: entry.here ? "" : root.reach(entry.modelData)
                        Layout.minimumWidth: 0
                        color: Theme.palette.color("textMuted", "#a1a1aa")
                        font.pixelSize: Math.round(12 * Theme.fontScale)
                        horizontalAlignment: Text.AlignRight
                        elide: Text.ElideMiddle
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    id: area

                    anchors.fill: parent
                    hoverEnabled: true
                    Accessible.ignored: true
                    onClicked: {
                        root.chosen = entry.index;
                        root.pick();
                    }
                }
            }
        }

        Text {
            Layout.fillWidth: true
            Layout.leftMargin: 8
            Layout.topMargin: 4
            Layout.bottomMargin: 4
            visible: root.shown.length === 0
            text: "No host by that name"
            color: Theme.palette.color("textMuted", "#a1a1aa")
            font.pixelSize: Math.round(13 * Theme.fontScale)
        }
    }
}
