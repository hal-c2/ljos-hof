//! Hof's window, as the QML builds it: tabs and split panes, each pane a
//! terminal that lives as long as its pane.

use ljos::item::{KeyEvent, Modifiers};
use ljos::prelude::*;
use ljos::testing::TestWindow;

/// `Qt::Key_T`.
const KEY_T: i32 = 0x54;

struct Hof {
    engine: Engine,
    window: TestWindow,
    root: ObjectId,
}

fn opened() -> Hof {
    // The defaults, whatever the user's own config says, and a first pane
    // that prints nothing of its own, whatever the user's shell does.
    hof::set_launch(hof::Launch {
        command: vec!["cat".to_owned()],
        config: Some(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("none/hof.toml")),
        ..hof::Launch::default()
    });
    let (engine, root) = hof::app().build().expect("the window builds");
    let mut window = TestWindow::new(&engine, 800.0, 400.0);
    window.show(root).expect("the window shows");
    window.settle().unwrap();
    Hof {
        engine,
        window,
        root,
    }
}

impl Hof {
    fn run(&mut self, action: &str) {
        self.engine
            .invoke_by_name(self.root, "run", &[Value::from(action)])
            .unwrap();
        self.window.settle().unwrap();
    }

    fn number(&self, obj: ObjectId, name: &str) -> f64 {
        self.engine.read_by_name(obj, name).to_number()
    }

    fn find(&self, name: &str) -> Option<ObjectId> {
        let mut items = vec![self.root];
        while let Some(item) = items.pop() {
            if self
                .engine
                .read_by_name(item, "objectName")
                .to_js_string()
                .as_str()
                == name
            {
                return Some(item);
            }
            items.extend(self.engine.children(item));
        }
        None
    }

    fn panes(&self) -> usize {
        self.number(self.root, "paneCount") as usize
    }

    fn layout(&self) -> ObjectId {
        self.engine
            .read_by_name(self.root, "layout")
            .as_object()
            .unwrap()
    }

    fn current(&self) -> Option<ObjectId> {
        self.engine.read_by_name(self.root, "current").as_object()
    }

    /// The window's width and height: it sizes itself to the cells the
    /// settings ask for once it knows its font.
    fn size(&self) -> (f64, f64) {
        (
            self.number(self.root, "width"),
            self.number(self.root, "height"),
        )
    }

    fn shows_tabs(&self) -> bool {
        self.engine
            .read_by_name(self.root, "tabsShown")
            .to_boolean()
    }
}

#[test]
fn the_window_opens_with_one_terminal() {
    let hof = opened();
    assert_eq!(hof.panes(), 1);
    let t = hof.current().expect("a terminal has the keys");
    assert!(hof.number(t, "columns") > 0.0);
    // Off macOS the window's menu is a button in the tab bar, which is
    // there for it with one tab; on a Mac it is the system's menu bar.
    let button = !cfg!(target_os = "macos");
    assert_eq!(
        hof.shows_tabs(),
        button,
        "the tab bar is there for the menu"
    );
    let menu = hof.find("menuButton").unwrap();
    assert_eq!(
        hof.engine.read_by_name(menu, "visible").to_boolean(),
        button
    );
}

#[test]
fn the_window_is_as_many_cells_as_the_settings_ask_and_its_program_runs() {
    let hof = opened();
    let t = hof.current().unwrap();
    // The defaults: 100 by 30, though the test window was made 800 by 400.
    assert_eq!(hof.number(t, "columns"), 100.0);
    assert_eq!(hof.number(t, "rows"), 30.0);
    assert!(hof.engine.read_by_name(t, "running").to_boolean());
}

#[test]
fn splitting_right_puts_a_second_terminal_beside_the_first() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("split-right");
    assert_eq!(hof.panes(), 2);
    let second = hof.current().unwrap();
    assert_ne!(first, second, "the new pane has the keys");
    let (w, _) = hof.size();
    let (a, b) = (hof.number(first, "width"), hof.number(second, "width"));
    // Each is the half less its frame: 2 apart and 2 of line each side.
    let frame = 8.0;
    assert!((a + frame - w / 2.0).abs() <= 1.0, "first is {a} of {w}");
    assert!(
        (a + b + 2.0 * frame - w).abs() <= 0.5,
        "with their frames they fill {w}"
    );
}

#[test]
fn a_split_and_a_new_tab_start_where_the_focused_pane_is() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    // The shell says where it is (OSC 7).
    hof.engine
        .invoke_by_name(
            first,
            "write",
            &[Value::from("\x1b]7;file://localhost/usr\x07")],
        )
        .unwrap();
    hof.window.advance(50).unwrap();
    assert_eq!(
        hof.engine
            .read_by_name(first, "workingDirectory")
            .to_js_string()
            .as_str(),
        "/usr"
    );
    let start = |hof: &Hof| {
        let t = hof.current().unwrap();
        hof.engine
            .read_by_name(t, "startDirectory")
            .to_js_string()
            .as_str()
            .to_owned()
    };
    hof.run("split-right");
    assert_eq!(start(&hof), "/usr");
    // The split's program has said nothing of where it is: it is where
    // it started.
    hof.run("new-tab");
    assert_eq!(start(&hof), "/usr");
}

#[test]
fn a_terminal_outlives_the_splits_around_it() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    let height = hof.number(first, "height");
    hof.run("split-down");
    hof.run("split-right");
    assert_eq!(hof.panes(), 3);
    hof.run("close-pane");
    hof.run("close-pane");
    assert_eq!(hof.panes(), 1);
    assert_eq!(
        hof.current(),
        Some(first),
        "the same terminal, not a new one"
    );
    assert_eq!(hof.number(first, "height"), height);
}

#[test]
fn a_second_tab_shows_the_tab_bar_and_hides_the_first_tab() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("new-tab");
    assert!(hof.shows_tabs());
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
    let second = hof.current().unwrap();
    assert_ne!(first, second);
    hof.run("previous-tab");
    assert_eq!(hof.current(), Some(first));
}

impl Hof {
    /// The objectNames of every object below the window that start with
    /// `prefix`.
    fn names(&self, prefix: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut items = vec![self.root];
        while let Some(item) = items.pop() {
            let name = self.engine.read_by_name(item, "objectName").to_js_string();
            if name.as_str().starts_with(prefix) {
                found.push(name.as_str().to_owned());
            }
            items.extend(self.engine.children(item));
        }
        found
    }
}

#[test]
fn the_menu_bar_has_each_action_it_lists_with_its_key() {
    let mut hof = opened();
    let shown = cfg!(target_os = "macos");
    assert_eq!(hof.find("menu:new-tab").is_some(), shown);
    hof.engine
        .write_by_name(hof.root, "menuBarShown", Value::from(true))
        .unwrap();
    hof.window.settle().unwrap();
    let listed = hof.engine.read_by_name(hof.root, "menuActions");
    let listed: Vec<String> = listed
        .as_list()
        .expect("a list")
        .iter()
        .map(|a| a.to_js_string().as_str().to_owned())
        .collect();
    let mut made = hof.names("menu:");
    made.sort();
    let mut want: Vec<String> = listed.iter().map(|a| format!("menu:{a}")).collect();
    want.sort();
    assert_eq!(
        made, want,
        "the bar and `menuActions` name the same actions"
    );

    let new_tab = hof.find("menu:new-tab").unwrap();
    let key = if cfg!(target_os = "macos") {
        "Ctrl+T"
    } else {
        "Ctrl+Shift+T"
    };
    assert_eq!(
        hof.engine
            .read_by_name(new_tab, "shortcut")
            .to_js_string()
            .as_str(),
        key
    );
    hof.engine.invoke_by_name(new_tab, "trigger", &[]).unwrap();
    hof.window.settle().unwrap();
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
}

#[test]
fn a_key_the_menu_bar_shows_does_its_action_once() {
    let mut hof = opened();
    hof.engine
        .write_by_name(hof.root, "menuBarShown", Value::from(true))
        .unwrap();
    hof.window.settle().unwrap();
    let chord = if cfg!(target_os = "macos") {
        KeyEvent::press(KEY_T)
            .with_modifiers(Modifiers::CONTROL)
            .with_text("t")
    } else {
        KeyEvent::press(KEY_T)
            .with_modifiers(Modifiers::CONTROL | Modifiers::SHIFT)
            .with_text("T")
    };
    hof.window.key(&chord);
    hof.window.settle().unwrap();
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
}

#[test]
fn a_tab_can_be_gone_to_by_its_number_or_as_the_last() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("new-tab");
    hof.run("new-tab");
    let third = hof.current().unwrap();
    hof.run("tab-1");
    assert_eq!(hof.current(), Some(first));
    hof.run("last-tab");
    assert_eq!(hof.current(), Some(third));
    // There is no fifth: nothing happens.
    hof.run("tab-5");
    assert_eq!(hof.current(), Some(third));
}

#[test]
fn focus_goes_round_the_panes_of_a_tab_in_reading_order() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("split-right");
    let second = hof.current().unwrap();
    hof.run("focus-next");
    assert_eq!(hof.current(), Some(first));
    hof.run("focus-next");
    assert_eq!(hof.current(), Some(second));
    hof.run("focus-previous");
    assert_eq!(hof.current(), Some(first));
}

#[test]
fn zooming_gives_one_pane_the_whole_tab() {
    let mut hof = opened();
    hof.run("split-right");
    let t = hof.current().unwrap();
    let half = hof.number(t, "width");
    hof.run("zoom-pane");
    assert_eq!(hof.number(t, "width"), hof.size().0);
    hof.run("zoom-pane");
    assert_eq!(hof.number(t, "width"), half);
}

#[test]
fn the_font_grows_and_shrinks_back() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let cell = hof.number(t, "cellWidth");
    hof.run("font-bigger");
    hof.run("font-bigger");
    assert!(hof.number(t, "cellWidth") > cell);
    hof.run("font-reset");
    assert_eq!(hof.number(t, "cellWidth"), cell);
}

#[test]
fn a_bound_key_does_its_action_and_does_not_reach_the_program() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let typed = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let sink = std::rc::Rc::clone(&typed);
    hof.engine
        .connect_by_name(t, "input", move |_: &Engine, args: &[Value]| {
            sink.borrow_mut().push_str(args[0].to_js_string().as_str());
        })
        .unwrap();
    // Command+T on a Mac, which Qt calls Control there; Ctrl+Shift+T
    // elsewhere.
    let chord = if cfg!(target_os = "macos") {
        KeyEvent::press(KEY_T)
            .with_modifiers(Modifiers::CONTROL)
            .with_text("t")
    } else {
        KeyEvent::press(KEY_T)
            .with_modifiers(Modifiers::CONTROL | Modifiers::SHIFT)
            .with_text("T")
    };
    hof.window.key(&chord);
    hof.window.settle().unwrap();
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
    assert_eq!(*typed.borrow(), "", "the chord was the window's");
}

/// `Qt::Key_Escape`.
const KEY_ESCAPE: i32 = 0x0100_0000;

impl Hof {
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let code = c.to_ascii_uppercase() as i32;
            self.window
                .key(&KeyEvent::press(code).with_text(c.to_string()));
        }
        self.window.settle().unwrap();
    }

    fn searching(&self) -> bool {
        self.engine
            .read_by_name(self.root, "searchShown")
            .to_boolean()
    }
}

#[test]
fn searching_finds_as_you_type_and_escape_puts_it_away() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("zqxj and zqxj\r\n")])
        .unwrap();
    hof.run("search");
    assert!(hof.searching(), "the find bar shows");
    hof.type_text("zqxj");
    assert_eq!(hof.number(t, "searchTotal"), 2.0);
    assert_eq!(hof.number(t, "searchCurrent"), 1.0);

    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.searching());
    assert!(
        !hof.engine.read_by_name(t, "searching").to_boolean(),
        "the highlights go with the bar"
    );
    assert!(
        hof.engine.read_by_name(t, "activeFocus").to_boolean(),
        "the keys go back to the terminal"
    );
}

/// `Qt::Key_Return`.
const KEY_RETURN: i32 = 0x0100_0004;

impl Hof {
    fn confirming(&self) -> bool {
        self.engine
            .read_by_name(self.root, "confirmingPaste")
            .to_boolean()
    }

    fn paste_text(&mut self, t: ObjectId, text: &str) -> bool {
        let ok = self
            .engine
            .invoke_by_name(t, "pasteText", &[Value::from(text)])
            .unwrap()
            .to_boolean();
        self.window.settle().unwrap();
        ok
    }
}

#[test]
fn a_paste_of_several_lines_waits_for_a_yes() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    assert!(!hof.paste_text(t, "rm -rf ~\nls\n"), "held back");
    assert!(hof.confirming(), "the user is asked");

    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(!hof.confirming());
    assert!(
        hof.engine.read_by_name(t, "activeFocus").to_boolean(),
        "the keys go back to the terminal"
    );
}

#[test]
fn escape_turns_a_held_paste_down() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    hof.paste_text(t, "one\ntwo\n");
    assert!(hof.confirming());
    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.confirming());
}

#[test]
fn a_single_line_pastes_without_asking() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    assert!(hof.paste_text(t, "echo hi"));
    assert!(!hof.confirming());
}

#[test]
fn going_back_to_a_tab_gives_the_keys_to_its_pane() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("new-tab");
    hof.run("previous-tab");
    assert!(
        hof.engine.read_by_name(first, "activeFocus").to_boolean(),
        "typing goes to the tab shown"
    );
}

impl Hof {
    fn asking(&self) -> bool {
        self.engine.read_by_name(self.root, "asking").to_boolean()
    }
}

#[test]
fn a_program_reading_the_clipboard_waits_for_the_user() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    assert_eq!(
        hof.engine
            .read_by_name(t, "clipboardRead")
            .to_js_string()
            .as_str(),
        "ask"
    );
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b]52;c;?\x07")])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(hof.asking(), "the user is asked");

    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.asking());
    assert!(
        hof.engine.read_by_name(t, "activeFocus").to_boolean(),
        "the keys go back to the terminal"
    );
}

#[test]
fn a_program_asking_to_open_a_url_waits_for_the_user() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let url = String::from_utf8(hof_vt::open_request("https://ljos.dev")).unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from(url.as_str())])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(hof.asking(), "the user is asked");
    let detail = hof.find("permitDetail").expect("the URL is shown");
    assert_eq!(hof.number(detail, "textFormat"), 0.0, "plain text");
    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.asking());
}

impl Hof {
    /// Moves time on until `done`, or fails saying `what` it waited for.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(self) {
            assert!(std::time::Instant::now() < deadline, "waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.window.advance(20).unwrap();
        }
    }

    fn screen(&self, t: ObjectId) -> String {
        self.engine
            .invoke_by_name(t, "screenText", &[])
            .unwrap()
            .to_js_string()
            .as_str()
            .to_owned()
    }
}

#[test]
fn hof_copy_and_paste_in_a_pane_go_through_its_terminal() {
    let hof = env!("CARGO_BIN_EXE_hof");
    let script = format!("'{hof}' copy ljós && echo got:$('{hof}' paste); read x");
    let mut hof = opened();
    // In a tab of its own: what the first pane runs is the process's, and
    // other tests set it as they go.
    let command = Value::list(
        ["sh", "-c", &script]
            .iter()
            .map(|&a| Value::from(a))
            .collect(),
    );
    hof.engine
        .invoke_by_name(hof.root, "newTab", &[Value::from(""), command])
        .unwrap();
    hof.window.settle().unwrap();
    let t = hof.current().unwrap();
    hof.until("the paste to ask", |h| h.asking());
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.until("the paste", |h| h.screen(t).contains("got:ljós"));
}

#[test]
fn a_background_panes_request_cannot_take_the_keys_or_consent() {
    let mut hof = opened();
    let first = hof.current().unwrap();
    hof.run("new-tab");
    let second = hof.current().unwrap();
    hof.engine
        .invoke_by_name(first, "write", &[Value::from("\x1b]52;c;?\x07")])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(
        !hof.asking(),
        "the hidden request is not the focused pane's"
    );
    assert!(hof.engine.read_by_name(second, "activeFocus").to_boolean());

    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.key(&KeyEvent::press('A' as i32));
    hof.window.settle().unwrap();
    assert_eq!(hof.current(), Some(second), "the keys stay in this tab");
    assert!(hof.engine.read_by_name(second, "activeFocus").to_boolean());

    hof.run("previous-tab");
    assert_eq!(hof.current(), Some(first));
    assert!(hof.asking(), "neither stray key granted the hidden request");
    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(hof.asking(), "a plain Enter is not consent either");
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();
    assert!(!hof.asking());
    assert!(hof.engine.read_by_name(first, "activeFocus").to_boolean());
}

#[test]
fn a_prompt_closed_by_another_cannot_take_focus_back() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let open = String::from_utf8(hof_vt::open_request("https://a.example")).unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from(open.as_str())])
        .unwrap();
    hof.window.settle().unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b]52;c;?\x07")])
        .unwrap();
    hof.window.settle().unwrap();
    let clipboard = hof.find("clipboardPermit").unwrap();
    let open = hof.find("openPermit").unwrap();
    assert!(
        hof.engine
            .read_by_name(clipboard, "activeFocus")
            .to_boolean()
    );
    assert!(hof.number(clipboard, "z") > hof.number(open, "z"));
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();
    assert!(!hof.asking());
    assert!(hof.engine.read_by_name(t, "activeFocus").to_boolean());
}

#[test]
fn an_unsafe_paste_is_shown_above_a_clipboard_request() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b]52;c;?\x07")])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(!hof.paste_text(t, "one\ntwo\n"));
    let paste = hof.find("pasteConfirm").unwrap();
    let clipboard = hof.find("clipboardPermit").unwrap();
    assert!(hof.engine.read_by_name(paste, "activeFocus").to_boolean());
    assert!(hof.number(paste, "z") > hof.number(clipboard, "z"));
    hof.run("new-tab");
    hof.run("previous-tab");
    assert!(hof.engine.read_by_name(paste, "activeFocus").to_boolean());
    assert!(hof.number(paste, "z") > hof.number(clipboard, "z"));

    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(
        hof.engine
            .read_by_name(clipboard, "activeFocus")
            .to_boolean()
    );
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();
    assert!(!hof.asking());
    assert!(hof.engine.read_by_name(t, "activeFocus").to_boolean());
}

#[test]
fn the_prompt_drawn_last_wins_a_z_tie() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    assert!(!hof.paste_text(t, "one\ntwo\n"));
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b]52;c;?\x07")])
        .unwrap();
    hof.window.settle().unwrap();
    let open_request = String::from_utf8(hof_vt::open_request("https://a.example")).unwrap();
    hof.engine
        .invoke_by_name(t, "write", &[Value::from(open_request.as_str())])
        .unwrap();
    hof.window.settle().unwrap();
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();

    let clipboard = hof.find("clipboardPermit").unwrap();
    let paste = hof.find("pasteConfirm").unwrap();
    assert_eq!(hof.number(clipboard, "z"), hof.number(paste, "z"));
    assert!(
        hof.engine
            .read_by_name(clipboard, "activeFocus")
            .to_boolean()
    );
    hof.run("new-tab");
    hof.run("previous-tab");
    assert!(
        hof.engine
            .read_by_name(clipboard, "activeFocus")
            .to_boolean()
    );
    hof.window
        .key(&KeyEvent::press(KEY_RETURN).with_modifiers(Modifiers::CONTROL));
    hof.window.settle().unwrap();
    hof.run("new-tab");
    hof.run("previous-tab");
    assert!(hof.engine.read_by_name(paste, "activeFocus").to_boolean());
    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(hof.engine.read_by_name(t, "activeFocus").to_boolean());
}

impl Hof {
    fn picking_host(&self) -> bool {
        self.engine
            .read_by_name(self.root, "pickingHost")
            .to_boolean()
    }

    fn host_choices(&self) -> usize {
        self.number(self.root, "hostChoices") as usize
    }
}

#[test]
fn picking_this_machine_opens_a_tab_here() {
    let mut hof = opened();
    hof.run("new-tab-on-host");
    assert!(hof.picking_host(), "the picker shows");
    assert_eq!(hof.host_choices(), 1, "no [hosts], so only this machine");

    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(!hof.picking_host());
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
    let t = hof.current().unwrap();
    assert!(
        hof.engine.read_by_name(t, "activeFocus").to_boolean(),
        "the new pane has the keys"
    );
}

#[test]
fn a_name_no_host_has_leaves_nothing_to_pick() {
    let mut hof = opened();
    hof.run("split-right-on-host");
    hof.type_text("zqx");
    assert_eq!(hof.host_choices(), 0);
    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.window.settle().unwrap();
    assert!(hof.picking_host(), "still open: there was nothing to pick");
    assert_eq!(hof.panes(), 1);

    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.picking_host());
    assert_eq!(hof.panes(), 1, "escape opens nothing");
    let t = hof.current().unwrap();
    assert!(
        hof.engine.read_by_name(t, "activeFocus").to_boolean(),
        "the keys go back to the pane"
    );
}

#[test]
fn a_link_under_the_pointer_is_named_until_the_screen_moves_it_away() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    // A link over the whole screen, so wherever the pane is, the pointer
    // in the middle of the window is on it.
    let row = format!("{}\r\n", "y".repeat(150));
    let screen = format!(
        "\x1b]8;;https://example.com/hof\x1b\\{}\x1b]8;;\x1b\\",
        row.repeat(40)
    );
    hof.engine
        .invoke_by_name(t, "write", &[Value::from(screen.as_str())])
        .unwrap();
    hof.window.settle().unwrap();
    let now = hof.window.now();
    hof.window.pointer(&ljos::item::PointerEvent::moved(
        400.0,
        200.0,
        Default::default(),
        now,
    ));
    hof.window.settle().unwrap();
    let hovered = |hof: &Hof| hof.engine.read_by_name(t, "hoveredLink").to_js_string();
    assert_eq!(hovered(&hof), "https://example.com/hof");
    let text = hof.find("hoveredLinkText").expect("the link is named");
    assert_eq!(hof.number(text, "textFormat"), 0.0, "plain text");

    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b[2J\x1b[H")])
        .unwrap();
    hof.window.settle().unwrap();
    assert_eq!(hovered(&hof), "", "the screen was cleared under it");
}

#[test]
fn notification_text_cannot_become_an_option() {
    let hof = opened();
    let args = hof
        .engine
        .invoke_by_name(
            hof.root,
            "notificationCommand",
            &[Value::from("--wait"), Value::from("--replace-id=7")],
        )
        .unwrap();
    let args: Vec<String> = serde_json::from_str(args.to_js_string().as_str()).unwrap();
    assert_eq!(args[3], "--");
    assert_eq!(args[4], "--wait");
    assert_eq!(args[5], "--replace-id=7");
}

#[test]
fn a_remote_directory_is_not_a_local_process_working_directory() {
    let hof = opened();
    let empty = Value::list(Vec::new());
    let pane = hof
        .engine
        .invoke_by_name(
            hof.root,
            "newTab",
            &[
                Value::from("/tmp"),
                empty.clone(),
                empty,
                Value::from("dev"),
            ],
        )
        .unwrap()
        .to_number();
    assert!(pane > 0.0);
    assert_eq!(
        hof.engine
            .invoke_by_name(hof.root, "directoryOf", &[Value::from(pane)])
            .unwrap()
            .to_js_string()
            .as_str(),
        "/tmp"
    );
    assert_eq!(
        hof.engine
            .invoke_by_name(hof.root, "localDirectoryOf", &[Value::from(pane)])
            .unwrap()
            .to_js_string()
            .as_str(),
        ""
    );
}

impl Hof {
    /// A press and release in the middle of `item`, with `button`.
    fn click(&mut self, item: ObjectId, button: ljos::item::MouseButtons) {
        // As large as the window made itself, so the pointer can reach
        // all of it.
        let (width, height) = self.size();
        self.window.resize(width, height).unwrap();
        let at = ljos::item::scene_rect(&self.engine, item).center();
        let now = self.window.now();
        self.window
            .pointer(&ljos::item::PointerEvent::press(at.x, at.y, now).with_button(button));
        self.window
            .pointer(&ljos::item::PointerEvent::release(at.x, at.y, now).with_button(button));
        self.window.settle().unwrap();
        // Long enough for a menu to fade in or out.
        self.window.advance(200).unwrap();
    }

    fn flag(&self, name: &str) -> bool {
        self.engine.read_by_name(self.root, name).to_boolean()
    }

    fn has_keys(&self, item: ObjectId) -> bool {
        self.engine.read_by_name(item, "activeFocus").to_boolean()
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_menu_button_opens_the_windows_menu_whose_items_do_their_actions() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let button = hof.find("menuButton").unwrap();
    hof.click(button, ljos::item::MouseButtons::LEFT);
    assert!(hof.flag("windowMenuShown"), "the button opens the menu");

    let item = hof.find("menuItem:new-tab").unwrap();
    assert_eq!(
        hof.engine
            .read_by_name(item, "shortcut")
            .to_js_string()
            .as_str(),
        "Ctrl+Shift+T",
        "the item shows its chord"
    );
    hof.click(item, ljos::item::MouseButtons::LEFT);
    assert!(!hof.flag("windowMenuShown"), "choosing closes it");
    assert_eq!(hof.number(hof.layout(), "tabCount"), 2.0);
    assert_ne!(hof.current(), Some(t));
    assert!(
        hof.has_keys(hof.current().unwrap()),
        "the new tab's pane has the keys"
    );
}

#[test]
fn a_right_click_in_a_pane_opens_its_menu() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    let area = hof.find("paneMenuArea").unwrap();
    hof.click(area, ljos::item::MouseButtons::RIGHT);
    let menu = hof.find("paneMenu").unwrap();
    assert!(hof.engine.read_by_name(menu, "opened").to_boolean());
    let copy = hof
        .names("menuItem:")
        .iter()
        .filter(|n| *n == "menuItem:copy")
        .count();
    assert!(copy >= 1);

    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    hof.window.advance(200).unwrap();
    assert!(!hof.engine.read_by_name(menu, "opened").to_boolean());
    assert!(hof.has_keys(t), "the keys go back to the terminal");
}

#[test]
fn a_right_click_goes_to_a_program_that_asked_for_the_mouse() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    // Any-event mouse tracking, as a full-screen program asks for it.
    hof.engine
        .invoke_by_name(t, "write", &[Value::from("\x1b[?1003h\x1b[?1006h")])
        .unwrap();
    hof.window.settle().unwrap();
    assert!(hof.engine.read_by_name(t, "mouseTracking").to_boolean());
    let area = hof.find("paneMenuArea").unwrap();
    hof.click(area, ljos::item::MouseButtons::RIGHT);
    let menu = hof.find("paneMenu").unwrap();
    assert!(
        !hof.engine.read_by_name(menu, "opened").to_boolean(),
        "the program has the click"
    );
}

#[test]
fn the_settings_open_in_the_window_and_escape_puts_them_away() {
    let mut hof = opened();
    let t = hof.current().unwrap();
    hof.run("settings");
    assert!(hof.flag("settingsShown"));
    let theme = hof.find("settingTheme").expect("the controls are made");
    assert!(
        hof.engine.read_by_name(theme, "count").to_number() >= 2.0,
        "every theme is offered"
    );
    assert_eq!(
        hof.engine
            .read_by_name(theme, "currentText")
            .to_js_string()
            .as_str(),
        "hof-dark"
    );

    hof.window.key(&KeyEvent::press(KEY_ESCAPE));
    hof.window.settle().unwrap();
    assert!(!hof.flag("settingsShown"));
    assert!(hof.has_keys(t), "the keys go back to the terminal");
}

#[test]
fn the_panes_of_a_split_are_framed_and_the_focused_ones_frame_stands_out() {
    let mut hof = opened();
    let frames = |hof: &Hof| {
        let mut found = Vec::new();
        let mut items = vec![hof.root];
        while let Some(item) = items.pop() {
            let name = hof.engine.read_by_name(item, "objectName").to_js_string();
            let shown = hof.engine.read_by_name(item, "visible").to_boolean();
            if name.as_str() == "paneFrame" && shown {
                found.push(item);
            }
            items.extend(hof.engine.children(item));
        }
        found
    };
    let colour = |hof: &Hof, frame: ObjectId| {
        hof.engine
            .read_by_name(frame, "edge")
            .to_js_string()
            .to_string()
    };
    assert!(frames(&hof).is_empty(), "one pane has no frame");
    let first = hof.current().unwrap();
    let whole = hof.number(first, "width");
    hof.run("split-right");
    let second = hof.current().unwrap();
    let framed = frames(&hof);
    assert_eq!(framed.len(), 2, "both panes are framed");
    let colours: Vec<_> = framed.iter().map(|&f| colour(&hof, f)).collect();
    assert_ne!(
        colours[0], colours[1],
        "the focused pane's frame stands out"
    );
    // The frame is round the cells, not over them.
    let (w, _) = hof.size();
    let (a, b) = (hof.number(first, "width"), hof.number(second, "width"));
    assert!(
        a + b < w - 8.0,
        "{a} and {b} leave room for the frames in {w}"
    );
    assert!(a < whole / 2.0);
}
