import QtQuick
import QtQuick.Controls

// What a right click in a pane offers.
HofMenu {
    id: menu

    // The pane's terminal has text selected.
    property bool canCopy: false

    objectName: "paneMenu"

    HofMenuItem { owner: menu; name: "copy"; text: "Copy"; enabled: menu.canCopy }
    HofMenuItem { owner: menu; name: "paste"; text: "Paste" }
    HofMenuItem { owner: menu; name: "select-all"; text: "Select All" }
    HofMenuSeparator { owner: menu }
    HofMenuItem { owner: menu; name: "split-right"; text: "Split Right" }
    HofMenuItem { owner: menu; name: "split-down"; text: "Split Down" }
    HofMenuItem { owner: menu; name: "new-tab"; text: "New Tab" }
    HofMenuSeparator { owner: menu }
    HofMenuItem { owner: menu; name: "search"; text: "Find…" }
    HofMenuItem { owner: menu; name: "reset"; text: "Reset Terminal" }
    HofMenuSeparator { owner: menu }
    HofMenuItem { owner: menu; name: "settings"; text: "Settings…" }
    HofMenuItem { owner: menu; name: "close-pane"; text: "Close Pane" }
}
