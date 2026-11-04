import QtQuick
import Hof

// The line between the two halves of a split; drag it to share the space
// anew. `rx`..`rh` are the whole split's, as fractions of the area.
Item {
    id: root

    required property string path
    required property bool vertical
    required property real ratio
    required property real rx
    required property real ry
    required property real rw
    required property real rh
    required property Item area
    required property Workspace workspace
    property color color

    // Halves side by side have an upright line between them.
    readonly property bool upright: !root.vertical
    readonly property real grip: 6
    readonly property real left: root.rx * root.area.width
    readonly property real top: root.ry * root.area.height
    readonly property real spanWidth: root.rw * root.area.width
    readonly property real spanHeight: root.rh * root.area.height

    x: Math.round(root.upright ? root.left + root.ratio * root.spanWidth - root.grip / 2 : root.left)
    y: Math.round(root.upright ? root.top : root.top + root.ratio * root.spanHeight - root.grip / 2)
    width: root.upright ? root.grip : Math.round(root.spanWidth)
    height: root.upright ? Math.round(root.spanHeight) : root.grip
    z: 1

    Rectangle {
        anchors.centerIn: parent
        width: root.upright ? 1 : parent.width
        height: root.upright ? parent.height : 1
        color: root.color
    }

    MouseArea {
        id: drag

        anchors.fill: parent
        cursorShape: root.upright ? Qt.SplitHCursor : Qt.SplitVCursor

        onPositionChanged: mouse => {
            if (!drag.pressed)
                return;
            const at = drag.mapToItem(root.area, mouse.x, mouse.y);
            const ratio = root.upright ? (at.x - root.left) / Math.max(1, root.spanWidth) : (at.y - root.top) / Math.max(1, root.spanHeight);
            root.workspace.setRatio(root.path, ratio);
        }
    }
}
