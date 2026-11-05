import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Hof.Bricks

// A page of the settings: its title and what it is for, then its groups,
// scrolled when they are more than the window holds.
Flickable {
    id: page

    property string title
    property string description
    default property alias groups: column.data

    implicitWidth: column.implicitWidth + 48
    implicitHeight: contentHeight
    clip: true
    contentHeight: column.implicitHeight + 48
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar {}

    ColumnLayout {
        id: column

        x: 24
        y: 20
        width: page.width - 48
        spacing: 20

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4

            Text {
                Layout.fillWidth: true
                text: page.title
                color: Theme.palette.color("text", "#e4e4e7")
                font.pixelSize: 20
                font.weight: Font.DemiBold
            }
            Text {
                Layout.fillWidth: true
                visible: page.description.length > 0
                text: page.description
                color: Theme.palette.color("textMuted", "#8b8b93")
                font.pixelSize: 13
                wrapMode: Text.Wrap
            }
        }
    }
}
