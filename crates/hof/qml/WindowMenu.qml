import QtQuick
import Hof.Bricks

// The menu the tab bar's button opens, where the system has no menu bar
// of its own: what the window can do, by name.
HofMenu {
    id: menu

    objectName: "windowMenu"

    HofMenuItem { owner: menu; name: "new-tab"; text: "New Tab"; iconName: "plus" }
    HofMenuItem { owner: menu; name: "new-tab-on-host"; text: "New Tab on Host…"; iconName: "server" }
    HofMenuItem { owner: menu; name: "split-right"; text: "Split Right" }
    HofMenuItem { owner: menu; name: "split-down"; text: "Split Down" }
    ShellMenuSeparator {}
    HofMenuItem { owner: menu; name: "search"; text: "Find…" }
    HofMenuItem { owner: menu; name: "font-bigger"; text: "Bigger Text" }
    HofMenuItem { owner: menu; name: "font-smaller"; text: "Smaller Text" }
    HofMenuItem { owner: menu; name: "font-reset"; text: "Actual Size" }
    HofMenuItem { owner: menu; name: "zoom-pane"; text: "Zoom Pane" }
    HofMenuItem { owner: menu; name: "toggle-fullscreen"; text: "Full Screen" }
    ShellMenuSeparator {}
    HofMenuItem { owner: menu; name: "settings"; text: "Settings…"; iconName: "settings" }
    HofMenuItem { owner: menu; name: "edit-config"; text: "Open Config File"; iconName: "file-text" }
    ShellMenuSeparator {}
    HofMenuItem { owner: menu; name: "close-tab"; text: "Close Tab"; iconName: "x" }
    HofMenuItem { owner: menu; name: "quit"; text: "Quit" }
}
