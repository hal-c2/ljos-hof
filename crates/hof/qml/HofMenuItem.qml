import QtQuick
import QtQuick.Controls

// An item of a `HofMenu`: an action's name, and its chord dimmed at the
// right. It sets no shortcut of its own; see `HofMenu`.
MenuItem {
    id: item

    required property var owner
    required property string name
    readonly property string key: item.owner.keyOf(item.name)

    objectName: "menuItem:" + item.name
    leftPadding: 12
    rightPadding: 12
    topPadding: 5
    bottomPadding: 5
    font: item.owner.font

    contentItem: Item {
        implicitWidth: label.implicitWidth + (item.key.length > 0 ? 32 + chord.implicitWidth : 0)
        implicitHeight: label.implicitHeight

        Text {
            id: label

            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            text: item.text
            font: item.font
            color: item.owner.textColor
            opacity: item.enabled ? 1 : 0.4
        }

        Text {
            id: chord

            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            text: item.key
            font: item.font
            color: item.owner.textColor
            opacity: item.enabled ? 0.5 : 0.25
        }
    }

    background: Rectangle {
        implicitHeight: 26
        radius: 4
        color: item.enabled && (item.highlighted || item.down) ? item.owner.hover : "transparent"
    }

    // Once the menu has closed and given the keys back, so that what the
    // action opens (the find bar, the settings) keeps them.
    onImplicitWidthChanged: item.owner.widest = Math.max(item.owner.widest, item.implicitWidth)
    Component.onCompleted: item.owner.widest = Math.max(item.owner.widest, item.implicitWidth)
    onTriggered: Qt.callLater(item.owner.chose, item.name)
}
