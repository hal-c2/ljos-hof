import QtQuick

// Picks the machine a new tab or split runs on: this one, or a name from
// `[hosts]`. Typing narrows the list, Up and Down move, Enter picks, and
// Escape goes back to the pane without opening anything.
Rectangle {
    id: root

    property var hosts: []
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    // What the pick is for: "tab", "right" or "down".
    property string purpose: "tab"
    property int chosen: 0

    // This machine first, as the empty name, then the hosts that have
    // what was typed in their name.
    readonly property var shown: {
        const typed = field.text.toLowerCase();
        const all = [""].concat(root.hosts);
        return all.filter(h => typed.length === 0 || (h.length === 0 ? "this machine" : h).toLowerCase().includes(typed));
    }

    signal picked(string host, string purpose)
    signal closed

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
    width: Math.min(parent.width - 32, 360)
    height: column.implicitHeight + 16
    radius: 6
    color: root.background
    border.color: root.border

    Column {
        id: column

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 8
        spacing: 4

        Text {
            width: parent.width
            text: root.purpose === "tab" ? "New tab on" : "Split on"
            color: root.foreground
            opacity: 0.7
            font.family: root.fontFamily
            font.pointSize: root.fontSize
        }

        TextInput {
            id: field

            width: parent.width
            color: root.foreground
            selectionColor: root.border
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            clip: true

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

            Text {
                anchors.fill: parent
                visible: field.text.length === 0
                text: "Host"
                color: root.foreground
                opacity: 0.5
                font: field.font
            }
        }

        Repeater {
            model: root.shown

            delegate: Rectangle {
                id: entry

                required property string modelData
                required property int index

                width: column.width
                height: name.implicitHeight + 6
                radius: 3
                color: entry.index === root.chosen ? root.border : "transparent"

                Text {
                    id: name

                    anchors.left: parent.left
                    anchors.leftMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    text: entry.modelData.length === 0 ? "this machine" : entry.modelData
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pointSize: root.fontSize
                    font.italic: entry.modelData.length === 0
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        root.chosen = entry.index;
                        root.pick();
                    }
                }
            }
        }

        Text {
            width: parent.width
            visible: root.shown.length === 0
            text: "No host by that name"
            color: root.foreground
            opacity: 0.5
            font.family: root.fontFamily
            font.pointSize: root.fontSize
        }
    }
}
