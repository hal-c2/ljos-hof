import QtQuick
import QtQuick.Controls.Basic


// A number field in the theme's colours: a bordered input
// with a minus stepper on the left and a plus on the right. Replaces the stock
// SpinBox, whose colours come from a system palette.
SpinBox {
    id: control

    readonly property color border: Theme.palette.color("input", "#27272a")
    readonly property color accentSurface: Theme.palette.color("accentSurface", "#27272a")

    implicitHeight: 30
    implicitWidth: 120
    padding: 0
    font.pixelSize: Math.round(13 * Theme.fontScale)
    hoverEnabled: true
    opacity: enabled ? 1 : 0.64

    contentItem: TextInput {
        z: 2
        text: control.displayText
        clip: width < implicitWidth
        font: control.font
        color: Theme.palette.color("text", "#e4e4e7")
        selectionColor: Theme.palette.color("accent", "#2563eb")
        selectedTextColor: Theme.palette.color("accentForeground", "#ffffff")
        horizontalAlignment: Qt.AlignHCenter
        verticalAlignment: Qt.AlignVCenter
        readOnly: !control.editable
        validator: control.validator
        inputMethodHints: control.inputMethodHints
    }

    up.indicator: Rectangle {
        objectName: "up"

        x: control.mirrored ? 0 : control.width - width
        y: 1
        width: 28
        height: control.height - 2
        radius: Math.max(0, control.background.radius - 1)
        color: control.up.pressed || control.up.hovered ? control.accentSurface : "transparent"
        opacity: control.value < control.to ? 1 : 0.4

        ShellIcon {
            anchors.centerIn: parent
            name: "plus"
            size: 14
            color: Theme.palette.color("text", "#e4e4e7")
        }
    }

    down.indicator: Rectangle {
        objectName: "down"

        x: control.mirrored ? control.width - width : 0
        y: 1
        width: 28
        height: control.height - 2
        radius: Math.max(0, control.background.radius - 1)
        color: control.down.pressed || control.down.hovered ? control.accentSurface : "transparent"
        opacity: control.value > control.from ? 1 : 0.4

        ShellIcon {
            anchors.centerIn: parent
            name: "minus"
            size: 14
            color: Theme.palette.color("text", "#e4e4e7")
        }
    }

    background: Rectangle {
        radius: Math.min(Theme.radius, 8)
        color: Theme.appearance === "dark" ? Qt.alpha(control.border, 0.32) : Theme.palette.color("canvas", "#ffffff")
        border.color: control.activeFocus ? Theme.palette.color("focus", "#3b82f6") : control.border
        border.width: 1
    }
}
