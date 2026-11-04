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
    assert!(!hof.shows_tabs(), "one tab needs no tab bar");
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
    assert!((a - w / 2.0).abs() <= 1.0, "first is {a} of {w}");
    assert!((a + b - w).abs() <= 0.5, "together they fill {w}");
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
    assert_eq!(hof.number(first, "height"), hof.size().1);
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
    let chord = KeyEvent::press(KEY_T)
        .with_modifiers(Modifiers::CONTROL | Modifiers::SHIFT)
        .with_text("T");
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
    hof.window.key(&KeyEvent::press(KEY_RETURN));
    hof.until("the paste", |h| h.screen(t).contains("got:ljós"));
}
