import QtQuick
import Hof

// The tabs, shown when there are two or more: click to select, middle
// click or × to close, + for a new one.
Item {
    id: root

    required property Workspace workspace
    property color background
    property color foreground
    property color border
    property string fontFamily
    // The title of the tab whose focused pane is `pane`.
    property var titleOf: pane => ""

    signal newTab
    signal closeTab(int index)

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

                width: Math.min(220, (root.width - add.width) / Math.max(1, root.workspace.tabCount))
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

                Text {
                    anchors.left: parent.left
                    anchors.right: close.left
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    text: tab.title
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

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: root.border
    }
}
