import QtQuick
import QtQuick.Controls

// An item of the menu bar that runs one of Hof's actions, showing the
// first chord the config binds to it.
Action {
    id: action

    required property string name
    // The window that runs it.
    required property var runner

    objectName: "menu:" + action.name
    shortcut: action.runner.menuKey(action.name)
    onTriggered: action.runner.run(action.name)
}
