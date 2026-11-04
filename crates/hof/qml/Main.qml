pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import Hof
import Hof.App

// Hof's window: tabs of split panes, one terminal each. The layout lives
// in `Workspace`; this file places a terminal where it says. Terminals are
// made once per pane and only moved, so splitting, closing and zooming
// keep every program and its scrollback.
Window {
    id: window

    // Points added to the font by font-bigger and font-smaller.
    property real zoom: 0
    readonly property real fontSize: Math.max(4, Settings.fontSize + window.zoom)
    readonly property color background: Settings.background
    readonly property color foreground: Settings.foreground
    readonly property color border: Qt.rgba(window.foreground.r, window.foreground.g, window.foreground.b, 0.15)

    // The Terminal of each pane, by pane id; `terminalsRevision` moves with it.
    property var terminals: ({})
    // The Pane item of each pane, by pane id, for what only it can do.
    property var paneItems: ({})
    property int terminalsRevision: 0
    // What a pane about to be made runs and where: `{ directory, command,
    // host, remote }`, `host` a `[hosts]` name (empty for this machine)
    // and `remote` its pane under `hof agent` there.
    property var starts: ({})
    // The window is sized to Settings' cells once the first terminal knows
    // its cell.
    property bool sized: App.dropdown
    // The layout is changing in steps (a new pane, a saved session put in
    // place): its new panes wait for their starts before they are made.
    property bool holding: false

    // Where the menu of actions is, as `[window] menu` says: the menu bar
    // (the system's own on macOS, else across the top of the window), a
    // button at the right of the tab bar, or nowhere but the keys.
    property bool menuBarShown: Settings.menu === "bar"
    readonly property bool menuButtonShown: Settings.menu === "button"
    // The actions the menu bar has, each shown with its first chord.
    readonly property list<string> menuActions: ["new-tab", "new-tab-on-host", "split-right", "split-down", "split-right-on-host", "split-down-on-host", "close-pane", "close-tab", "settings", "edit-config", "reload-config", "quit", "copy", "paste", "select-all", "search", "reset", "font-bigger", "font-smaller", "font-reset", "zoom-pane", "toggle-fullscreen", "minimize", "maximize", "previous-prompt", "next-prompt", "scroll-top", "scroll-bottom", "previous-tab", "next-tab", "focus-previous", "focus-next", "focus-left", "focus-right", "focus-up", "focus-down", "grow-left", "grow-right", "grow-up", "grow-down", "next-waiting"]

    // `[commands]` and `[hooks]`, as `Settings` gives them.
    readonly property var commands: JSON.parse(Settings.commands)
    readonly property var hooks: JSON.parse(Settings.hooks)

    // For whoever drives the window from outside (tests, the MCP server).
    readonly property Workspace layout: workspace
    readonly property int paneCount: paneModel.count
    readonly property bool tabsShown: tabBar.visible
    readonly property bool settingsShown: settingsDialog.opened
    readonly property bool windowMenuShown: windowMenu.opened
    readonly property bool confirmingPaste: {
        void window.terminalsRevision;
        const item = window.paneItems[workspace.focusedPane];
        return item !== undefined && item.confirmingPaste;
    }
    // The focused pane asks whether its program may do something.
    readonly property bool asking: {
        void window.terminalsRevision;
        const item = window.paneItems[workspace.focusedPane];
        return item !== undefined && item.asking;
    }
    readonly property bool pickingHost: hostPicker.visible
    readonly property int hostChoices: hostPicker.shown.length
    readonly property bool searchShown: {
        void window.terminalsRevision;
        const item = window.paneItems[workspace.focusedPane];
        return item !== undefined && item.searching;
    }

    readonly property Terminal current: {
        void window.terminalsRevision;
        return window.terminals[workspace.focusedPane] ?? null;
    }

    visible: !App.dropdown
    width: 900
    height: 560
    title: window.current !== null && window.current.title.length > 0 ? window.current.title : Settings.title
    color: Qt.rgba(window.background.r, window.background.g, window.background.b, Settings.opacity)
    flags: App.dropdown ? (Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint) : (Settings.decorations ? Qt.Window : (Qt.Window | Qt.FramelessWindowHint))

    Workspace {
        id: workspace

        onRevisionChanged: if (!window.holding)
            window.sync()
    }

    ListModel {
        id: paneModel
    }

    ListModel {
        id: dividerModel
    }

    function indexIn(model, key, value) {
        for (let i = 0; i < model.count; ++i) {
            if (model.get(i)[key] === value)
                return i;
        }
        return -1;
    }

    // Brings the models in line with the layout, changing rows in place.
    function sync() {
        const panes = workspace.panes();
        const ids = panes.map(p => p.pane);
        for (let i = paneModel.count - 1; i >= 0; --i) {
            if (ids.indexOf(paneModel.get(i).pane) < 0)
                paneModel.remove(i);
        }
        for (const p of panes) {
            const row = {
                pane: p.pane,
                tab: p.tab,
                rx: p.x,
                ry: p.y,
                rw: p.width,
                rh: p.height,
                focused: p.focused,
                shown: p.visible
            };
            const at = window.indexIn(paneModel, "pane", p.pane);
            if (at < 0)
                paneModel.append(row);
            else
                paneModel.set(at, row);
        }
        const dividers = workspace.dividers();
        const paths = dividers.map(d => d.path);
        for (let i = dividerModel.count - 1; i >= 0; --i) {
            if (paths.indexOf(dividerModel.get(i).path) < 0)
                dividerModel.remove(i);
        }
        for (const d of dividers) {
            const row = {
                path: d.path,
                vertical: d.vertical,
                ratio: d.ratio,
                rx: d.x,
                ry: d.y,
                rw: d.width,
                rh: d.height
            };
            const at = window.indexIn(dividerModel, "path", d.path);
            if (at < 0)
                dividerModel.append(row);
            else
                dividerModel.set(at, row);
        }
    }

    // Where a pane is: where its shell last said, else where it started.
    function directoryOf(pane) {
        const t = window.terminalOf(pane);
        if (t !== null && t.workingDirectory.length > 0)
            return t.workingDirectory;
        return (window.starts[pane] ?? {}).directory ?? "";
    }

    // A path on another machine is data for display and the hook's
    // environment, never a directory for a process started here.
    function localDirectoryOf(pane) {
        return window.hostOf(pane).length > 0 ? "" : window.directoryOf(pane);
    }

    function directoryOfCurrent() {
        return window.directoryOf(workspace.focusedPane);
    }

    // The `[hosts]` name the pane runs on; empty for this machine.
    function hostOf(pane) {
        return (window.starts[pane] ?? {}).host ?? "";
    }

    // How the agents in `panes` are, the one most in need first: one
    // waiting, else one working, else one idle, else "none".
    function agentStateOf(panes) {
        const order = ["none", "idle", "working", "waiting"];
        let most = 0;
        for (const pane of panes) {
            const t = window.terminalOf(pane);
            if (t !== null)
                most = Math.max(most, order.indexOf(t.agentState));
        }
        return order[most];
    }

    // Focuses the next pane, after the focused one in reading order and
    // across tabs, whose agent waits for the user. Returns whether there
    // was one.
    function focusNextWaiting() {
        const panes = workspace.panes().map(p => p.pane);
        const from = panes.indexOf(workspace.focusedPane);
        for (let i = 1; i <= panes.length; ++i) {
            const pane = panes[(from + i) % panes.length];
            const t = window.terminalOf(pane);
            if (t !== null && t.agentState === "waiting" && pane !== workspace.focusedPane)
                return workspace.focusPane(pane);
        }
        return false;
    }

    function notificationCommand(title, where) {
        return JSON.stringify(["notify-send", "--app-name=Hof", "--icon=utilities-terminal", "--", title, where]);
    }

    // Tells the desktop that the agent in `pane` waits, unless the user
    // can see it already: the pane has the keys of the window they are in.
    function notifyWaiting(pane, shown, focused) {
        if (!Settings.notifyAgentWaiting || (window.active && window.visible && shown && focused))
            return;
        const t = window.terminalOf(pane);
        if (t === null)
            return;
        const host = window.hostOf(pane);
        const where = t.title.length > 0 ? t.title : window.directoryOf(pane);
        const title = t.agent + " is waiting" + (host.length > 0 ? " on " + host : "");
        App.spawn(JSON.parse(window.notificationCommand(title, where)), [], "");
    }

    function newTab(directory, command, environment, host) {
        const on = host ?? "";
        // The focused pane's directory means nothing on another machine.
        const from = window.hostOf(workspace.focusedPane) === on ? window.directoryOfCurrent() : "";
        window.holding = true;
        const pane = workspace.newTab();
        window.holding = false;
        window.starts[pane] = {
            directory: directory ?? from,
            command: command ?? [],
            environment: environment ?? [],
            host: on
        };
        window.sync();
        return pane;
    }

    function split(toward, command, environment, host) {
        // A split runs on the machine of the pane it splits, unless told.
        const from = window.hostOf(workspace.focusedPane);
        const on = host ?? from;
        const directory = on === from ? window.directoryOfCurrent() : "";
        window.holding = true;
        const pane = workspace.split(toward);
        window.holding = false;
        if (pane > 0)
            window.starts[pane] = {
                directory: directory,
                command: command ?? [],
                environment: environment ?? [],
                host: on
            };
        window.sync();
        return pane;
    }

    // What a program Hof starts for a pane is told, beyond the config's
    // environment: the pane, how to reach Hof, and where the pane is.
    function environmentOf(pane, pluginDir) {
        const env = ["HOF_PANE_ID=" + pane, "HOF_SOCKET=" + App.controlSocket, "HOF_DIRECTORY=" + window.directoryOf(pane)];
        if (pluginDir.length > 0)
            env.push("HOF_PLUGIN_DIR=" + pluginDir);
        return Settings.environment.concat(env);
    }

    // Runs a `[commands]` entry by name, from the focused pane: types its
    // text there, or runs its program in a tab, a split or the background.
    function runCommand(name) {
        const c = window.commands[name];
        if (c === undefined) {
            console.warn("hof: no command called " + name);
            return false;
        }
        const pane = workspace.focusedPane;
        if (c.send !== null) {
            const t = window.terminalOf(pane);
            if (t === null)
                return false;
            t.send(c.send);
            return true;
        }
        const plugin = c.pluginDir.length > 0 ? ["HOF_PLUGIN_DIR=" + c.pluginDir] : [];
        switch (c.in) {
        case "background":
            return App.spawn(c.run, window.environmentOf(pane, c.pluginDir), window.localDirectoryOf(pane));
        case "split-right":
        case "split-down":
            return window.split(c.in.slice(6), c.run, plugin) > 0;
        default:
            return window.newTab(undefined, c.run, plugin) > 0;
        }
    }

    // Runs the `[hooks]` for `event` in `pane`, with what the event says
    // (`NAME=value` each) in `extra`.
    function fireHook(pane, event, extra) {
        const hooks = window.hooks[event];
        if (hooks === undefined)
            return;
        const directory = window.localDirectoryOf(pane);
        for (const h of hooks)
            App.spawn(h.run, window.environmentOf(pane, h.pluginDir).concat(["HOF_EVENT=" + event], extra), directory);
    }

    // What the MCP server asks of the window (see `src/mcp.rs`): plain
    // values in and out, JSON where there is more than one.
    function mcpPanes() {
        const out = [];
        for (const p of workspace.panes()) {
            const t = window.terminalOf(p.pane);
            out.push({
                id: p.pane,
                tab: p.tab,
                title: t !== null ? t.title : "",
                workingDirectory: window.directoryOf(p.pane),
                focused: p.pane === workspace.focusedPane,
                running: t !== null && t.commandRunning,
                agent: t !== null ? t.agent : "",
                agentState: t !== null ? t.agentState : "none",
                columns: t !== null ? t.columns : 0,
                rows: t !== null ? t.rows : 0
            });
        }
        return JSON.stringify(out);
    }

    function mcpNewTab(directory, command, host) {
        return window.newTab(directory.length > 0 ? directory : undefined, command, undefined, host);
    }

    // `host` null for the split pane's.
    function mcpSplit(pane, toward, command, host) {
        if (!workspace.focusPane(pane))
            return 0;
        return window.split(toward, command, undefined, host);
    }

    function mcpFocus(pane) {
        return workspace.focusPane(pane);
    }

    // Closes a pane whose program is done or that the user closed. The
    // last pane closes the window; a dropdown hides and starts afresh.
    function closePane(pane) {
        const closed = workspace.closePane(pane);
        if (closed !== "everything")
            return;
        if (App.dropdown) {
            window.hideDropdown();
            window.newTab("", []);
        } else {
            // Everything was closed on purpose: the next start is a fresh one.
            if (Settings.sessionRestore)
                App.clearSession();
            Qt.quit();
        }
    }

    // Saves the tabs and splits, and where each pane was and what it
    // showed, for the next start to open again.
    function saveSession() {
        const panes = {};
        for (const pane in window.terminals) {
            const t = window.terminals[pane];
            const start = window.starts[pane] ?? {};
            panes[pane] = {
                directory: window.directoryOf(pane),
                command: start.command ?? [],
                title: t.title,
                scrollback: Settings.sessionSaveScrollback ? t.snapshot() : "",
                host: t.host,
                // Its program runs on there, to attach to again.
                remote: t.remotePane >= 0 ? t.remotePane : null
            };
        }
        return App.saveSession(workspace, JSON.stringify(panes), window.width, window.height);
    }

    // Opens the last session in place of a first tab, when the settings
    // ask for it and the command line did not ask for something else.
    function restoreSession() {
        if (!Settings.sessionRestore || App.dropdown || App.command.length > 0)
            return false;
        window.holding = true;
        const text = App.restoreSession(workspace);
        window.holding = false;
        if (text.length === 0)
            return false;
        const saved = JSON.parse(text);
        for (const pane in saved.panes)
            window.starts[pane] = saved.panes[pane];
        if (saved.width > 0 && saved.height > 0) {
            window.sized = true;
            window.width = saved.width;
            window.height = saved.height;
        }
        window.sync();
        return true;
    }

    onClosing: close => {
        if (Settings.sessionRestore && !App.dropdown)
            window.saveSession();
    }

    function register(pane, terminal, item) {
        const opened = window.terminals[pane] === undefined;
        window.terminals[pane] = terminal;
        window.paneItems[pane] = item;
        window.terminalsRevision++;
        // Both: the cell's width is known a moment before its height.
        if (!window.sized && terminal.cellWidth > 0 && terminal.cellHeight > 0) {
            window.sized = true;
            const chrome = (tabBar.visible ? tabBar.height : 0) + menuBar.height;
            window.width = Math.round(Settings.columns * terminal.cellWidth + 2 * terminal.padding);
            window.height = Math.round(Settings.rows * terminal.cellHeight + 2 * terminal.padding + chrome);
        }
        if (opened)
            window.fireHook(pane, "pane-opened", []);
    }

    function unregister(pane) {
        window.fireHook(pane, "pane-closed", []);
        delete window.terminals[pane];
        delete window.paneItems[pane];
        window.terminalsRevision++;
    }

    function terminalOf(pane) {
        void window.terminalsRevision;
        return window.terminals[pane] ?? null;
    }

    // The chord the menu bar shows for `action`: the first one bound.
    function menuKey(action) {
        const i = Settings.bindingActions.indexOf(action);
        return i >= 0 ? Settings.bindingSequences[i] : "";
    }

    // The same chord as a menu in the window writes it: as the keys are
    // marked on a Mac, else as Qt writes it.
    function menuKeyText(action) {
        const key = window.menuKey(action);
        if (Qt.platform.os !== "osx" || key.length === 0)
            return key;
        const marks = { "Meta": "⌃", "Ctrl": "⌘", "Alt": "⌥", "Shift": "⇧" };
        const parts = key.split("+");
        // `Ctrl++`: the last part is the key, even when it is a plus.
        const last = key.endsWith("++") ? "+" : parts[parts.length - 1];
        const mods = parts.slice(0, key.endsWith("++") ? -2 : -1);
        return mods.map(m => marks[m] ?? m).join("") + last;
    }

    // Gives the keys back to the focused pane, once a menu or the
    // settings close, unless something else was opened meanwhile.
    function refocus() {
        if (settingsDialog.opened || windowMenu.opened || hostPicker.visible)
            return;
        const item = window.paneItems[workspace.focusedPane];
        if (item !== undefined)
            item.giveFocus();
    }

    // An action a key binding or the menu names.
    function run(action) {
        const t = window.current;
        switch (action) {
        case "new-tab":
            window.newTab();
            break;
        case "close-tab":
            for (const pane of workspace.tabPanes(workspace.currentTab))
                window.closePane(pane);
            break;
        case "close-pane":
            if (t !== null)
                window.closePane(workspace.focusedPane);
            break;
        case "next-tab":
            workspace.nextTab();
            break;
        case "previous-tab":
            workspace.previousTab();
            break;
        case "split-right":
            window.split("right");
            break;
        case "split-down":
            window.split("down");
            break;
        case "focus-left":
        case "focus-right":
        case "focus-up":
        case "focus-down":
            workspace.focusToward(action.slice(6));
            break;
        case "grow-left":
        case "grow-right":
        case "grow-up":
        case "grow-down":
            workspace.grow(action.slice(5), 0.05);
            break;
        case "zoom-pane":
            workspace.toggleZoom();
            break;
        case "copy":
            if (t !== null)
                t.copy();
            break;
        case "paste":
            if (t !== null)
                t.paste();
            break;
        case "select-all":
            if (t !== null)
                t.selectAll();
            break;
        case "scroll-page-up":
            if (t !== null)
                t.scrollLines(-Math.max(1, t.rows - 1));
            break;
        case "scroll-page-down":
            if (t !== null)
                t.scrollLines(Math.max(1, t.rows - 1));
            break;
        case "scroll-top":
            if (t !== null)
                t.scrollToTop();
            break;
        case "scroll-bottom":
            if (t !== null)
                t.scrollToBottom();
            break;
        case "previous-prompt":
            if (t !== null)
                t.scrollToPrompt(-1);
            break;
        case "next-prompt":
            if (t !== null)
                t.scrollToPrompt(1);
            break;
        case "font-bigger":
            window.zoom += 1;
            break;
        case "font-smaller":
            window.zoom -= 1;
            break;
        case "font-reset":
            window.zoom = 0;
            break;
        case "reset":
            if (t !== null)
                t.reset();
            break;
        case "search": {
            const item = window.paneItems[workspace.focusedPane];
            if (item !== undefined)
                item.openSearch();
            break;
        }
        case "next-waiting":
            window.focusNextWaiting();
            break;
        case "new-tab-on-host":
            hostPicker.open("tab");
            break;
        case "split-right-on-host":
            hostPicker.open("right");
            break;
        case "split-down-on-host":
            hostPicker.open("down");
            break;
        case "reload-config":
            Settings.reload();
            break;
        case "toggle-fullscreen":
            window.visibility = window.visibility === Window.FullScreen ? Window.Windowed : Window.FullScreen;
            break;
        case "minimize":
            window.showMinimized();
            break;
        case "maximize":
            if (window.visibility === Window.Maximized)
                window.showNormal();
            else
                window.showMaximized();
            break;
        case "toggle-dropdown":
            if (App.dropdown)
                window.toggleDropdown();
            break;
        case "focus-next":
        case "focus-previous": {
            const panes = workspace.tabPanes(workspace.currentTab);
            const at = panes.indexOf(workspace.focusedPane);
            const by = action === "focus-next" ? 1 : -1;
            if (panes.length > 1)
                workspace.focusPane(panes[(at + by + panes.length) % panes.length]);
            break;
        }
        case "tab-1":
        case "tab-2":
        case "tab-3":
        case "tab-4":
        case "tab-5":
        case "tab-6":
        case "tab-7":
        case "tab-8":
            workspace.selectTab(Number(action.slice(4)) - 1);
            break;
        case "last-tab":
            workspace.selectTab(workspace.tabCount - 1);
            break;
        case "edit-config":
            App.editConfig();
            break;
        case "settings":
            settingsDialog.open();
            break;
        case "quit":
            // As the window manager's close: the session is saved first.
            window.close();
            break;
        default:
            if (action.startsWith("command:"))
                window.runCommand(action.slice(8));
            else
                console.warn("hof: no action called " + action);
        }
    }

    // The dropdown: where it sits, and its slide in and out. On a desktop
    // that places windows itself (most Wayland compositors) the position
    // is a hint; a window rule puts it at the edge.
    property real shownFraction: App.dropdown ? 0 : 1

    Behavior on shownFraction {
        enabled: App.dropdown && Settings.animationMs > 0

        NumberAnimation {
            duration: Settings.animationMs
            easing.type: Easing.OutCubic
        }
    }
    onShownFractionChanged: if (App.dropdown && window.shownFraction === 0 && window.hiding) {
        window.hiding = false;
        window.visible = false;
    }
    property bool hiding: false

    function showDropdown() {
        window.hiding = false;
        if (!window.visible) {
            window.width = Math.round(Screen.width * Settings.dropdownWidth);
            window.height = Math.round(Screen.height * Settings.dropdownHeight);
            window.x = Screen.virtualX + Math.round((Screen.width - window.width) / 2);
            window.y = Screen.virtualY + (Settings.dropdownBottom ? Screen.height - window.height : 0);
            window.visible = true;
        }
        window.raise();
        window.requestActivate();
        window.shownFraction = 1;
        if (window.current !== null)
            window.current.forceActiveFocus();
    }

    function hideDropdown() {
        if (!window.visible)
            return;
        window.hiding = true;
        window.shownFraction = 0;
        if (Settings.animationMs <= 0) {
            window.hiding = false;
            window.visible = false;
        }
    }

    function toggleDropdown() {
        if (window.visible && !window.hiding)
            window.hideDropdown();
        else
            window.showDropdown();
    }

    onActiveChanged: if (App.dropdown && !window.active && Settings.hideOnFocusLoss && window.visible)
        window.hideDropdown()

    Connections {
        target: App

        function onToggleRequested() {
            window.toggleDropdown();
        }

        function onTabRequested(command, directory) {
            window.newTab(directory, command);
            window.showDropdown();
        }
    }

    Item {
        id: content

        width: window.width
        height: window.height
        // A dropdown slides in from its edge.
        y: Math.round((1 - window.shownFraction) * window.height * (Settings.dropdownBottom ? 1 : -1))

        TabBar {
            id: tabBar

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: menuBar.bottom
            visible: workspace.tabCount > 1 || window.menuButtonShown
            height: visible ? implicitHeight : 0
            workspace: workspace
            menuButton: window.menuButtonShown
            background: window.background
            foreground: window.foreground
            border: window.border
            fontFamily: Settings.fontFamily
            titleOf: pane => {
                const t = window.terminalOf(pane);
                const title = t !== null && t.title.length > 0 ? t.title : Settings.title;
                const host = window.hostOf(pane);
                return host.length > 0 ? host + ": " + title : title;
            }
            agentStateOf: index => {
                void window.terminalsRevision;
                return window.agentStateOf(workspace.tabPanes(index));
            }
            waitingColor: Settings.palette.length > 3 ? Settings.palette[3] : "orange"
            workingColor: Settings.palette.length > 4 ? Settings.palette[4] : "steelblue"
            idleColor: Settings.palette.length > 2 ? Settings.palette[2] : "seagreen"
            onNewTab: window.newTab()
            onMenuRequested: from => {
                if (windowMenu.opened)
                    windowMenu.close();
                else
                    windowMenu.popup(from, from.width - windowMenu.width, from.height);
            }
            onCloseTab: index => {
                for (const pane of workspace.tabPanes(index))
                    window.closePane(pane);
            }
        }

        Item {
            id: area

            anchors.top: tabBar.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom

            Repeater {
                model: paneModel

                delegate: Pane {
                    id: slot

                    required property real rx
                    required property real ry
                    required property real rw
                    required property real rh

                    x: Math.round(slot.rx * area.width)
                    y: Math.round(slot.ry * area.height)
                    width: Math.round((slot.rx + slot.rw) * area.width) - x
                    height: Math.round((slot.ry + slot.rh) * area.height) - y
                    visible: slot.shown
                    start: window.starts[slot.pane] ?? ({
                            directory: App.workingDirectory,
                            command: App.command
                        })
                    fontSize: window.fontSize
                    background: window.background
                    foreground: window.foreground
                    keyOf: window.menuKeyText
                    onTerminalReady: (terminal, item) => window.register(slot.pane, terminal, item)
                    onTerminalGone: window.unregister(slot.pane)
                    onFocusTaken: workspace.focusPane(slot.pane)
                    onFinished: window.closePane(slot.pane)
                    onAction: name => window.run(name)
                    onHook: (event, extra) => window.fireHook(slot.pane, event, extra)
                    onAttention: window.notifyWaiting(slot.pane, slot.shown, slot.focused)
                }
            }

            Repeater {
                model: dividerModel

                delegate: Divider {
                    area: area
                    workspace: workspace
                    color: window.border
                }
            }
        }

        // The menu bar: the system's own on macOS, where it takes no room
        // in the window, and across the top of it elsewhere. In a file of
        // its own: QtQuick.Controls has a Pane and a TabBar, which would
        // take the place of Hof's here.
        Loader {
            id: menuBar

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: item !== null ? item.height : 0
            active: window.menuBarShown

            sourceComponent: SystemMenuBar {
                runner: window
            }
        }

        // Every binding the config file names, as a window-wide shortcut:
        // it wins over the terminal, which never claims a key first.
        Repeater {
            model: Settings.bindingActions.length

            delegate: Item {
                id: binding

                required property int index

                Shortcut {
                    sequence: Settings.bindingSequences[binding.index]
                    // The menu bar's item has this chord, and the system
                    // gives the key to it; two of a chord would cancel out.
                    enabled: {
                        const action = Settings.bindingActions[binding.index];
                        return !(window.menuBarShown && window.menuActions.includes(action) && Settings.bindingActions.indexOf(action) === binding.index);
                    }
                    context: Qt.WindowShortcut
                    onActivated: window.run(Settings.bindingActions[binding.index])
                }
            }
        }

        // Where a new tab or split runs, for the `...-on-host` keys.
        HostPicker {
            id: hostPicker

            anchors.horizontalCenter: parent.horizontalCenter
            y: Math.round(parent.height / 5)
            z: 5
            hosts: Settings.hosts
            background: window.background
            foreground: window.foreground
            border: Qt.rgba(window.foreground.r, window.foreground.g, window.foreground.b, 0.25)
            fontFamily: Settings.fontFamily
            fontSize: window.fontSize

            onPicked: (host, purpose) => {
                if (purpose === "tab")
                    window.newTab(undefined, undefined, undefined, host);
                else
                    window.split(purpose, undefined, undefined, host);
            }
            onClosed: if (window.current !== null)
                window.current.forceActiveFocus()
        }

        // The menu the tab bar's button opens.
        WindowMenu {
            id: windowMenu

            keyOf: window.menuKeyText
            baseColor: window.background
            textColor: window.foreground
            fontFamily: Settings.fontFamily
            fontSize: Math.max(8, Settings.fontSize - 1)

            onChose: name => window.run(name)
            onClosed: Qt.callLater(window.refocus)
        }

        SettingsDialog {
            id: settingsDialog

            baseColor: window.background
            textColor: window.foreground
            accent: Settings.palette.length > 4 ? Settings.palette[4] : "steelblue"
            danger: Settings.palette.length > 1 ? Settings.palette[1] : "firebrick"

            onClosed: Qt.callLater(window.refocus)
        }

        // What was wrong with the config file, until dismissed.
        Problems {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 12
            problems: Settings.problems
            background: window.background
            foreground: window.foreground
        }
    }

    Component.onCompleted: {
        // First, so the first panes are told where it is.
        App.serve(window);
        if (!window.restoreSession())
            window.newTab(App.workingDirectory, App.command);
        if (App.dropdown)
            window.showDropdown();
    }
}
