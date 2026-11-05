import QtQuick
import QtQuick.Controls.Basic


// A hairline between groups of ShellMenuItems.
MenuSeparator {
    padding: 0
    topPadding: 4
    bottomPadding: 4

    contentItem: Rectangle {
        implicitHeight: 1
        color: Qt.alpha(Theme.palette.color("text", "#e4e4e7"), 0.1)
    }
}
