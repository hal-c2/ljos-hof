import QtQuick
import Hof

// Asks before the program reads the clipboard (OSC 52), which may hold a
// password or anything else the user copied. The program waits for the
// answer. Enter lets it this once, A always (for this pane), Escape not.
Rectangle {
    id: root

    required property Terminal terminal
    // The `[hosts]` machine the program runs on; empty for this one.
    property string host
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    property string clipboard: "clipboard"

    function ask(clipboard) {
        root.clipboard = clipboard;
        root.visible = true;
        root.forceActiveFocus();
    }

    function answer(allow, always) {
        root.visible = false;
        root.terminal.forceActiveFocus();
        root.terminal.answerClipboard(allow, always);
    }

    visible: false
    width: Math.min(parent.width - 32, 520)
    height: column.implicitHeight + 24
    radius: 6
    color: root.background
    border.color: root.border
    focus: root.visible

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            root.answer(true, false);
            event.accepted = true;
        } else if (event.key === Qt.Key_A) {
            root.answer(true, true);
            event.accepted = true;
        } else if (event.key === Qt.Key_Escape) {
            root.answer(false, false);
            event.accepted = true;
        }
    }

    Column {
        id: column

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 12
        spacing: 8

        Text {
            width: parent.width
            text: (root.host.length > 0 ? "A program on " + root.host : "The program") + " wants to read the " + (root.clipboard === "selection" ? "selection" : "clipboard") + "."
            color: root.foreground
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            font.bold: true
            wrapMode: Text.Wrap
        }

        Text {
            width: parent.width
            text: "It would see whatever was copied last, here or in any other program."
            color: root.foreground
            opacity: 0.7
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            wrapMode: Text.Wrap
        }

        Row {
            anchors.right: parent.right
            spacing: 8

            Repeater {
                model: [
                    {
                        label: "Deny (Esc)",
                        allow: false,
                        always: false
                    },
                    {
                        label: "Always (A)",
                        allow: true,
                        always: true
                    },
                    {
                        label: "Allow (Enter)",
                        allow: true,
                        always: false
                    }
                ]

                delegate: Rectangle {
                    id: button

                    required property var modelData

                    width: label.implicitWidth + 20
                    height: label.implicitHeight + 10
                    radius: 3
                    color: button.modelData.allow && !button.modelData.always ? Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15) : "transparent"
                    border.color: root.border

                    Text {
                        id: label

                        anchors.centerIn: parent
                        text: button.modelData.label
                        color: root.foreground
                        font.family: root.fontFamily
                        font.pointSize: root.fontSize
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: root.answer(button.modelData.allow, button.modelData.always)
                    }
                }
            }
        }
    }
}
