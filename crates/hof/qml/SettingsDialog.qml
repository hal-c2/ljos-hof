import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Hof.App

// Hof's settings, for changing the config file without opening it. Each
// change is written into the file at once, its comments and the rest of
// it kept, and the window follows as it does when the file is saved.
// What has no control here (keys, hosts, commands) is in the file, which
// "Open Config File" opens.
Popup {
    id: dialog

    property color baseColor
    property color textColor
    property color accent
    property color danger
    // What went wrong with the last change; empty when nothing did.
    property string error: ""

    // `a` with `t` of `b` in it.
    function mix(a, b, t) {
        return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1);
    }

    // Writes `[section] key = value`; null takes the key out.
    function put(section, key, value) {
        dialog.error = Settings.set(section, key, value);
    }

    // Text that means "unset" when empty.
    function putText(section, key, text) {
        dialog.put(section, key, text.length > 0 ? text : null);
    }

    objectName: "settingsDialog"
    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)
    width: Math.min(parent.width - 32, 580)
    height: Math.min(parent.height - 32, 560)
    modal: true
    focus: true
    padding: 0

    palette.window: dialog.mix(dialog.baseColor, dialog.textColor, 0.06)
    palette.windowText: dialog.textColor
    palette.base: dialog.baseColor
    palette.text: dialog.textColor
    palette.button: dialog.mix(dialog.baseColor, dialog.textColor, 0.14)
    palette.buttonText: dialog.textColor
    palette.brightText: dialog.textColor
    palette.light: dialog.mix(dialog.baseColor, dialog.textColor, 0.2)
    palette.midlight: dialog.mix(dialog.baseColor, dialog.textColor, 0.26)
    palette.mid: dialog.mix(dialog.baseColor, dialog.textColor, 0.36)
    palette.dark: dialog.accent
    palette.highlight: dialog.accent
    palette.highlightedText: dialog.baseColor
    palette.placeholderText: Qt.rgba(dialog.textColor.r, dialog.textColor.g, dialog.textColor.b, 0.45)

    onAboutToShow: dialog.error = ""

    // The window behind is dimmed while the settings are open.
    Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.35)
    }

    background: Rectangle {
        color: dialog.palette.window
        border.color: Qt.rgba(dialog.textColor.r, dialog.textColor.g, dialog.textColor.b, 0.2)
        radius: 8
    }

    contentItem: ColumnLayout {
        spacing: 0

        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 20
            Layout.topMargin: 16
            Layout.bottomMargin: 8
            text: "Settings"
            font.pointSize: 15
            font.bold: true
        }

        TabBar {
            id: tabs

            objectName: "settingsTabs"
            Layout.fillWidth: true
            Layout.leftMargin: 16
            Layout.rightMargin: 16
            background: Item {}

            SettingTab {
                text: "Appearance"
            }
            SettingTab {
                text: "Terminal"
            }
            SettingTab {
                text: "Window"
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Qt.rgba(dialog.textColor.r, dialog.textColor.g, dialog.textColor.b, 0.12)
        }

        // Made afresh each time the dialog opens, so that every control
        // starts from what the file says now.
        Loader {
            Layout.fillWidth: true
            Layout.fillHeight: true
            active: dialog.visible

            sourceComponent: StackLayout {
                currentIndex: tabs.currentIndex

                // Appearance
                Flickable {
                    id: appearance

                    clip: true
                    contentHeight: appearanceGrid.implicitHeight + 32
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: ScrollBar {}

                    GridLayout {
                        id: appearanceGrid

                        x: 20
                        y: 16
                        width: appearance.width - 40
                        columns: 2
                        columnSpacing: 16
                        rowSpacing: 8

                        Label {
                            text: "Theme"
                        }
                        ComboBox {
                            objectName: "settingTheme"
                            Layout.fillWidth: true
                            model: Settings.themeNames
                            currentIndex: Settings.themeNames.indexOf(Settings.themeName)
                            onActivated: index => dialog.put("theme", "name", Settings.themeNames[index])
                        }

                        Label {
                            text: "Font"
                        }
                        TextField {
                            objectName: "settingFontFamily"
                            Layout.fillWidth: true
                            text: Settings.fontFamily
                            placeholderText: "monospace"
                            onEditingFinished: if (text !== Settings.fontFamily)
                                dialog.putText("font", "family", text)
                        }

                        Label {
                            text: "Size"
                        }
                        SpinBox {
                            objectName: "settingFontSize"
                            from: 4
                            to: 72
                            value: Math.round(Settings.fontSize)
                            editable: true
                            onValueModified: dialog.put("font", "size", value)
                        }

                        Label {
                            text: "Bold text in bright colours"
                        }
                        SettingSwitch {
                            objectName: "settingBoldIsBright"
                            checked: Settings.boldIsBright
                            onToggled: dialog.put("font", "bold-is-bright", checked)
                        }

                        Label {
                            text: "Background opacity"
                        }
                        RowLayout {
                            Layout.fillWidth: true

                            Slider {
                                id: opacitySlider

                                objectName: "settingOpacity"
                                Layout.fillWidth: true
                                from: 0.1
                                to: 1
                                stepSize: 0.05
                                value: Settings.opacity
                                // Written once the slider stops: each
                                // write reads the file again.
                                onMoved: opacityWrite.restart()

                                Timer {
                                    id: opacityWrite

                                    interval: 250
                                    onTriggered: dialog.put("window", "opacity", Math.round(opacitySlider.value * 100) / 100)
                                }
                            }
                            Label {
                                Layout.preferredWidth: 40
                                horizontalAlignment: Text.AlignRight
                                text: Math.round(opacitySlider.value * 100) + "%"
                            }
                        }

                        Label {
                            text: "Padding"
                        }
                        SpinBox {
                            objectName: "settingPadding"
                            from: 0
                            to: 64
                            value: Math.round(Settings.padding)
                            editable: true
                            onValueModified: dialog.put("window", "padding", value)
                        }
                    }
                }

                // Terminal
                Flickable {
                    id: terminal

                    clip: true
                    contentHeight: terminalGrid.implicitHeight + 32
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: ScrollBar {}

                    GridLayout {
                        id: terminalGrid

                        x: 20
                        y: 16
                        width: terminal.width - 40
                        columns: 2
                        columnSpacing: 16
                        rowSpacing: 8

                        Label {
                            text: "Cursor"
                        }
                        ComboBox {
                            readonly property list<string> values: ["block", "bar", "underline"]

                            objectName: "settingCursorShape"
                            Layout.fillWidth: true
                            model: ["Block", "Bar", "Underline"]
                            currentIndex: values.indexOf(Settings.cursorShape)
                            onActivated: index => dialog.put("terminal", "cursor-shape", values[index])
                        }

                        Label {
                            text: "Blinking cursor"
                        }
                        SettingSwitch {
                            objectName: "settingCursorBlink"
                            checked: Settings.cursorBlink
                            onToggled: dialog.put("terminal", "cursor-blink", checked)
                        }

                        Label {
                            text: "Scrollback lines"
                        }
                        SpinBox {
                            objectName: "settingScrollback"
                            Layout.fillWidth: true
                            from: 0
                            to: 1000000
                            stepSize: 1000
                            value: Settings.scrollback
                            editable: true
                            onValueModified: dialog.put("terminal", "scrollback", value)
                        }

                        Label {
                            text: "Copy what is selected"
                        }
                        SettingSwitch {
                            objectName: "settingCopyOnSelect"
                            checked: Settings.copyOnSelect
                            onToggled: dialog.put("terminal", "copy-on-select", checked)
                        }

                        Label {
                            text: "Ask before an unsafe paste"
                        }
                        SettingSwitch {
                            objectName: "settingConfirmPaste"
                            checked: Settings.confirmUnsafePaste
                            onToggled: dialog.put("terminal", "confirm-unsafe-paste", checked)
                        }

                        Label {
                            text: "Programs may read the clipboard"
                        }
                        ComboBox {
                            readonly property list<string> values: ["ask", "allow", "deny"]

                            objectName: "settingClipboardRead"
                            Layout.fillWidth: true
                            model: ["Ask each time", "Always", "Never"]
                            currentIndex: values.indexOf(Settings.clipboardRead)
                            onActivated: index => dialog.put("terminal", "clipboard-read", values[index])
                        }

                        Label {
                            text: "Programs may copy"
                        }
                        SettingSwitch {
                            objectName: "settingClipboardWrite"
                            checked: Settings.clipboardWrite
                            onToggled: dialog.put("terminal", "clipboard-write", checked)
                        }

                        Label {
                            text: "Programs may open links"
                        }
                        ComboBox {
                            readonly property list<string> values: ["ask", "allow", "deny"]

                            objectName: "settingOpenUrls"
                            Layout.fillWidth: true
                            model: ["Ask each time", "Always", "Never"]
                            currentIndex: values.indexOf(Settings.openUrls)
                            onActivated: index => dialog.put("terminal", "open-urls", values[index])
                        }

                        Label {
                            text: "Shell"
                        }
                        TextField {
                            objectName: "settingShell"
                            Layout.fillWidth: true
                            text: Settings.shell
                            placeholderText: "Your login shell"
                            onEditingFinished: if (text !== Settings.shell)
                                dialog.putText("terminal", "shell", text)
                        }

                        Label {
                            text: "Start in"
                        }
                        TextField {
                            objectName: "settingWorkingDirectory"
                            Layout.fillWidth: true
                            text: Settings.workingDirectory
                            placeholderText: "Where Hof was started"
                            onEditingFinished: if (text !== Settings.workingDirectory)
                                dialog.putText("terminal", "working-directory", text)
                        }
                    }
                }

                // Window
                Flickable {
                    id: windowPage

                    clip: true
                    contentHeight: windowGrid.implicitHeight + 32
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: ScrollBar {}

                    GridLayout {
                        id: windowGrid

                        x: 20
                        y: 16
                        width: windowPage.width - 40
                        columns: 2
                        columnSpacing: 16
                        rowSpacing: 8

                        Label {
                            text: "Menu"
                        }
                        ComboBox {
                            readonly property list<string> values: ["auto", "bar", "button", "none"]

                            objectName: "settingMenu"
                            Layout.fillWidth: true
                            model: ["As usual here", "Menu bar", "Button in the tab bar", "None, only keys"]
                            currentIndex: values.indexOf(Settings.menuChoice)
                            onActivated: index => dialog.put("window", "menu", values[index])
                        }

                        Label {
                            text: "Title bar and borders"
                        }
                        SettingSwitch {
                            objectName: "settingDecorations"
                            checked: Settings.decorations
                            onToggled: dialog.put("window", "decorations", checked)
                        }

                        Label {
                            text: "Title"
                        }
                        TextField {
                            objectName: "settingTitle"
                            Layout.fillWidth: true
                            text: Settings.title
                            placeholderText: "Hof"
                            onEditingFinished: if (text !== Settings.title)
                                dialog.putText("window", "title", text)
                        }

                        Label {
                            text: "New windows, in cells"
                        }
                        RowLayout {
                            SpinBox {
                                objectName: "settingColumns"
                                from: 20
                                to: 1000
                                value: Settings.columns
                                editable: true
                                onValueModified: dialog.put("window", "width", value)
                            }
                            Label {
                                text: "×"
                            }
                            SpinBox {
                                objectName: "settingRows"
                                from: 5
                                to: 500
                                value: Settings.rows
                                editable: true
                                onValueModified: dialog.put("window", "height", value)
                            }
                        }

                        Label {
                            text: "Reopen the last session"
                        }
                        SettingSwitch {
                            objectName: "settingSessionRestore"
                            checked: Settings.sessionRestore
                            onToggled: dialog.put("session", "restore", checked)
                        }

                        Label {
                            text: "Keep scrollback with it"
                        }
                        SettingSwitch {
                            objectName: "settingSaveScrollback"
                            checked: Settings.sessionSaveScrollback
                            onToggled: dialog.put("session", "save-scrollback", checked)
                        }

                        Label {
                            text: "Tell me when an agent waits"
                        }
                        SettingSwitch {
                            objectName: "settingAgentWaiting"
                            checked: Settings.notifyAgentWaiting
                            onToggled: dialog.put("notify", "agent-waiting", checked)
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Qt.rgba(dialog.textColor.r, dialog.textColor.g, dialog.textColor.b, 0.12)
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 12
            Layout.leftMargin: 20
            spacing: 8

            Label {
                objectName: "settingsError"
                Layout.fillWidth: true
                text: dialog.error.length > 0 ? dialog.error : Settings.path
                color: dialog.error.length > 0 ? dialog.danger : Qt.rgba(dialog.textColor.r, dialog.textColor.g, dialog.textColor.b, 0.5)
                elide: Text.ElideMiddle
                font.pointSize: 9
            }
            Button {
                leftPadding: 14
                rightPadding: 14
                text: "Open Config File"
                onClicked: App.editConfig()
            }
            Button {
                objectName: "settingsClose"
                leftPadding: 14
                rightPadding: 14
                text: "Close"
                onClicked: dialog.close()
            }
        }
    }
}
