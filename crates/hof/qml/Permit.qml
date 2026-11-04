import QtQuick
import Hof

// Asks whether the program may do something: read the clipboard, open a
// URL. Enter lets it this once, A always (for this pane), Escape not.
Rectangle {
    id: root

    required property Terminal terminal
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    property string question
    property string detail

    signal answered(bool allow, bool always)

    function ask(question, detail) {
        root.question = question;
        root.detail = detail;
        root.visible = true;
        root.forceActiveFocus();
    }

    function answer(allow, always) {
        root.visible = false;
        root.terminal.forceActiveFocus();
        root.answered(allow, always);
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
            text: root.question
            color: root.foreground
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            font.bold: true
            wrapMode: Text.Wrap
        }

        Text {
            width: parent.width
            text: root.detail
            visible: root.detail.length > 0
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
