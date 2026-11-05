import QtQuick
import Hof.Bricks

// An item of a `HofMenu`: an action's name, and its chord muted at the
// right. It sets no shortcut of its own; see `HofMenu`.
ShellMenuItem {
    id: item

    required property var owner
    required property string name

    objectName: "menuItem:" + item.name
    shortcut: item.owner.keyOf(item.name)
    iconSpace: true

    // Once the menu has closed and given the keys back, so that what the
    // action opens (the find bar, the settings) keeps them.
    onTriggered: Qt.callLater(item.owner.chose, item.name)
}
