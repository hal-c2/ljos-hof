import QtQuick
import Hof

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

                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.MiddleButton
                    onClicked: mouse => {
                        if (mouse.button === Qt.MiddleButton)
                            root.closeTab(tab.index);
                        else
                            root.workspace.selectTab(tab.index);
                    }
                }

                Text {
                    id: close

                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: "×"
                    color: root.foreground
                    opacity: closeArea.containsMouse ? 1 : 0.4

                    MouseArea {
                        id: closeArea

                        anchors.fill: parent
                        anchors.margins: -4
                        hoverEnabled: true
                        onClicked: root.closeTab(tab.index)
                    }
                }
            }
        }
    }

    Text {
        id: add

        anchors.left: tabs.right
        anchors.verticalCenter: parent.verticalCenter
        width: 30
        horizontalAlignment: Text.AlignHCenter
        text: "+"
        color: root.foreground
        opacity: addArea.containsMouse ? 1 : 0.5

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

        Rectangle {
            anchors.fill: parent
            anchors.margins: 3
            radius: 4
            color: root.foreground
            opacity: menuArea.pressed ? 0.18 : menuArea.containsMouse ? 0.1 : 0
        }

        Column {
            anchors.centerIn: parent
            spacing: 3

            Repeater {
                model: 3

                delegate: Rectangle {
                    width: 14
                    height: 2
                    radius: 1
                    color: root.foreground
                    opacity: menuArea.containsMouse ? 0.9 : 0.6
                }
            }
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
