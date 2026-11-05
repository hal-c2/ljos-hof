pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts


// A select: the composer's ghost pickers by default
// (icon, muted label, chevron), `outline: true` for a bordered field.
// The popup marks the current value (a quiet fill and a check) and the row
// the keyboard is on (`highlighted`, the hover fill); `markCurrent: false`
// leaves the current value unmarked.
// `disabledRows` greys out the rows at those indexes in the popup; keys can still
// reach them, so `onActivated` should refuse them too.
// `rowAction` is a control at the end of each popup row, which reads its row as
// `parent.row`; pressing it neither chooses the row nor closes the popup.
ComboBox {
    id: control

    property bool outline: false
    property var disabledRows: []
    property bool markCurrent: true
    property Component rowAction: null
    property string iconName: ""
    property real iconSize: 16
    property real chevronSize: 14
    // The ghost picker is borderless until the keyboard reaches it; the outline one
    // also shows its ring when it holds focus.
    readonly property bool showFocusRing: control.visualFocus || (control.outline && control.activeFocus)
    readonly property color accentSurface: Theme.palette.color("accentSurface", "#27272a")
    readonly property color hoverFill: Qt.alpha(accentSurface, accentSurface.a * (outline ? 0.5 : 1))
    readonly property color labelColor: control.hovered || control.down || control.popup.visible ? Theme.palette.color("text", "#e4e4e7") : Theme.palette.color("secondaryLabel", "#a1a1aa")

    implicitHeight: 28
    leftPadding: iconName.length > 0 ? 10 + iconSize + 6 : 10
    rightPadding: 10 + chevronSize + 4
    font.pixelSize: Math.round(14 * Theme.fontScale)
    font.weight: Font.Medium
    hoverEnabled: true
    opacity: enabled ? 1 : 0.64

    background: Rectangle {
        radius: Math.min(Theme.radius, 8)
        color: control.down || control.hovered || control.popup.visible ? control.hoverFill : control.outline ? Qt.alpha(Theme.palette.color("input", "#27272a"), 0.32) : Qt.alpha(control.hoverFill, 0)
        border.color: control.showFocusRing ? Theme.palette.color("focus", "#3b82f6") : control.outline ? Theme.palette.color("input", "#27272a") : "transparent"
        border.width: control.outline || control.showFocusRing ? 1 : 0

        Behavior on color {
            ColorAnimation {
                duration: 120
            }
        }

        ShellIcon {
            x: 10
            y: (parent.height - height) / 2
            visible: control.iconName.length > 0
            name: control.iconName
            size: control.iconSize
            color: control.labelColor
        }
    }

    contentItem: Text {
        text: control.displayText
        font: control.font
        color: control.labelColor
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    indicator: ShellIcon {
        x: control.width - width - 10
        y: (control.height - height) / 2
        name: "chevron-down"
        size: control.chevronSize
        strokeWidth: 2.25
        color: Theme.palette.color("iconMuted", "#8b8b93")
    }

    delegate: ItemDelegate {
        id: item

        required property var model
        required property int index

        width: ListView.view.width
        height: 28
        text: item.model[control.textRole] ?? item.model.modelData ?? item.model.display ?? ""
        readonly property bool selected: control.markCurrent && item.index === control.currentIndex

        enabled: !control.disabledRows.includes(item.index)
        highlighted: control.highlightedIndex === item.index
        opacity: enabled ? 1 : 0.5
        leftPadding: 8
        rightPadding: 8
        // No vertical padding: the label takes the whole row and centres on its fill.
        topPadding: 0
        bottomPadding: 0
        hoverEnabled: true

        Accessible.name: item.text
        Accessible.checkable: control.markCurrent
        Accessible.checked: item.selected

        contentItem: RowLayout {
            spacing: 8

            Text {
                text: item.text
                font: control.font
                color: Theme.palette.color("text", "#e4e4e7")
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            ShellIcon {
                objectName: "currentMark"
                visible: item.selected
                name: "check"
                size: 14
                color: Theme.palette.color("iconMuted", "#8b8b93")
                Layout.alignment: Qt.AlignVCenter
            }

            Loader {
                readonly property int row: item.index
                active: control.rowAction !== null
                visible: active
                sourceComponent: control.rowAction
                Layout.alignment: Qt.AlignVCenter
            }
        }

        background: Rectangle {
            radius: 6
            // The keyboard row and the pointer's read as one fill; the chosen
            // row is the quieter one beneath them.
            color: item.highlighted || item.hovered ? Theme.palette.color("accentSurface", "#27272a") : item.selected ? Qt.alpha(Theme.palette.color("text", "#e4e4e7"), 0.08) : "transparent"
        }
    }

    popup: Popup {
        // Opens upward when the window has no room below the combo, set as
        // it shows (the combo does not move while it is open).
        property bool upward: false
        y: upward ? -implicitHeight - 4 : control.height + 4
        onAboutToShow: upward = control.mapToItem(null, 0, control.height).y + implicitHeight + 4 > control.Window.height
        width: Math.max(control.width, 160)
        implicitHeight: Math.min(contentItem.implicitHeight + 8, 320)
        padding: 4

        enter: Transition {
            NumberAnimation {
                property: "opacity"
                from: 0
                to: 1
                duration: 120
                easing.type: Easing.OutCubic
            }
        }

        exit: Transition {
            NumberAnimation {
                property: "opacity"
                from: 1
                to: 0
                duration: 90
                easing.type: Easing.InCubic
            }
        }

        contentItem: ListView {
            clip: true
            // Counted, not measured: the rows exist only once the popup shows,
            // after `onAboutToShow` has chosen which way it opens.
            implicitHeight: control.count * 28
            model: control.popup.visible ? control.delegateModel : null
            currentIndex: control.highlightedIndex
            boundsBehavior: Flickable.StopAtBounds
        }

        background: Rectangle {
            radius: Math.min(Theme.radius, 10)
            color: Theme.palette.color("surfaceOverlay", "#18181b")
            border.color: Qt.alpha(Theme.palette.color("text", "#e4e4e7"), 0.1)
            border.width: 1
        }
    }
}
