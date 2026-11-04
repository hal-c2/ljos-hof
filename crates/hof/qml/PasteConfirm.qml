import QtQuick
import Hof

// Asks before a paste that could run commands goes to the program: the
// text has a newline or control codes in it. Enter pastes, Escape doesn't.
Rectangle {
    id: root

    required property Terminal terminal
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    property string pending: ""
    readonly property int lineCount: root.pending.split("\n").length

    function ask(text) {
        root.pending = text;
        root.visible = true;
        root.forceActiveFocus();
    }

    function answer(paste) {
        const text = root.pending;
        root.pending = "";
        root.visible = false;
        root.terminal.forceActiveFocus();
        if (paste)
            root.terminal.forcePaste(text);
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
            root.answer(true);
            event.accepted = true;
        } else if (event.key === Qt.Key_Escape) {
            root.answer(false);
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
            text: root.lineCount > 1 ? "Paste " + root.lineCount + " lines? They may run as commands." : "Paste text with control codes in it?"
            color: root.foreground
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            font.bold: true
            wrapMode: Text.Wrap
        }

        // The start of it, so the user can see what they would run.
        Rectangle {
            width: parent.width
            height: preview.implicitHeight + 12
            color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.06)
            radius: 3

            Text {
                id: preview

                anchors.fill: parent
                anchors.margins: 6
                text: root.pending.length > 400 ? root.pending.slice(0, 400) + "…" : root.pending
                color: root.foreground
                font.family: root.fontFamily
                font.pointSize: root.fontSize
                maximumLineCount: 8
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }
        }

        Row {
            anchors.right: parent.right
            spacing: 8

            Repeater {
                model: [
                    {
                        label: "Cancel (Esc)",
                        paste: false
                    },
                    {
                        label: "Paste (Enter)",
                        paste: true
                    }
                ]

                delegate: Rectangle {
                    id: button

                    required property var modelData

                    width: label.implicitWidth + 20
                    height: label.implicitHeight + 10
                    radius: 3
                    color: button.modelData.paste ? Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15) : "transparent"
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
                        onClicked: root.answer(button.modelData.paste)
                    }
                }
            }
        }
    }
}
