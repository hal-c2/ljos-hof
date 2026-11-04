import QtQuick

// What was wrong with the config file, in a corner, until clicked away.
// Comes back when the file is read again with new problems.
Rectangle {
    id: root

    property var problems: []
    property color background
    property color foreground
    property bool dismissed: false

    onProblemsChanged: root.dismissed = false

    visible: root.problems.length > 0 && !root.dismissed
    width: Math.min(480, list.implicitWidth + 24)
    height: list.implicitHeight + 24
    radius: 6
    color: Qt.lighter(root.background, 1.4)
    border.color: Qt.rgba(0.9, 0.3, 0.3, 1)

    Column {
        id: list

        x: 12
        y: 12
        width: root.width - 24
        spacing: 4

        Text {
            text: "hof.toml"
            font.bold: true
            color: root.foreground
        }

        Repeater {
            model: root.problems

            delegate: Text {
                required property string modelData

                width: list.width
                text: modelData
                wrapMode: Text.Wrap
                color: root.foreground
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: root.dismissed = true
    }
}
