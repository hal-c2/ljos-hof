import QtQuick
import QtQuick.Controls.Basic

// A slider in the theme's colours: the `input` groove, filled with the
// accent up to the handle, and a round handle that rings on focus.
Slider {
    id: control

    implicitHeight: 28
    hoverEnabled: true
    opacity: enabled ? 1 : 0.64

    background: Rectangle {
        x: control.leftPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        implicitWidth: 120
        implicitHeight: 4
        width: control.availableWidth
        height: implicitHeight
        radius: 2
        color: Theme.palette.color("input", "#27272a")

        Rectangle {
            width: control.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: Theme.palette.color("accent", "#2563eb")
        }
    }

    handle: Rectangle {
        x: control.leftPadding + control.visualPosition * (control.availableWidth - width)
        y: control.topPadding + (control.availableHeight - height) / 2
        implicitWidth: 16
        implicitHeight: 16
        radius: 8
        color: Theme.palette.color("text", "#e4e4e7")
        border.color: Theme.palette.color("focus", "#3b82f6")
        border.width: control.visualFocus ? 2 : 0
    }
}
