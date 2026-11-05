import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts


MenuItem {
    id: control

    property bool destructive: false
    property string iconName: ""
    // Keeps the icon's room when there is none, so that the labels of a
    // menu where some items have icons line up.
    property bool iconSpace: false
    // A trailing check mark, for menus that show the current choice.
    property bool current: false
    // The chord that does the same, muted at the end of the row; it is
    // only shown, the window's shortcuts hold the keys.
    property string shortcut: ""
    // A muted second part after the text (what tells two same-named entries
    // apart); it is elided before the text is.
    property string detail: ""
    // Why the item is disabled, drawn as a muted second line under the label.
    // A disabled row gets no hover, so a tooltip could never show it.
    property string reason: ""

    implicitHeight: reason.length > 0 ? 28 + reasonText.contentHeight + 4 : 28
    leftPadding: 8
    rightPadding: 8
    font.pixelSize: Math.round(14 * Theme.fontScale)
    hoverEnabled: true
    Accessible.description: [detail, reason].filter(part => part.length > 0).join(". ")

    contentItem: ColumnLayout {
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            spacing: 8

            ShellIcon {
                visible: control.iconName.length > 0 || control.iconSpace
                name: control.iconName
                size: 16
                color: Qt.alpha(Theme.palette.color("textMuted", "#8b8b93"), 0.8)
                Layout.alignment: Qt.AlignVCenter
            }

            Text {
                text: control.text
                font: control.font
                color: control.destructive ? Theme.palette.color("errorForeground", "#ef4444") : Theme.palette.color("text", "#e4e4e7")
                opacity: control.enabled ? 1 : 0.5
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                Layout.fillWidth: control.detail.length === 0
                Layout.minimumWidth: control.detail.length === 0 ? 0 : Math.min(implicitWidth, 80)
            }

            Text {
                objectName: "menuItemDetail"
                visible: control.detail.length > 0
                text: control.detail
                font: control.font
                color: Theme.palette.color("textMuted", "#8b8b93")
                opacity: control.enabled ? 1 : 0.5
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            Text {
                objectName: "menuItemShortcut"
                visible: control.shortcut.length > 0
                Layout.leftMargin: 24
                text: control.shortcut
                font: control.font
                color: Theme.palette.color("textMuted", "#8b8b93")
                opacity: control.enabled ? 1 : 0.5
                verticalAlignment: Text.AlignVCenter
            }

            ShellIcon {
                visible: control.subMenu !== null
                name: "chevron-right"
                size: 14
                color: Qt.alpha(Theme.palette.color("textMuted", "#8b8b93"), 0.8)
                Layout.alignment: Qt.AlignVCenter
            }

            ShellIcon {
                visible: control.current
                name: "check"
                size: 14
                color: Theme.palette.color("text", "#e4e4e7")
                Layout.alignment: Qt.AlignVCenter
            }
        }

        Text {
            id: reasonText

            objectName: "menuItemReason"
            visible: control.reason.length > 0
            Layout.fillWidth: true
            Layout.leftMargin: control.iconName.length > 0 || control.iconSpace ? 24 : 0
            Layout.bottomMargin: 4
            text: control.reason
            font.family: control.font.family
            font.pixelSize: Math.round(12 * Theme.fontScale)
            color: Theme.palette.color("textMuted", "#8b8b93")
            wrapMode: Text.Wrap
        }
    }

    background: Rectangle {
        radius: 6
        color: control.highlighted || control.hovered || control.visualFocus ? Theme.palette.color("accentSurface", "#27272a") : "transparent"
    }
}
