import QtQuick
import QtQuick.Layouts
import Hof.Bricks

// One setting: what it is, and a line on what it does, on the left; its
// control, the row's other child, on the right.
RowLayout {
    id: row

    property string title
    property string description

    Layout.fillWidth: true
    Layout.minimumHeight: 52
    spacing: 16

    ColumnLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        Layout.bottomMargin: 8
        spacing: 2

        Text {
            Layout.fillWidth: true
            text: row.title
            color: Theme.palette.color("text", "#e4e4e7")
            font.pixelSize: 14
            wrapMode: Text.Wrap
        }
        Text {
            Layout.fillWidth: true
            visible: row.description.length > 0
            text: row.description
            color: Theme.palette.color("textMuted", "#8b8b93")
            font.pixelSize: 12
            wrapMode: Text.Wrap
        }
    }
}
