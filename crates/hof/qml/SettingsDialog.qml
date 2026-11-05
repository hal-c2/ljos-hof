import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Hof.App
import Hof.Bricks

// Hof's settings, for changing the config file without opening it. Each
// change is written into the file at once, its comments and the rest of
// it kept, and the window follows as it does when the file is saved.
// What has no control here (keys, commands, agents) is in the file, which
// "Open Config File" opens.
Popup {
    id: dialog

    // What went wrong with the last change; empty when nothing did.
    property string error: ""
    // The page shown: an index into `pages`.
    property int page: 0
    readonly property var pages: [
        {
            "name": "Appearance",
            "icon": "palette"
        },
        {
            "name": "Terminal",
            "icon": "terminal"
        },
        {
            "name": "Window",
            "icon": "monitor"
        },
        {
            "name": "Hosts",
            "icon": "server"
        }
    ]
    readonly property var hosts: JSON.parse(Settings.hostDetails)

    // Writes `[section] key = value`; null takes the key out.
    function put(section, key, value) {
        dialog.error = Settings.set(section, key, value);
    }

    // Text that means "unset" when empty.
    function putText(section, key, text) {
        dialog.put(section, key, text.length > 0 ? text : null);
    }

    // A new `[hosts.<name>]`, reached at its name until told otherwise.
    function addHost(name) {
        if (!/^[A-Za-z0-9_-]+$/.test(name)) {
            dialog.error = "A host's name is letters, digits, - and _";
            return false;
        }
        if (Settings.hosts.indexOf(name) >= 0) {
            dialog.error = "There is a host called " + name + " already";
            return false;
        }
        dialog.put("hosts." + name, "ssh", name);
        return dialog.error.length === 0;
    }

    function removeHost(name) {
        dialog.error = Settings.removeHost(name);
    }

    objectName: "settingsDialog"
    Accessible.name: "Settings"
    x: Math.round((parent.width - width) / 2)
    y: Math.round((parent.height - height) / 2)
    width: Math.min(parent.width - 32, 760)
    height: Math.min(parent.height - 32, 580)
    modal: true
    focus: true
    padding: 0

    onAboutToShow: dialog.error = ""

    // The window behind is dimmed while the settings are open.
    Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.4)
    }

    background: Rectangle {
        color: Theme.palette.color("canvas", "#09090b")
        border.color: Theme.palette.color("border", "#27272a")
        radius: Math.min(Theme.radius, 10)
    }

    contentItem: RowLayout {
        spacing: 0

        // The pages, down the side.
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 188
            color: Theme.palette.color("surfaceRaised", "#18181b")
            radius: Math.min(Theme.radius, 10)

            // Square on the side that meets the page.
            Rectangle {
                anchors.right: parent.right
                width: parent.radius
                height: parent.height
                color: parent.color
            }

            ColumnLayout {
                objectName: "settingsTabs"
                anchors.fill: parent
                anchors.margins: 10
                spacing: 2

                Text {
                    Layout.leftMargin: 10
                    Layout.topMargin: 6
                    Layout.bottomMargin: 10
                    text: "Settings"
                    color: Theme.palette.color("text", "#e4e4e7")
                    font.pixelSize: 15
                    font.weight: Font.DemiBold
                }

                Repeater {
                    model: dialog.pages

                    SettingNavItem {
                        required property var modelData
                        required property int index

                        objectName: "settingsPage" + modelData.name
                        text: modelData.name
                        iconName: modelData.icon
                        current: dialog.page === index
                        onClicked: dialog.page = index
                    }
                }

                Item {
                    Layout.fillHeight: true
                }

                ShellButton {
                    Layout.fillWidth: true
                    subtle: true
                    iconName: "file-text"
                    text: "Open Config File"
                    onClicked: App.editConfig()
                }
            }
        }

        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 1
            color: Theme.palette.color("border", "#27272a")
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            // Made afresh each time the dialog opens, so that every control
            // starts from what the file says now.
            Loader {
                Layout.fillWidth: true
                Layout.fillHeight: true
                active: dialog.visible

                sourceComponent: StackLayout {
                    currentIndex: dialog.page

                    SettingPage {
                        title: "Appearance"
                        description: "How the terminal's text and window look."

                        SettingGroup {
                            title: "Colours"

                            SettingRow {
                                title: "Theme"
                                description: "Hof's own, or one in the themes folder."

                                ShellComboBox {
                                    objectName: "settingTheme"
                                    Layout.preferredWidth: 220
                                    outline: true
                                    model: Settings.themeNames
                                    currentIndex: Settings.themeNames.indexOf(Settings.themeName)
                                    Accessible.name: "Theme"
                                    onActivated: index => dialog.put("theme", "name", Settings.themeNames[index])
                                }
                            }
                            SettingRow {
                                title: "Background opacity"
                                description: "Below 100% the desktop shows through."

                                RowLayout {
                                    spacing: 8

                                    ShellSlider {
                                        id: opacitySlider

                                        objectName: "settingOpacity"
                                        Layout.preferredWidth: 160
                                        from: 0.1
                                        to: 1
                                        stepSize: 0.05
                                        value: Settings.opacity
                                        Accessible.name: "Background opacity"
                                        // Written once the slider stops: each
                                        // write reads the file again.
                                        onMoved: opacityWrite.restart()

                                        Timer {
                                            id: opacityWrite

                                            interval: 250
                                            onTriggered: dialog.put("window", "opacity", Math.round(opacitySlider.value * 100) / 100)
                                        }
                                    }
                                    Text {
                                        Layout.preferredWidth: 40
                                        horizontalAlignment: Text.AlignRight
                                        text: Math.round(opacitySlider.value * 100) + "%"
                                        color: Theme.palette.color("textMuted", "#8b8b93")
                                        font.pixelSize: 13
                                    }
                                }
                            }
                        }

                        SettingGroup {
                            title: "Text"

                            SettingRow {
                                title: "Font"
                                description: "A family installed here; empty is the system's monospace."

                                ShellTextField {
                                    objectName: "settingFontFamily"
                                    Layout.preferredWidth: 220
                                    text: Settings.fontFamily
                                    placeholderText: "monospace"
                                    Accessible.name: "Font"
                                    onEditingFinished: if (text !== Settings.fontFamily)
                                        dialog.putText("font", "family", text)
                                }
                            }
                            SettingRow {
                                title: "Size"
                                description: "In points."

                                ShellSpinBox {
                                    objectName: "settingFontSize"
                                    from: 4
                                    to: 72
                                    value: Math.round(Settings.fontSize)
                                    editable: true
                                    Accessible.name: "Font size"
                                    onValueModified: dialog.put("font", "size", value)
                                }
                            }
                            SettingRow {
                                title: "Bold text in bright colours"
                                description: "As older terminals drew bold."

                                ShellSwitch {
                                    objectName: "settingBoldIsBright"
                                    checked: Settings.boldIsBright
                                    Accessible.name: "Bold text in bright colours"
                                    onToggled: dialog.put("font", "bold-is-bright", checked)
                                }
                            }
                            SettingRow {
                                title: "Padding"
                                description: "Space between the text and the pane's edge, in pixels."

                                ShellSpinBox {
                                    objectName: "settingPadding"
                                    from: 0
                                    to: 64
                                    value: Math.round(Settings.padding)
                                    editable: true
                                    Accessible.name: "Padding"
                                    onValueModified: dialog.put("window", "padding", value)
                                }
                            }
                        }
                    }

                    SettingPage {
                        title: "Terminal"
                        description: "The cursor, the scrollback, and what programs may do."

                        SettingGroup {
                            title: "Cursor and scrollback"

                            SettingRow {
                                title: "Cursor"

                                ShellComboBox {
                                    readonly property list<string> values: ["block", "bar", "underline"]

                                    objectName: "settingCursorShape"
                                    Layout.preferredWidth: 180
                                    outline: true
                                    model: ["Block", "Bar", "Underline"]
                                    currentIndex: values.indexOf(Settings.cursorShape)
                                    Accessible.name: "Cursor"
                                    onActivated: index => dialog.put("terminal", "cursor-shape", values[index])
                                }
                            }
                            SettingRow {
                                title: "Blinking cursor"

                                ShellSwitch {
                                    objectName: "settingCursorBlink"
                                    checked: Settings.cursorBlink
                                    Accessible.name: "Blinking cursor"
                                    onToggled: dialog.put("terminal", "cursor-blink", checked)
                                }
                            }
                            SettingRow {
                                title: "Scrollback lines"
                                description: "Kept for each pane; 0 keeps none."

                                ShellSpinBox {
                                    objectName: "settingScrollback"
                                    Layout.preferredWidth: 140
                                    from: 0
                                    to: 1000000
                                    stepSize: 1000
                                    value: Settings.scrollback
                                    editable: true
                                    Accessible.name: "Scrollback lines"
                                    onValueModified: dialog.put("terminal", "scrollback", value)
                                }
                            }
                        }

                        SettingGroup {
                            title: "Clipboard"

                            SettingRow {
                                title: "Copy what is selected"

                                ShellSwitch {
                                    objectName: "settingCopyOnSelect"
                                    checked: Settings.copyOnSelect
                                    Accessible.name: "Copy what is selected"
                                    onToggled: dialog.put("terminal", "copy-on-select", checked)
                                }
                            }
                            SettingRow {
                                title: "Ask before an unsafe paste"
                                description: "Text with a line break or a control character could run a command."

                                ShellSwitch {
                                    objectName: "settingConfirmPaste"
                                    checked: Settings.confirmUnsafePaste
                                    Accessible.name: "Ask before an unsafe paste"
                                    onToggled: dialog.put("terminal", "confirm-unsafe-paste", checked)
                                }
                            }
                            SettingRow {
                                title: "Programs may read the clipboard"

                                ShellComboBox {
                                    readonly property list<string> values: ["ask", "allow", "deny"]

                                    objectName: "settingClipboardRead"
                                    Layout.preferredWidth: 180
                                    outline: true
                                    model: ["Ask each time", "Always", "Never"]
                                    currentIndex: values.indexOf(Settings.clipboardRead)
                                    Accessible.name: "Programs may read the clipboard"
                                    onActivated: index => dialog.put("terminal", "clipboard-read", values[index])
                                }
                            }
                            SettingRow {
                                title: "Programs may copy"

                                ShellSwitch {
                                    objectName: "settingClipboardWrite"
                                    checked: Settings.clipboardWrite
                                    Accessible.name: "Programs may copy"
                                    onToggled: dialog.put("terminal", "clipboard-write", checked)
                                }
                            }
                            SettingRow {
                                title: "Programs may open links"

                                ShellComboBox {
                                    readonly property list<string> values: ["ask", "allow", "deny"]

                                    objectName: "settingOpenUrls"
                                    Layout.preferredWidth: 180
                                    outline: true
                                    model: ["Ask each time", "Always", "Never"]
                                    currentIndex: values.indexOf(Settings.openUrls)
                                    Accessible.name: "Programs may open links"
                                    onActivated: index => dialog.put("terminal", "open-urls", values[index])
                                }
                            }
                        }

                        SettingGroup {
                            title: "New panes"

                            SettingRow {
                                title: "Shell"

                                ShellTextField {
                                    objectName: "settingShell"
                                    Layout.preferredWidth: 220
                                    text: Settings.shell
                                    placeholderText: "Your login shell"
                                    Accessible.name: "Shell"
                                    onEditingFinished: if (text !== Settings.shell)
                                        dialog.putText("terminal", "shell", text)
                                }
                            }
                            SettingRow {
                                title: "Start in"

                                ShellTextField {
                                    objectName: "settingWorkingDirectory"
                                    Layout.preferredWidth: 220
                                    text: Settings.workingDirectory
                                    placeholderText: "Where Hof was started"
                                    Accessible.name: "Start in"
                                    onEditingFinished: if (text !== Settings.workingDirectory)
                                        dialog.putText("terminal", "working-directory", text)
                                }
                            }
                        }
                    }

                    SettingPage {
                        title: "Window"
                        description: "The menu, the frame, and what a new window is."

                        SettingGroup {
                            title: "Frame"

                            SettingRow {
                                title: "Menu"
                                description: "A Mac has its menu bar either way."

                                ShellComboBox {
                                    readonly property list<string> values: ["auto", "bar", "button", "none"]

                                    objectName: "settingMenu"
                                    Layout.preferredWidth: 200
                                    outline: true
                                    model: ["As usual here", "Menu bar", "Button in the tab bar", "None, only keys"]
                                    currentIndex: values.indexOf(Settings.menuChoice)
                                    Accessible.name: "Menu"
                                    onActivated: index => dialog.put("window", "menu", values[index])
                                }
                            }
                            SettingRow {
                                title: "Title bar and borders"

                                ShellSwitch {
                                    objectName: "settingDecorations"
                                    checked: Settings.decorations
                                    Accessible.name: "Title bar and borders"
                                    onToggled: dialog.put("window", "decorations", checked)
                                }
                            }
                            SettingRow {
                                title: "Title"

                                ShellTextField {
                                    objectName: "settingTitle"
                                    Layout.preferredWidth: 220
                                    text: Settings.title
                                    placeholderText: "Hof"
                                    Accessible.name: "Title"
                                    onEditingFinished: if (text !== Settings.title)
                                        dialog.putText("window", "title", text)
                                }
                            }
                            SettingRow {
                                title: "New windows"
                                description: "Columns by rows."

                                RowLayout {
                                    spacing: 6

                                    ShellSpinBox {
                                        objectName: "settingColumns"
                                        Layout.preferredWidth: 104
                                        from: 20
                                        to: 1000
                                        value: Settings.columns
                                        editable: true
                                        Accessible.name: "Columns"
                                        onValueModified: dialog.put("window", "width", value)
                                    }
                                    Text {
                                        text: "×"
                                        color: Theme.palette.color("textMuted", "#8b8b93")
                                    }
                                    ShellSpinBox {
                                        objectName: "settingRows"
                                        Layout.preferredWidth: 104
                                        from: 5
                                        to: 500
                                        value: Settings.rows
                                        editable: true
                                        Accessible.name: "Rows"
                                        onValueModified: dialog.put("window", "height", value)
                                    }
                                }
                            }
                        }

                        SettingGroup {
                            title: "Session"

                            SettingRow {
                                title: "Reopen the last session"
                                description: "Its windows, tabs and splits, where they were."

                                ShellSwitch {
                                    objectName: "settingSessionRestore"
                                    checked: Settings.sessionRestore
                                    Accessible.name: "Reopen the last session"
                                    onToggled: dialog.put("session", "restore", checked)
                                }
                            }
                            SettingRow {
                                title: "Keep scrollback with it"

                                ShellSwitch {
                                    objectName: "settingSaveScrollback"
                                    checked: Settings.sessionSaveScrollback
                                    Accessible.name: "Keep scrollback with it"
                                    onToggled: dialog.put("session", "save-scrollback", checked)
                                }
                            }
                            SettingRow {
                                title: "Tell me when an agent waits"
                                description: "A notification when an agent out of sight needs you."

                                ShellSwitch {
                                    objectName: "settingAgentWaiting"
                                    checked: Settings.notifyAgentWaiting
                                    Accessible.name: "Tell me when an agent waits"
                                    onToggled: dialog.put("notify", "agent-waiting", checked)
                                }
                            }
                        }
                    }

                    SettingPage {
                        title: "Hosts"
                        description: "Machines a pane can run on, reached over ssh. Open a pane on one from the host picker."

                        SettingGroup {
                            title: "Add a host"

                            SettingRow {
                                title: "Name"
                                description: "What the host picker shows; it is the address too until you change it."

                                RowLayout {
                                    spacing: 8

                                    ShellTextField {
                                        id: newHost

                                        objectName: "settingNewHost"
                                        Layout.preferredWidth: 160
                                        placeholderText: "dev"
                                        Accessible.name: "New host's name"
                                        onAccepted: if (dialog.addHost(text))
                                            text = ""
                                    }
                                    ShellButton {
                                        objectName: "settingAddHost"
                                        iconName: "plus"
                                        text: "Add"
                                        onClicked: if (dialog.addHost(newHost.text))
                                            newHost.text = ""
                                    }
                                }
                            }
                        }

                        Repeater {
                            model: dialog.hosts

                            SettingGroup {
                                id: host

                                required property var modelData
                                readonly property string section: "hosts." + modelData.name

                                title: modelData.name
                                trailing: ShellButton {
                                    objectName: "settingRemoveHost_" + host.modelData.name
                                    subtle: true
                                    iconName: "trash-2"
                                    text: "Remove"
                                    onClicked: dialog.removeHost(host.modelData.name)
                                }

                                SettingRow {
                                    visible: host.modelData.command
                                    title: "Reached by a command of its own"
                                    description: "Set in the file; the address, user and port are not used."
                                }
                                SettingRow {
                                    title: "Address"
                                    description: "A name or address, or a Host of your ssh config."

                                    ShellTextField {
                                        objectName: "settingHostSsh_" + host.modelData.name
                                        Layout.preferredWidth: 220
                                        text: host.modelData.ssh
                                        placeholderText: host.modelData.name
                                        Accessible.name: "Address of " + host.modelData.name
                                        onEditingFinished: if (text !== host.modelData.ssh)
                                            dialog.putText(host.section, "ssh", text)
                                    }
                                }
                                SettingRow {
                                    title: "User"
                                    description: "Who to log in as; empty is ssh's choice."

                                    ShellTextField {
                                        objectName: "settingHostUser_" + host.modelData.name
                                        Layout.preferredWidth: 220
                                        text: host.modelData.user
                                        placeholderText: "As ssh chooses"
                                        Accessible.name: "User on " + host.modelData.name
                                        onEditingFinished: if (text !== host.modelData.user)
                                            dialog.putText(host.section, "user", text)
                                    }
                                }
                                SettingRow {
                                    title: "Port"
                                    description: "Empty is ssh's choice, usually 22."

                                    ShellTextField {
                                        objectName: "settingHostPort_" + host.modelData.name
                                        Layout.preferredWidth: 100
                                        text: host.modelData.port > 0 ? String(host.modelData.port) : ""
                                        placeholderText: "22"
                                        inputMethodHints: Qt.ImhDigitsOnly
                                        Accessible.name: "Port on " + host.modelData.name
                                        onEditingFinished: {
                                            const port = text.length > 0 ? Number(text) : 0;
                                            if (port === host.modelData.port)
                                                return;
                                            if (!Number.isInteger(port) || port < 0 || port > 65535) {
                                                dialog.error = "A port is a number from 1 to 65535";
                                                return;
                                            }
                                            dialog.put(host.section, "port", port > 0 ? port : null);
                                        }
                                    }
                                }
                                SettingRow {
                                    title: "X11 windows"
                                    description: "Programs there may open windows here."

                                    ShellSwitch {
                                        checked: host.modelData.x11
                                        Accessible.name: "X11 windows from " + host.modelData.name
                                        onToggled: dialog.put(host.section, "x11", checked)
                                    }
                                }
                                SettingRow {
                                    title: "Sound"
                                    description: "Programs there may play sound here."

                                    ShellSwitch {
                                        checked: host.modelData.sound
                                        Accessible.name: "Sound from " + host.modelData.name
                                        onToggled: dialog.put(host.section, "sound", checked)
                                    }
                                }
                                SettingRow {
                                    title: "Files"
                                    description: "Show that machine's files here."

                                    ShellSwitch {
                                        checked: host.modelData.files
                                        Accessible.name: "Files of " + host.modelData.name
                                        onToggled: dialog.put(host.section, "files", checked)
                                    }
                                }
                                SettingRow {
                                    title: "Programs there may read the clipboard"

                                    ShellComboBox {
                                        readonly property list<string> values: ["ask", "allow", "deny"]

                                        Layout.preferredWidth: 180
                                        outline: true
                                        model: ["Ask each time", "Always", "Never"]
                                        currentIndex: values.indexOf(host.modelData.clipboard)
                                        Accessible.name: "Programs on " + host.modelData.name + " may read the clipboard"
                                        onActivated: index => dialog.put(host.section, "clipboard", values[index])
                                    }
                                }
                            }
                        }

                        Text {
                            Layout.fillWidth: true
                            visible: dialog.hosts.length === 0
                            text: "No hosts yet."
                            color: Theme.palette.color("textMuted", "#8b8b93")
                            font.pixelSize: 13
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 1
                color: Theme.palette.color("border", "#27272a")
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.margins: 12
                Layout.leftMargin: 24
                spacing: 8

                // What went wrong, or else the file the settings go to,
                // by its name; the whole path on hover.
                Text {
                    objectName: "settingsError"
                    Layout.fillWidth: true
                    Layout.preferredWidth: 0
                    text: dialog.error.length > 0 ? dialog.error : "Saved in " + Settings.path.slice(Settings.path.lastIndexOf("/") + 1)
                    Accessible.role: Accessible.StatusBar
                    Accessible.description: Settings.path
                    ToolTip.visible: dialog.error.length === 0 && pathHover.hovered
                    ToolTip.text: Settings.path
                    ToolTip.delay: 500
                    color: dialog.error.length > 0 ? Theme.palette.color("errorForeground", "#ef4444") : Theme.palette.color("textMuted", "#8b8b93")
                    elide: Text.ElideMiddle
                    font.pixelSize: 12

                    HoverHandler {
                        id: pathHover
                    }
                }
                ShellButton {
                    objectName: "settingsClose"
                    primary: true
                    text: "Done"
                    onClicked: dialog.close()
                }
            }
        }
    }
}
