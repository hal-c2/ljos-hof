import QtQuick
import QtQuick.Controls.Basic


// A popup menu; use ShellMenuItem for entries.
Menu {
    id: control

    padding: 4
    // Wide enough for its longest row, between the usual 200 and 360.
    implicitWidth: Math.max(200, Math.min(360, widest() + leftPadding + rightPadding))

    function widest() {
        let widest = 0;
        for (let i = 0; i < count; ++i) {
            widest = Math.max(widest, itemAt(i).implicitWidth);
        }
        return widest;
    }

    // The row a submenu is reached through.
    delegate: ShellMenuItem {}

    enter: Transition {
        NumberAnimation {
            property: "opacity"
            from: 0
            to: 1
            duration: 120
            easing.type: Easing.OutCubic
        }
    }

    exit: Transition {
        NumberAnimation {
            property: "opacity"
            from: 1
            to: 0
            duration: 90
            easing.type: Easing.InCubic
        }
    }

    background: Rectangle {
        radius: Math.min(Theme.radius, 10)
        color: Theme.palette.color("surfaceOverlay", "#18181b")
        border.color: Qt.alpha(Theme.palette.color("text", "#e4e4e7"), 0.1)
        border.width: 1
    }
}
