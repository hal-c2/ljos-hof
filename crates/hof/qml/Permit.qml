import QtQuick
import Hof

// Asks whether the program may do something: read the clipboard, open a
// URL. Ctrl+Enter lets it this once, Ctrl+Shift+A always (for this pane),
// Escape not. Approval needs an intentional chord or click, never a stray key.
Rectangle {
    id: root

    required property Terminal terminal
    property bool active: true
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    property string question
    property string detail
    property Item previousFocus: null

    signal answered(bool allow, bool always)

    function ask(question, detail) {
        root.question = question;
        root.detail = detail;
        if (root.active) {
            root.previousFocus = root.Window.window.activeFocusItem;
            root.forceActiveFocus();
        }
        // The prompt with the keys must be the one the user can read.
        root.z = 4;
        root.visible = true;
    }

    function answer(allow, always) {
        root.z = 3;
        root.visible = false;
        if (root.active) {
            if (root.previousFocus !== null && root.previousFocus.visible)
                root.previousFocus.forceActiveFocus();
            else
                root.terminal.forceActiveFocus();
        }
        root.previousFocus = null;
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
        if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && event.modifiers === Qt.ControlModifier) {
            root.answer(true, false);
            event.accepted = true;
        } else if (event.key === Qt.Key_A && event.modifiers === (Qt.ControlModifier | Qt.ShiftModifier)) {
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
            textFormat: Text.PlainText
            font.bold: true
            wrapMode: Text.Wrap
        }

        Text {
            objectName: "permitDetail"
            width: parent.width
            text: root.detail
            visible: root.detail.length > 0
            color: root.foreground
            opacity: 0.7
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            textFormat: Text.PlainText
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
                        label: "Always (Ctrl+Shift+A)",
                        allow: true,
                        always: true
                    },
                    {
                        label: "Allow (Ctrl+Enter)",
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
            textFormat: Text.PlainText
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
