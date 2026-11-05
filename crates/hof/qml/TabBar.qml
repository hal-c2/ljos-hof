import QtQuick
import Hof
import Hof.Bricks

// The tabs, shown when there are two or more or when the menu is here:
// click to select, middle click or × to close, + for a new one, and the
// button at the right for the window's menu.
Item {
    id: root

    required property Workspace workspace
    property color background
    property color foreground
    property color border
    property string fontFamily
    // The title of the tab whose focused pane is `pane`.
    property var titleOf: pane => ""
    // How the agents of tab `index` are, the one most in need first:
    // "waiting", "working", "idle" or "none".
    property var agentStateOf: index => "none"
    property color waitingColor: "orange"
    property color workingColor: "steelblue"
    property color idleColor: "seagreen"
    // Whether the window's menu opens from a button here.
    property bool menuButton: false

    signal newTab
    signal closeTab(int index)
    // The menu button was pressed: open the menu under `from`.
    signal menuRequested(Item from)

    implicitHeight: 30

    Rectangle {
        anchors.fill: parent
        color: Qt.darker(root.background, 1.15)
    }

    Row {
        id: tabs

        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom

        Repeater {
            model: root.workspace.tabCount

            delegate: Item {
                id: tab

                required property int index
                readonly property bool current: tab.index === root.workspace.currentTab
                readonly property string title: {
                    void root.workspace.revision;
                    return root.titleOf(root.workspace.tabFocus(tab.index));
                }
                readonly property string agentState: {
                    void root.workspace.revision;
                    return root.agentStateOf(tab.index);
                }

                width: Math.min(220, (root.width - add.width - menu.width) / Math.max(1, root.workspace.tabCount))
                height: tabs.height

                Rectangle {
                    anchors.fill: parent
                    color: tab.current ? root.background : "transparent"
                }

                Rectangle {
                    anchors.right: parent.right
                    width: 1
                    height: parent.height
                    color: root.border
                }

                // A dot for the tab's agents.
                Text {
                    id: mark

                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    width: tab.agentState === "none" ? 0 : implicitWidth + 6
                    visible: tab.agentState !== "none"
                    text: tab.agentState === "waiting" ? "●" : tab.agentState === "working" ? "◌" : "○"
                    color: tab.agentState === "waiting" ? root.waitingColor : tab.agentState === "working" ? root.workingColor : root.idleColor
                    font.family: root.fontFamily
                }

                Text {
                    anchors.left: mark.right
                    anchors.right: close.left
                    anchors.verticalCenter: parent.verticalCenter
                    text: tab.title
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    color: root.foreground
                    opacity: tab.current ? 1 : 0.6
                    font.family: root.fontFamily
                }

                // The tab itself, beside its close button rather than
                // around it. Tabs, like the buttons here, are for the
                // pointer: the keyboard has its bindings for them.
                MouseArea {
                    id: tabArea

                    anchors.left: parent.left
                    anchors.right: close.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    Accessible.role: Accessible.PageTab
                    Accessible.name: tab.title.length > 0 ? tab.title : "Tab " + (tab.index + 1)
                    Accessible.selected: tab.current
                    Accessible.focusable: false
                    Accessible.onPressAction: root.workspace.selectTab(tab.index)
                    acceptedButtons: Qt.LeftButton | Qt.MiddleButton
                    onClicked: mouse => {
                        if (mouse.button === Qt.MiddleButton)
                            root.closeTab(tab.index);
                        else
                            root.workspace.selectTab(tab.index);
                    }
                }

                Item {
                    id: close

                    anchors.right: parent.right
                    anchors.rightMargin: 3
                    anchors.verticalCenter: parent.verticalCenter
                    width: 24
                    height: 24
                    Accessible.role: Accessible.Button
                    Accessible.name: "Close " + tabArea.Accessible.name
                    Accessible.focusable: false
                    Accessible.onPressAction: root.closeTab(tab.index)

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: 3
                        radius: 4
                        color: root.foreground
                        opacity: closeArea.containsMouse ? 0.12 : 0
                    }

                    ShellIcon {
                        anchors.centerIn: parent
                        name: "x"
                        size: 12
                        color: root.foreground
                        opacity: closeArea.containsMouse || tab.current ? 0.9 : 0.6
                    }

                    MouseArea {
                        id: closeArea

                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.closeTab(tab.index)
                    }
                }
            }
        }
    }

    Item {
        id: add

        anchors.left: tabs.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 30
        Accessible.role: Accessible.Button
        Accessible.name: "New tab"
        Accessible.focusable: false
        Accessible.onPressAction: root.newTab()

        Rectangle {
            anchors.fill: parent
            anchors.margins: 3
            radius: 4
            color: root.foreground
            opacity: addArea.containsMouse ? 0.1 : 0
        }

        ShellIcon {
            anchors.centerIn: parent
            name: "plus"
            size: 14
            color: root.foreground
            opacity: addArea.containsMouse ? 0.95 : 0.7
        }

        MouseArea {
            id: addArea

            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.newTab()
        }
    }

    // Three bars, the usual sign of a menu where a window has no menu
    // bar.
    Item {
        id: menu
        objectName: "menuButton"

        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.menuButton ? 36 : 0
        visible: root.menuButton
        Accessible.role: Accessible.ButtonMenu
        Accessible.name: "Menu"
        Accessible.focusable: false
        Accessible.onPressAction: root.menuRequested(menu)

        Rectangle {
            anchors.fill: parent
            anchors.margins: 3
            radius: 4
            color: root.foreground
            opacity: menuArea.pressed ? 0.18 : menuArea.containsMouse ? 0.1 : 0
        }

        ShellIcon {
            anchors.centerIn: parent
            name: "menu"
            size: 16
            color: root.foreground
            opacity: menuArea.containsMouse ? 0.95 : 0.7
        }

        MouseArea {
            id: menuArea

            anchors.fill: parent
            hoverEnabled: true
            onPressed: root.menuRequested(menu)
        }
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: root.border
    }
}
