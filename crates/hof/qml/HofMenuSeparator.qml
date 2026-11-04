import QtQuick
import QtQuick.Controls

// A line between the groups of a `HofMenu`.
MenuSeparator {
    id: line

    required property var owner

    topPadding: 4
    bottomPadding: 4
    leftPadding: 8
    rightPadding: 8

    contentItem: Rectangle {
        implicitWidth: 100
        implicitHeight: 1
        color: line.owner.border
    }
}
