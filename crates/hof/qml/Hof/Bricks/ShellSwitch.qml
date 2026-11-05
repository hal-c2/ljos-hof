import QtQuick
import QtQuick.Controls.Basic


// A switch in the theme's colours: the accent track when on,
// the `input` track and a muted thumb when off. Replaces the stock Switch,
// whose colours come from a system palette the theme does not reach.
Switch {
    id: control

    readonly property color trackOn: Theme.palette.color("accent", "#2563eb")
    readonly property color trackOff: Theme.palette.color("input", "#27272a")

    padding: 4
    spacing: 8
    font.pixelSize: Math.round(13 * Theme.fontScale)
    hoverEnabled: true
    opacity: enabled ? 1 : 0.64

    indicator: Rectangle {
        objectName: "track"

        implicitWidth: 30
        implicitHeight: 18
        x: control.text ? (control.mirrored ? control.width - width - control.rightPadding : control.leftPadding) : control.leftPadding + (control.availableWidth - width) / 2
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: height / 2
        color: control.checked ? control.trackOn : control.trackOff
        // Keyboard focus draws the focus ring; pointer focus stays quiet.
        border.color: Theme.palette.color("focus", "#3b82f6")
        border.width: control.visualFocus ? 2 : 0

        Rectangle {
            objectName: "thumb"

            x: 2 + control.visualPosition * (parent.width - width - 4)
            y: 2
            width: parent.height - 4
            height: width
            radius: width / 2
            // Off, the thumb is the muted text, so it stands out from a track
            // that is only a shade off the window's ground.
            color: control.checked ? Theme.palette.color("accentForeground", "#ffffff") : Theme.palette.color("textMuted", "#8b8b93")

            Behavior on x {
                enabled: !control.down
                NumberAnimation {
                    duration: 150
                    easing.type: Easing.OutCubic
                }
            }
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
