import QtQuick
import QtQuick.Controls

// The system's menu bar, where it has one (macOS): Hof's actions, each
// with the first chord bound to it. Settings… and Quit are moved to the
// application menu there.
MenuBar {
    id: bar

    // The window whose actions it runs.
    required property var runner

    Menu {
        title: "File"
        MenuAction { name: "new-tab"; text: "New Tab"; runner: bar.runner }
        MenuAction { name: "new-tab-on-host"; text: "New Tab on Host…"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "split-right"; text: "Split Right"; runner: bar.runner }
        MenuAction { name: "split-down"; text: "Split Down"; runner: bar.runner }
        MenuAction { name: "split-right-on-host"; text: "Split Right on Host…"; runner: bar.runner }
        MenuAction { name: "split-down-on-host"; text: "Split Down on Host…"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "close-pane"; text: "Close Pane"; runner: bar.runner }
        MenuAction { name: "close-tab"; text: "Close Tab"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "edit-config"; text: "Settings…"; runner: bar.runner }
        MenuAction { name: "reload-config"; text: "Reload Settings"; runner: bar.runner }
        MenuAction { name: "quit"; text: "Quit"; runner: bar.runner }
    }
    Menu {
        title: "Edit"
        MenuAction { name: "copy"; text: "Copy"; runner: bar.runner }
        MenuAction { name: "paste"; text: "Paste"; runner: bar.runner }
        MenuAction { name: "select-all"; text: "Select All"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "search"; text: "Find…"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "reset"; text: "Reset Terminal"; runner: bar.runner }
    }
    Menu {
        title: "View"
        MenuAction { name: "font-bigger"; text: "Bigger Text"; runner: bar.runner }
        MenuAction { name: "font-smaller"; text: "Smaller Text"; runner: bar.runner }
        MenuAction { name: "font-reset"; text: "Actual Size"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "zoom-pane"; text: "Zoom Pane"; runner: bar.runner }
        MenuAction { name: "toggle-fullscreen"; text: "Full Screen"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "previous-prompt"; text: "Previous Prompt"; runner: bar.runner }
        MenuAction { name: "next-prompt"; text: "Next Prompt"; runner: bar.runner }
        MenuAction { name: "scroll-top"; text: "Scroll to Top"; runner: bar.runner }
        MenuAction { name: "scroll-bottom"; text: "Scroll to Bottom"; runner: bar.runner }
    }
    Menu {
        title: "Window"
        MenuAction { name: "previous-tab"; text: "Previous Tab"; runner: bar.runner }
        MenuAction { name: "next-tab"; text: "Next Tab"; runner: bar.runner }
        MenuSeparator {}
        MenuAction { name: "focus-previous"; text: "Previous Pane"; runner: bar.runner }
        MenuAction { name: "focus-next"; text: "Next Pane"; runner: bar.runner }
        Menu {
            title: "Go to Pane"
            MenuAction { name: "focus-left"; text: "Left"; runner: bar.runner }
            MenuAction { name: "focus-right"; text: "Right"; runner: bar.runner }
            MenuAction { name: "focus-up"; text: "Above"; runner: bar.runner }
            MenuAction { name: "focus-down"; text: "Below"; runner: bar.runner }
        }
        Menu {
            title: "Resize Pane"
            MenuAction { name: "grow-left"; text: "Grow Left"; runner: bar.runner }
            MenuAction { name: "grow-right"; text: "Grow Right"; runner: bar.runner }
            MenuAction { name: "grow-up"; text: "Grow Up"; runner: bar.runner }
            MenuAction { name: "grow-down"; text: "Grow Down"; runner: bar.runner }
        }
        MenuSeparator {}
        MenuAction { name: "next-waiting"; text: "Next Waiting Agent"; runner: bar.runner }
    }
}
