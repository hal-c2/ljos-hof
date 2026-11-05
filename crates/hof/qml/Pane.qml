import QtQuick
import Hof
import Hof.App

// One pane: a terminal, its scrollbar, and what to say when its program
// could not start. Made once per pane; Main moves and resizes it.
Item {
    id: root

    required property int pane
    // The focused pane of its tab, and whether that tab is the one shown:
    // only both together take the keys.
    required property bool focused
    required property bool shown
    // `{ directory, command, replay }`: where the program starts and what
    // it is, empty for the shell, and what a restored pane showed.
    property var start: ({})
    property real fontSize: 11
    property color background
    property color foreground
    // The line round a pane of a split that does not have the keys.
    property color border: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15)
    // The line round the one that has them.
    readonly property color highlight: Settings.palette.length > 12 ? Settings.palette[12] : Settings.palette.length > 4 ? Settings.palette[4] : "lightsteelblue"
    // One of several in its tab: drawn in a frame, a little apart from the
    // others, when the config wants a line round the panes.
    readonly property bool framed: Settings.paneBorder > 0 && root.parent !== null && (root.width < root.parent.width || root.height < root.parent.height)
    // The space between the pane's edge and its frame: half the gap
    // between two frames.
    readonly property real gap: root.framed ? 2 : 0
    // The chord bound to an action, for the right-click menu to show.
    property var keyOf: name => ""

    // The find bar is showing.
    readonly property bool searching: search.visible
    // The right-click menu is open.
    readonly property bool menuOpen: menu.opened
    // A paste is waiting for the user to say yes.
    readonly property bool confirmingPaste: confirm.visible
    // Asking whether the program may read the clipboard or open a URL.
    readonly property bool asking: clipboardPermit.visible || openPermit.visible
    // Who "the program" is, in a question.
    readonly property string programName: root.remote ? "A program on " + root.host : "The program"
    // The AI agent the pane runs, if any, and how it is: "working",
    // "waiting" for the user, "idle", or "none".
    readonly property string agent: terminal.agent
    readonly property string agentState: terminal.agentState
    // The colours for each state, from the palette.
    readonly property color waitingColor: Settings.palette.length > 3 ? Settings.palette[3] : "orange"
    readonly property color workingColor: Settings.palette.length > 4 ? Settings.palette[4] : "steelblue"
    readonly property color idleColor: Settings.palette.length > 2 ? Settings.palette[2] : "seagreen"

    signal terminalReady(Terminal terminal, Item pane)
    signal terminalGone
    signal focusTaken
    signal finished
    signal action(string name)
    // Something happened that `[hooks]` may want, with what it says as
    // `NAME=value` each.
    signal hook(string event, var extra)
    // The pane's agent began waiting for the user.
    signal attention

    // The command the shell is running, for its hook when it finishes.
    property string command: ""
    // The `[hosts]` machine the program runs on; empty for this one. What
    // the config says of this machine's shell and directories is not for
    // that one, which uses its own.
    readonly property string host: root.start.host ?? ""
    readonly property bool remote: root.host.length > 0

    clip: true

    function openSearch() {
        search.open();
    }

    function giveFocus() {
        if (!root.focused || !root.shown)
            return;
        if (openPermit.visible && openPermit.z >= clipboardPermit.z && openPermit.z >= confirm.z)
            openPermit.forceActiveFocus();
        else if (clipboardPermit.visible && clipboardPermit.z >= confirm.z)
            clipboardPermit.forceActiveFocus();
        else if (confirm.visible)
            confirm.forceActiveFocus();
        else
            terminal.forceActiveFocus();
    }

    onFocusedChanged: root.giveFocus()
    onShownChanged: root.giveFocus()

    Terminal {
        id: terminal

        anchors.fill: parent
        anchors.margins: root.framed ? root.gap + Settings.paneBorder : 0
        font.family: Settings.fontFamily
        // Points to pixels, as at 96 dpi; the Terminal's own default font
        // is in pixels, which a point size would not win over.
        font.pixelSize: Math.max(1, Math.round(root.fontSize * 4 / 3))
        padding: Settings.padding
        backgroundColor: root.background
        foregroundColor: root.foreground
        cursorColor: Settings.cursor
        selectionColor: Settings.selection
        palette: Settings.palette
        boldIsBright: Settings.boldIsBright
        cursorBlink: Settings.cursorBlink
        cursorShape: Settings.cursorShape
        scrollback: Settings.scrollback
        copyOnSelect: Settings.copyOnSelect
        confirmPaste: Settings.confirmUnsafePaste
        // A program on another machine reads as its host lets it.
        clipboardRead: {
            if (!root.remote)
                return Settings.clipboardRead;
            const i = Settings.hosts.indexOf(root.host);
            return i >= 0 ? Settings.hostClipboards[i] : "ask";
        }
        clipboardWrite: Settings.clipboardWrite
        openUrls: Settings.openUrls
        // Main's shortcuts own the keys the config binds.
        shortcuts: false
        program: {
            const command = root.start.command ?? [];
            return command.length > 0 ? command[0] : root.remote ? "" : Settings.shell;
        }
        arguments: {
            const command = root.start.command ?? [];
            return command.length > 0 ? command.slice(1) : root.remote ? [] : Settings.shellArguments;
        }
        // Scripts in the pane find it, and Hof, by these (`hof ctl`); not
        // on another machine, where this one's socket is nowhere.
        environment: Settings.environment.concat(root.start.environment ?? [], root.remote ? [] : ["HOF_PANE_ID=" + root.pane, "HOF_SOCKET=" + App.controlSocket])
        startDirectory: {
            const d = root.start.directory ?? "";
            return d.length > 0 || root.remote ? d : Settings.workingDirectory;
        }
        host: root.host
        // A restored pane's program, which ran on while Hof was away.
        remotePane: root.start.remote ?? -1
        // The scrollback of a restored pane, drawn before its program starts.
        replay: root.start.replay ?? ""
        autoStart: true

        // The keys back in the terminal, by a click or another pane's
        // turn, are done with finding.
        onActiveFocusChanged: if (terminal.activeFocus) {
            search.close();
            root.focusTaken();
        }
        onCellWidthChanged: if (terminal.cellWidth > 0)
            root.terminalReady(terminal, root)
        onCellHeightChanged: if (terminal.cellHeight > 0)
            root.terminalReady(terminal, root)
        onExited: root.finished()
        onPasteBlocked: text => {
            clipboardPermit.z = 3;
            openPermit.z = 3;
            confirm.ask(text);
        }
        onClipboardRequested: clipboard => {
            confirm.z = 3;
            openPermit.z = 3;
            clipboardPermit.ask(root.programName + " wants to read the " + clipboard + ".", "It would see whatever was copied last, here or in any other program.");
        }
        onOpenRequested: url => {
            confirm.z = 3;
            clipboardPermit.z = 3;
            openPermit.ask(root.programName + " wants to open:", url.length > 300 ? url.slice(0, 300) + "…" : url);
        }
        onBell: root.hook("bell", [])
        onCommandStarted: command => {
            root.command = command;
            root.hook("command-started", ["HOF_COMMAND=" + command]);
        }
        onCommandFinished: code => root.hook("command-finished", ["HOF_COMMAND=" + root.command, "HOF_EXIT_CODE=" + (code < 0 ? "" : code)])
        onNotification: (title, body) => root.hook("notification", ["HOF_TITLE=" + title, "HOF_BODY=" + body])
        onNotice: text => {
            notice.text = text;
            if (text.length > 0)
                noticeShown.restart();
        }
        onTitleChanged: root.hook("title-changed", ["HOF_TITLE=" + terminal.title])
        onWorkingDirectoryChanged: root.hook("directory-changed", ["HOF_DIRECTORY=" + terminal.workingDirectory])
        onAgentStatus: (agent, state) => {
            root.hook("agent-changed", ["HOF_AGENT=" + agent, "HOF_AGENT_STATE=" + state]);
            if (state === "waiting")
                root.attention();
        }

        Component.onCompleted: {
            root.terminalReady(terminal, root);
            root.giveFocus();
        }
        Component.onDestruction: root.terminalGone()

        // A hand over a link, which Ctrl+click opens.
        HoverHandler {
            cursorShape: terminal.hoveredLink.length > 0 ? Qt.PointingHandCursor : Qt.IBeamCursor
        }
    }

    // A right click opens the pane's menu, unless the program asked for
    // the mouse, which Shift takes back as it does for selecting.
    MouseArea {
        objectName: "paneMenuArea"

        anchors.fill: parent
        acceptedButtons: Qt.RightButton
        enabled: !root.asking && !root.confirmingPaste
        onPressed: mouse => {
            if (terminal.mouseTracking && !(mouse.modifiers & Qt.ShiftModifier)) {
                mouse.accepted = false;
                return;
            }
            terminal.forceActiveFocus();
            root.openMenu(mouse.x, mouse.y);
        }
    }

    function openMenu(x, y) {
        menu.popup(x, y);
    }

    PaneMenu {
        id: menu

        canCopy: terminal.hasSelection
        keyOf: root.keyOf

        onChose: name => root.action(name)
        onClosed: root.giveFocus()
    }


    Scrollbar {
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        terminal: terminal
        color: root.foreground
    }

    SearchBar {
        id: search

        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 8
        z: 2
        terminal: terminal
        background: root.background
        foreground: root.foreground
        border: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)
        fontFamily: Settings.fontFamily
        fontSize: root.fontSize
    }

    PasteConfirm {
        id: confirm
        objectName: "pasteConfirm"

        anchors.centerIn: parent
        z: 3
        terminal: terminal
        background: root.background
        foreground: root.foreground
        border: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)
        fontFamily: Settings.fontFamily
        fontSize: root.fontSize
    }

    Permit {
        id: clipboardPermit
        objectName: "clipboardPermit"

        anchors.centerIn: parent
        z: 3
        terminal: terminal
        background: root.background
        foreground: root.foreground
        border: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)
        fontFamily: Settings.fontFamily
        fontSize: root.fontSize
        active: root.focused && root.shown

        onAnswered: (allow, always) => terminal.answerClipboard(allow, always)
    }

    Permit {
        id: openPermit
        objectName: "openPermit"

        anchors.centerIn: parent
        z: 3
        terminal: terminal
        background: root.background
        foreground: root.foreground
        border: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)
        fontFamily: Settings.fontFamily
        fontSize: root.fontSize
        active: root.focused && root.shown

        onAnswered: (allow, always) => terminal.answerOpen(allow, always)
    }

    // The frame round a pane of a split, in the highlight colour for the
    // one with the keys. An agent waiting for the user turns it its own
    // colour, seen from across the window, split or not.
    Rectangle {
        objectName: "paneFrame"

        readonly property bool waiting: root.agentState === "waiting"
        readonly property color edge: waiting ? root.waitingColor : root.focused ? root.highlight : root.border

        anchors.fill: parent
        anchors.margins: root.gap
        z: 1
        visible: root.framed || waiting
        color: "transparent"
        radius: Settings.paneRadius
        border.width: waiting ? Math.max(2, Settings.paneBorder) : Settings.paneBorder
        border.color: edge
    }

    // Which agent runs, and how it is.
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        anchors.margins: 6
        z: 1
        visible: root.agentState !== "none" && root.agent.length > 0
        width: agentText.implicitWidth + 14
        height: agentText.implicitHeight + 6
        radius: height / 2
        color: Qt.darker(root.background, 1.3)
        border.color: root.agentState === "waiting" ? root.waitingColor : root.agentState === "working" ? root.workingColor : root.idleColor
        opacity: root.agentState === "idle" ? 0.6 : 0.9

        Text {
            id: agentText

            anchors.centerIn: parent
            text: (root.agentState === "waiting" ? "● " : root.agentState === "working" ? "◌ " : "○ ") + root.agent + " · " + root.agentState
            color: root.foreground
            font.family: Settings.fontFamily
            textFormat: Text.PlainText
            font.pointSize: Math.max(1, root.fontSize - 2)
        }
    }

    // The link under the pointer, as a browser shows it.
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.margins: 6
        z: 1
        visible: terminal.hoveredLink.length > 0
        width: Math.min(parent.width - 12, hoveredText.implicitWidth + 14)
        height: hoveredText.implicitHeight + 6
        radius: 3
        color: Qt.darker(root.background, 1.3)
        border.color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)

        Text {
            id: hoveredText
            objectName: "hoveredLinkText"

            anchors.verticalCenter: parent.verticalCenter
            x: 7
            width: parent.width - 14
            text: terminal.hoveredLink + "  ·  Ctrl+click to open"
            color: root.foreground
            elide: Text.ElideMiddle
            font.family: Settings.fontFamily
            textFormat: Text.PlainText
            font.pointSize: Math.max(1, root.fontSize - 2)
        }
    }

    // Which machine a remote pane is on.
    Text {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 6
        visible: root.remote && terminal.connection === "up"
        text: root.host
        color: root.foreground
        opacity: 0.45
        font.family: Settings.fontFamily
        textFormat: Text.PlainText
        font.pointSize: Math.max(1, root.fontSize - 2)
    }

    // The connection is being made, again or for the first time; the
    // screen stays as it was meanwhile.
    Rectangle {
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.topMargin: 8
        z: 1
        visible: terminal.connection === "connecting" || terminal.connection === "down"
        width: Math.min(parent.width - 16, linkText.implicitWidth + 24)
        height: linkText.implicitHeight + 12
        radius: 4
        color: Qt.darker(root.background, 1.3)
        border.color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)

        Text {
            id: linkText

            anchors.centerIn: parent
            width: parent.width - 24
            text: {
                const why = terminal.connectionMessage;
                if (terminal.connection === "down")
                    return "Reconnecting to " + root.host + (why.length > 0 ? ": " + why : "…");
                return "Connecting to " + root.host + "…";
            }
            color: root.foreground
            elide: Text.ElideRight
            horizontalAlignment: Text.AlignHCenter
            font.family: Settings.fontFamily
            textFormat: Text.PlainText
            font.pointSize: Math.max(1, root.fontSize - 1)
        }
    }

    // A file of the other machine being copied here to open, or why it
    // could not be.
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottomMargin: 8
        z: 1
        visible: notice.text.length > 0
        width: Math.min(parent.width - 16, notice.implicitWidth + 24)
        height: notice.implicitHeight + 12
        radius: 4
        color: Qt.darker(root.background, 1.3)
        border.color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.25)

        Text {
            id: notice

            anchors.centerIn: parent
            width: parent.width - 24
            color: root.foreground
            elide: Text.ElideMiddle
            horizontalAlignment: Text.AlignHCenter
            font.family: Settings.fontFamily
            textFormat: Text.PlainText
            font.pointSize: Math.max(1, root.fontSize - 1)
        }

        Timer {
            id: noticeShown

            interval: 8000
            onTriggered: notice.text = ""
        }

        MouseArea {
            anchors.fill: parent
            onClicked: notice.text = ""
        }
    }

    // Why the program did not start.
    Text {
        anchors.centerIn: parent
        width: parent.width - 48
        visible: terminal.error.length > 0
        text: terminal.error
        color: root.foreground
        font.family: Settings.fontFamily
        textFormat: Text.PlainText
        font.pointSize: root.fontSize
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
    }
}
