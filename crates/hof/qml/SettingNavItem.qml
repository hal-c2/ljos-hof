import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Hof.Bricks

// A page in the settings' side list: its icon and name, filled when it
// is the page shown.
ItemDelegate {
    id: control

    property string iconName
    property bool current: false

    Layout.fillWidth: true
    implicitHeight: 32
    leftPadding: 10
    rightPadding: 10
    topPadding: 6
    bottomPadding: 6
    hoverEnabled: true
    font.pixelSize: 14
    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: current

    contentItem: RowLayout {
        spacing: 8

        ShellIcon {
            name: control.iconName
            size: 16
            color: control.current ? Theme.palette.color("text", "#e4e4e7") : Theme.palette.color("iconMuted", "#8b8b93")
        }
        Text {
            Layout.fillWidth: true
            text: control.text
            font: control.font
            color: control.current ? Theme.palette.color("text", "#e4e4e7") : Theme.palette.color("textMuted", "#8b8b93")
            elide: Text.ElideRight
        }
    }

    background: Rectangle {
        radius: Math.min(Theme.radius, 6)
        color: control.current ? Theme.palette.color("accentSurface", "#27272a") : control.hovered ? Qt.alpha(Theme.palette.color("accentSurface", "#27272a"), 0.5) : "transparent"
        border.color: Theme.palette.color("focus", "#3b82f6")
        border.width: control.visualFocus ? 1 : 0
    }
}
