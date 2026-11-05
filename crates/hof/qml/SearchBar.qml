import QtQuick
import Hof

// Finding text in a terminal's screen and scrollback, as the user types.
// Enter goes to the next older match, Shift+Enter to the next newer one,
// and Escape or the cross closes the bar and hands the keys back to the
// terminal; so does going back to the terminal some other way.
Rectangle {
    id: root

    required property Terminal terminal
    property color background
    property color foreground
    property color border
    property string fontFamily
    property real fontSize: 11

    readonly property alias text: field.text

    signal closed

    function open() {
        root.visible = true;
        field.selectAll();
        field.forceActiveFocus();
        // Text left from last time is looked for again at once.
        if (field.text.length > 0)
            root.terminal.find(field.text);
    }

    function close() {
        if (!root.visible)
            return;
        root.visible = false;
        root.terminal.endFind();
        root.terminal.forceActiveFocus();
        root.closed();
    }

    visible: false
    width: Math.min(parent.width - 16, 360)
    height: row.implicitHeight + 12
    radius: 4
    color: root.background
    border.color: root.border

    Row {
        id: row

        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 8

        TextInput {
            id: field

            width: row.width - count.width - shut.width - 2 * row.spacing
            anchors.verticalCenter: parent.verticalCenter
            color: root.foreground
            selectionColor: root.border
            font.family: root.fontFamily
            font.pointSize: root.fontSize
            clip: true

            onTextEdited: root.terminal.find(field.text)
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Escape) {
                    root.close();
                    event.accepted = true;
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (event.modifiers & Qt.ShiftModifier)
                        root.terminal.findPrevious();
                    else
                        root.terminal.findNext();
                    event.accepted = true;
                }
            }

            Text {
                anchors.fill: parent
                visible: field.text.length === 0
                text: "Find"
                color: root.foreground
                opacity: 0.5
                font: field.font
            }
        }

        // "3 of 12", or nothing to say before anything is typed.
        Text {
            id: count

            anchors.verticalCenter: parent.verticalCenter
            text: {
                if (field.text.length === 0)
                    return "";
                if (root.terminal.searchTotal === 0)
                    return "none";
                return root.terminal.searchCurrent + " of " + root.terminal.searchTotal;
            }
            color: root.foreground
            opacity: 0.7
            font.family: root.fontFamily
            font.pointSize: root.fontSize
        }

        Text {
            id: shut
            objectName: "searchClose"

            anchors.verticalCenter: parent.verticalCenter
            text: "×"
            color: root.foreground
            opacity: shutArea.containsMouse ? 1 : 0.7
            font.pointSize: root.fontSize
            Accessible.role: Accessible.Button
            Accessible.name: "Close find"

            MouseArea {
                id: shutArea

                anchors.fill: parent
                anchors.margins: -4
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.close()
            }
        }
    }
}
