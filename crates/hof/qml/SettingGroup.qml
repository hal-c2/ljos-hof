import QtQuick
import QtQuick.Layouts
import Hof.Bricks

// Settings that go together, on a card under their heading.
ColumnLayout {
    id: group

    property string title
    // Beside the heading, at its end: a remove button, say.
    property alias trailing: trailingSlot.data
    default property alias rows: column.data

    Layout.fillWidth: true
    spacing: 6

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 4
        visible: group.title.length > 0

        Text {
            Layout.fillWidth: true
            text: group.title
            color: Theme.palette.color("textMuted", "#8b8b93")
            font.pixelSize: 12
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
        Item {
            id: trailingSlot

            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: column.implicitHeight + 8
        radius: Theme.radius
        color: Theme.palette.color("surfaceRaised", "#18181b")
        border.color: Theme.palette.color("border", "#27272a")
        border.width: 1

        ColumnLayout {
            id: column

            x: 16
            y: 4
            width: parent.width - 32
            spacing: 0
        }
    }
}
