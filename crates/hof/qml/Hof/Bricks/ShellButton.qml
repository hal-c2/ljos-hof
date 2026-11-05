import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts


// A button. Outline by default (the header pills),
// `subtle` for the ghost buttons, `primary` for the accent one; `iconName` is a
// lucide id drawn before the text, `chevron` appends the menu chevron. A
// button with only an icon or only the chevron names itself on hover from its
// accessible name; `toolTip` says something else, or names a button that has text.
Button {
    id: control

    property bool primary: false
    property bool subtle: false
    property bool chevron: false
    property string iconName: ""
    property string toolTip: ""
    property real iconSize: 14
    // Fills the icon's outline too, for the filled variants (the fast-mode bolt).
    property bool iconFilled: false
    property real chevronSize: 14
    property color tint: primary ? Theme.palette.color("accentForeground", "#ffffff") : Theme.palette.color("text", "#e4e4e7")
    property color iconTint: tint
    property real radius: Math.min(Theme.radius, 8)
    readonly property bool iconOnly: text.length === 0 && iconName.length > 0 && !chevron
    readonly property bool chevronOnly: text.length === 0 && iconName.length === 0 && chevron
    readonly property color accentSurface: Theme.palette.color("accentSurface", "#27272a")
    // The theme owns the surface's alpha (the accent surface); ghost
    // buttons use it as authored, outline ones at half strength.
    readonly property color hoverFill: Qt.alpha(accentSurface, accentSurface.a * (control.subtle ? 1 : 0.5))
    readonly property color focusRing: Theme.palette.color("focus", "#3b82f6")
    // A lone glyph sits on the button's centre: the padding is what is left
    // of the height, so a 24px chevron half stays square and centred.
    readonly property real glyphPadding: iconOnly ? (height - iconSize) / 2 : (height - chevronSize) / 2

    implicitHeight: 28
    topPadding: 0
    bottomPadding: 0
    leftPadding: iconOnly || chevronOnly ? glyphPadding : 8
    rightPadding: iconOnly || chevronOnly ? glyphPadding : 8
    font.pixelSize: Math.round(13 * Theme.fontScale)
    font.weight: Font.Medium
    hoverEnabled: true
    scale: down ? 0.97 : 1
    opacity: enabled ? 1 : 0.64
    ToolTip.visible: hovered && ToolTip.text.length > 0
    ToolTip.text: toolTip.length > 0 ? toolTip : iconOnly || chevronOnly ? Accessible.name : ""
    ToolTip.delay: 400

    Behavior on scale {
        NumberAnimation {
            duration: 100
            easing.type: Easing.OutCubic
        }
    }

    background: Rectangle {
        radius: control.radius
        color: control.primary ? (control.down ? Qt.darker(Theme.palette.color("accent", "#2563eb"), 1.15) : control.hovered ? Qt.lighter(Theme.palette.color("accent", "#2563eb"), 1.08) : Theme.palette.color("accent", "#2563eb")) : control.hovered || control.down || control.checked ? control.hoverFill : control.subtle ? Qt.alpha(control.hoverFill, 0) : Qt.alpha(Theme.palette.color("input", "#27272a"), 0.32)
        // Keyboard focus draws a ring; pointer focus stays quiet.
        border.color: control.visualFocus ? control.focusRing : control.primary || control.subtle ? "transparent" : Theme.palette.color("input", "#27272a")
        border.width: control.visualFocus || !(control.primary || control.subtle) ? 1 : 0

        Behavior on color {
            ColorAnimation {
                duration: 120
            }
        }
    }

    contentItem: Item {
        implicitWidth: contents.implicitWidth
        implicitHeight: contents.implicitHeight

        RowLayout {
            id: contents
            objectName: "contents"

            anchors.centerIn: parent
            width: Math.min(parent.width, implicitWidth)
            spacing: 6

            ShellIcon {
                objectName: "buttonIcon"
                visible: control.iconName.length > 0
                name: control.iconName
                size: control.iconSize
                filled: control.iconFilled
                color: control.iconTint
                Layout.alignment: Qt.AlignVCenter
            }

            Text {
                visible: control.text.length > 0
                text: control.text
                font: control.font
                color: control.tint
                elide: Text.ElideRight
                verticalAlignment: Text.AlignVCenter
                Layout.fillWidth: true
                Layout.minimumWidth: 0
            }

            ShellIcon {
                visible: control.chevron
                name: "chevron-down"
                size: control.chevronSize
                color: control.text.length > 0 || control.iconName.length > 0 ? Theme.palette.color("iconMuted", "#8b8b93") : control.tint
                Layout.alignment: Qt.AlignVCenter
            }
        }
    }
}
