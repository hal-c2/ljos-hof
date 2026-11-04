import QtQuick
import Hof

// A thin bar over a terminal's right edge: where the view is in the
// scrollback. Shown while there is scrollback and the view is off the
// bottom, or the pointer is on it; dragging it scrolls.
Item {
    id: root

    required property Terminal terminal
    property color color

    readonly property int total: Math.max(1, root.terminal.scrollTotal)
    readonly property bool scrollable: root.terminal.scrollTotal > root.terminal.scrollVisible

    width: 10
    visible: root.scrollable

    Rectangle {
        id: thumb

        x: root.width - width - 2
        width: area.containsMouse || area.pressed ? 6 : 3
        radius: width / 2
        height: Math.max(16, root.height * root.terminal.scrollVisible / root.total)
        y: (root.height - thumb.height) * root.terminal.scrollOffset / Math.max(1, root.total - root.terminal.scrollVisible)
        color: root.color
        opacity: area.containsMouse || area.pressed ? 0.6 : (root.terminal.atBottom ? 0 : 0.35)

        Behavior on opacity {
            NumberAnimation {
                duration: 120
            }
        }
    }

    MouseArea {
        id: area

        anchors.fill: parent
        hoverEnabled: true

        function scrollAt(y) {
            const room = Math.max(1, root.height - thumb.height);
            const fraction = Math.max(0, Math.min(1, (y - thumb.height / 2) / room));
            root.terminal.scrollTo(fraction * (root.total - root.terminal.scrollVisible));
        }

        onPressed: mouse => area.scrollAt(mouse.y)
        onPositionChanged: mouse => {
            if (area.pressed)
                area.scrollAt(mouse.y);
        }
    }
}
