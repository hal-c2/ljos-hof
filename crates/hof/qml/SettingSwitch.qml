import QtQuick
import QtQuick.Controls

// A switch of the settings: a track that fills with the accent colour
// when on, so on and off read apart in any theme.
Switch {
    id: control

    padding: 4

    indicator: Rectangle {
        implicitWidth: 38
        implicitHeight: 22
        x: control.leftPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: height / 2
        color: control.checked ? control.palette.highlight : control.palette.button
        border.color: control.visualFocus ? control.palette.highlight : control.palette.mid
        border.width: control.visualFocus ? 2 : 1

        Rectangle {
            x: control.checked ? parent.width - width - 3 : 3
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: 8
            color: control.checked ? control.palette.highlightedText : control.palette.windowText
            opacity: control.checked ? 1 : 0.7

            Behavior on x {
                NumberAnimation {
                    duration: 100
                }
            }
        }
    }
}
