import QtQuick
import QtQuick.Controls

// A menu of Hof's actions drawn in the window, in the theme's colours:
// the tab bar's button opens one, and a right click in a pane another.
// Its items show the chord bound to each action, but leave the keys to
// the window's shortcuts, which would cancel out with a second of each.
Menu {
    id: menu

    // The chord shown beside an action; empty for none.
    property var keyOf: name => ""
    property color baseColor
    property color textColor
    property string fontFamily
    property real fontSize: 11

    readonly property color border: Qt.rgba(menu.textColor.r, menu.textColor.g, menu.textColor.b, 0.2)
    // The widest item, for the menu to be as wide.
    property real widest: 0
    readonly property color hover: Qt.rgba(menu.textColor.r, menu.textColor.g, menu.textColor.b, 0.12)

    // An item was chosen: the name of its action.
    signal chose(string name)

    padding: 4
    implicitWidth: Math.max(200, menu.widest) + menu.leftPadding + menu.rightPadding
    font.family: menu.fontFamily
    font.pointSize: menu.fontSize

    background: Rectangle {
        implicitWidth: 220
        color: Qt.darker(menu.baseColor, 1.25)
        border.color: menu.border
        radius: 6
    }
}
