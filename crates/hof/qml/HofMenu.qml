import QtQuick
import Hof.Bricks

// A menu of Hof's actions drawn in the window, in the theme's colours:
// the tab bar's button opens one, and a right click in a pane another.
// Its items show the chord bound to each action, but leave the keys to
// the window's shortcuts, which would cancel out with a second of each.
ShellMenu {
    id: menu

    // The chord shown beside an action; empty for none.
    property var keyOf: name => ""

    // An item was chosen: the name of its action.
    signal chose(string name)
}
