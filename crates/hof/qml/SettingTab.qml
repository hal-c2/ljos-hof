import QtQuick
import QtQuick.Controls

// A page of the settings, named in the bar along their top: underlined
// in the accent colour when it is the one shown.
TabButton {
    id: control

    padding: 8

    contentItem: Text {
        text: control.text
        font: control.font
        color: control.palette.windowText
        opacity: control.checked ? 1 : 0.6
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }

    background: Item {
        implicitHeight: 34

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 2
            color: control.palette.highlight
            visible: control.checked
        }
    }
}
