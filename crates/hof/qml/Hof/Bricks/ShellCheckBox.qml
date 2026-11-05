import QtQuick
import QtQuick.Controls.Basic


// A checkbox in the theme's colours: an `input` outline on
// the canvas, the accent fill with its foreground mark when checked. Replaces
// the stock CheckBox, whose colours come from a system palette.
CheckBox {
    id: control

    padding: 4
    spacing: 8
    font.pixelSize: Math.round(13 * Theme.fontScale)
    hoverEnabled: true
    opacity: enabled ? 1 : 0.64

    indicator: Rectangle {
        objectName: "box"

        implicitWidth: 16
        implicitHeight: 16
        x: control.text ? (control.mirrored ? control.width - width - control.rightPadding : control.leftPadding) : control.leftPadding + (control.availableWidth - width) / 2
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: 4
        color: control.checkState === Qt.Unchecked ? Theme.palette.color("canvas", "#ffffff") : Theme.palette.color("accent", "#2563eb")
        border.color: control.visualFocus ? Theme.palette.color("focus", "#3b82f6") : control.checkState === Qt.Unchecked ? Theme.palette.color("input", "#27272a") : Theme.palette.color("accent", "#2563eb")
        border.width: control.visualFocus ? 2 : 1

        ShellIcon {
            anchors.centerIn: parent
            visible: control.checkState !== Qt.Unchecked
            name: control.checkState === Qt.PartiallyChecked ? "minus" : "check"
            size: 12
            strokeWidth: 3
            color: Theme.palette.color("accentForeground", "#ffffff")
        }
    }

    contentItem: Text {
        leftPadding: control.indicator && !control.mirrored ? control.indicator.width + control.spacing : 0
        rightPadding: control.indicator && control.mirrored ? control.indicator.width + control.spacing : 0
        text: control.text
        font: control.font
        color: control.palette.windowText
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    palette.windowText: Theme.palette.color("text", "#e4e4e7")
}
