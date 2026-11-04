//! The `Terminal` item in a test window, fed output by `write()` as a
//! session run elsewhere would feed it, so no program has to start.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use hof_qml::{Terminal, register};
use ljos::item::{
    DisplayList, DrawOp, Font, KeyEvent, MouseButtons, Paint, PointerEvent, Rgba, WheelEvent, key,
    kurbo,
};
use ljos::prelude::*;
use ljos::testing::TestWindow;

const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 200.0;

struct Fixture {
    engine: Engine,
    window: TestWindow,
    t: ObjectId,
}

fn shown() -> Fixture {
    let engine = ljos::engine();
    register(&engine);
    let mut window = TestWindow::new(&engine, WIDTH, HEIGHT);
    let t = engine.create_by_name("Hof", "Terminal", None).unwrap();
    engine.set(t, Terminal::EXTERNAL, true).unwrap();
    let font = Font {
        pixel_size: 10,
        ..Font::default()
    };
    engine.set(t, Terminal::FONT, font).unwrap();
    engine.set(t, Item::WIDTH, WIDTH).unwrap();
    engine.set(t, Item::HEIGHT, HEIGHT).unwrap();
    window.show(t).unwrap();
    window.settle().unwrap();
    Fixture { engine, window, t }
}

impl Fixture {
    fn write(&mut self, text: &str) {
        self.engine
            .invoke_by_name(self.t, "write", &[Value::from(text)])
            .unwrap();
        self.window.settle().unwrap();
    }

    fn painted(&self) -> Arc<DisplayList> {
        self.window.painted(self.t).expect("the terminal paints")
    }

    fn cell(&self) -> (f64, f64) {
        (
            self.engine.get(self.t, Terminal::CELL_WIDTH).unwrap(),
            self.engine.get(self.t, Terminal::CELL_HEIGHT).unwrap(),
        )
    }

    /// Where the middle of cell (`col`, `row`) is.
    fn at(&self, col: u16, row: u16) -> kurbo::Point {
        let (w, h) = self.cell();
        kurbo::Point::new((f64::from(col) + 0.5) * w, (f64::from(row) + 0.5) * h)
    }

    /// Strings `signal` carries, as they come.
    fn record(&self, signal: &str) -> Rc<RefCell<Vec<Vec<Value>>>> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&seen);
        self.engine
            .connect_by_name(self.t, signal, move |_, args| {
                into.borrow_mut().push(args.to_vec());
            })
            .unwrap();
        seen
    }
}

fn glyph_runs(list: &DisplayList) -> usize {
    list.ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::Glyphs { .. }))
        .count()
}

/// The rects filled in `colour`.
fn rects(list: &DisplayList, colour: Rgba) -> Vec<kurbo::Rect> {
    list.ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::Rect {
                rect,
                fill: Paint::Solid(c),
                ..
            } if *c == colour => Some(*rect),
            _ => None,
        })
        .collect()
}

#[test]
fn the_cells_that_fit_are_counted() {
    let f = shown();
    let cols = f.engine.get(f.t, Terminal::COLUMNS).unwrap();
    let rows = f.engine.get(f.t, Terminal::ROWS).unwrap();
    let (w, h) = f.cell();
    assert!(w > 0.0 && h > 0.0, "{w} x {h} cells");
    assert_eq!(cols, (WIDTH / w).floor() as i32);
    assert_eq!(rows, (HEIGHT / h).floor() as i32);
}

#[test]
fn padding_takes_cells_away() {
    let f = shown();
    let cols = f.engine.get(f.t, Terminal::COLUMNS).unwrap();
    let (w, _) = f.cell();
    f.engine.set(f.t, Terminal::PADDING, w * 2.0).unwrap();
    let fewer = f.engine.get(f.t, Terminal::COLUMNS).unwrap();
    assert_eq!(fewer, cols - 4);
}

#[test]
fn output_is_painted() {
    let mut f = shown();
    let before = glyph_runs(&f.painted());
    f.write("hello");
    assert!(glyph_runs(&f.painted()) > before);
    let text = f.engine.invoke_by_name(f.t, "text", &[]).unwrap();
    assert!(text.to_js_string().as_str().contains("hello"), "{text:?}");
}

#[test]
fn a_title_from_the_program_is_the_title() {
    let mut f = shown();
    f.write("\x1b]2;make test\x07");
    assert_eq!(f.engine.get(f.t, Terminal::TITLE).unwrap(), "make test");
}

#[test]
fn the_bell_rings() {
    let mut f = shown();
    let rang = f.record("bell");
    f.write("\x07");
    assert_eq!(rang.borrow().len(), 1);
}

#[test]
fn typing_leaves_as_input() {
    let mut f = shown();
    let input = f.record("input");
    // A click gives the terminal focus, as it does in a window.
    let p = f.at(0, 0);
    let now = f.window.now();
    f.window.pointer(&PointerEvent::press(p.x, p.y, now));
    f.window.pointer(&PointerEvent::release(p.x, p.y, now));
    assert!(f.engine.get(f.t, Item::ACTIVE_FOCUS).unwrap());
    input.borrow_mut().clear();
    assert!(f.window.key(&KeyEvent::press(key::A).with_text("a")));
    assert!(f.window.key(&KeyEvent::press(key::RETURN).with_text("\r")));
    let typed: String = input
        .borrow()
        .iter()
        .map(|a| a[0].to_js_string().as_str().to_owned())
        .collect();
    assert_eq!(typed, "a\r");
}

#[test]
fn what_the_input_method_commits_is_typed() {
    let mut f = shown();
    let input = f.record("input");
    let p = f.at(0, 0);
    let now = f.window.now();
    f.window.pointer(&PointerEvent::press(p.x, p.y, now));
    f.window.pointer(&PointerEvent::release(p.x, p.y, now));
    assert_eq!(f.window.input_method_item(), Some(f.t));
    input.borrow_mut().clear();
    assert!(f.window.commit("é"));
    let typed: String = input
        .borrow()
        .iter()
        .map(|a| a[0].to_js_string().as_str().to_owned())
        .collect();
    assert_eq!(typed, "é");
}

#[test]
fn a_new_size_is_new_cells() {
    let mut f = shown();
    let resized = f.record("resized");
    let cols = f.engine.get(f.t, Terminal::COLUMNS).unwrap();
    f.engine.set(f.t, Item::WIDTH, WIDTH / 2.0).unwrap();
    f.window.settle().unwrap();
    let fewer = f.engine.get(f.t, Terminal::COLUMNS).unwrap();
    assert!(fewer < cols, "{fewer} columns after halving, {cols} before");
    let rows = f.engine.get(f.t, Terminal::ROWS).unwrap();
    let last = resized.borrow().last().cloned().expect("resized");
    assert_eq!(last, vec![Value::from(fewer), Value::from(rows)]);
}

#[test]
fn dragging_selects() {
    let mut f = shown();
    let marker = Color::from_rgba8(10, 200, 30, 255);
    f.engine
        .set(f.t, Terminal::SELECTION_COLOR, marker)
        .unwrap();
    f.write("one two three");
    let marked = Rgba::from_straight_u8(10, 200, 30, 255);
    assert!(rects(&f.painted(), marked).is_empty());
    let (from, to) = (f.at(0, 0), f.at(6, 0));
    let now = f.window.now();
    f.window.pointer(&PointerEvent::press(from.x, from.y, now));
    f.window
        .pointer(&PointerEvent::moved(to.x, to.y, MouseButtons::LEFT, now));
    f.window.pointer(&PointerEvent::release(to.x, to.y, now));
    assert!(f.engine.get(f.t, Terminal::HAS_SELECTION).unwrap());
    let selected = f.engine.invoke_by_name(f.t, "selectedText", &[]).unwrap();
    assert_eq!(selected.to_js_string().as_str(), "one two");
    // One rect, seven cells wide, on the first row.
    let (w, h) = f.cell();
    let painted = rects(&f.painted(), marked);
    assert_eq!(painted, vec![kurbo::Rect::new(0.0, 0.0, 7.0 * w, h)]);
}

#[test]
fn the_wheel_scrolls_back() {
    let mut f = shown();
    let rows = f.engine.get(f.t, Terminal::ROWS).unwrap();
    let lines: String = (0..rows * 3).map(|i| format!("line {i}\r\n")).collect();
    f.write(&lines);
    assert!(f.engine.get(f.t, Terminal::AT_BOTTOM).unwrap());
    let bottom = f.engine.get(f.t, Terminal::SCROLL_OFFSET).unwrap();
    let p = f.at(1, 1);
    let now = f.window.now();
    let up = WheelEvent::notches(p.x, p.y, 0.0, 1.0, now);
    assert!(f.window.wheel(&up));
    let offset = f.engine.get(f.t, Terminal::SCROLL_OFFSET).unwrap();
    assert_eq!(offset, bottom - 3);
    assert!(!f.engine.get(f.t, Terminal::AT_BOTTOM).unwrap());
}

#[test]
fn a_program_runs_and_exits() {
    let engine = ljos::engine();
    register(&engine);
    let mut window = TestWindow::new(&engine, WIDTH, HEIGHT);
    let t = engine.create_by_name("Hof", "Terminal", None).unwrap();
    engine
        .set(t, Terminal::PROGRAM, "/bin/sh".to_owned())
        .unwrap();
    let args = vec!["-c".to_owned(), "printf 'from sh'; exit 3".to_owned()];
    engine.set(t, Terminal::ARGUMENTS, args).unwrap();
    let exited = Rc::new(RefCell::new(None));
    let into = Rc::clone(&exited);
    engine
        .connect_by_name(t, "exited", move |_, a| {
            *into.borrow_mut() = Some(a[0].to_number());
        })
        .unwrap();
    engine.set(t, Item::WIDTH, WIDTH).unwrap();
    engine.set(t, Item::HEIGHT, HEIGHT).unwrap();
    window.show(t).unwrap();
    // It starts once sized; it may have finished already.
    assert_eq!(engine.get(t, Terminal::ERROR).unwrap(), "");
    // The reader thread posts to the inbox; frames deliver it.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while exited.borrow().is_none() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
        window.settle().unwrap();
    }
    assert_eq!(*exited.borrow(), Some(3.0));
    assert!(!engine.get(t, Terminal::RUNNING).unwrap());
    assert_eq!(engine.get(t, Terminal::EXIT_CODE).unwrap(), 3);
    let text = engine.invoke_by_name(t, "text", &[]).unwrap();
    assert!(text.to_js_string().as_str().contains("from sh"), "{text:?}");
}

impl Fixture {
    fn invoke(&mut self, name: &str, args: &[Value]) -> Value {
        let v = self.engine.invoke_by_name(self.t, name, args).unwrap();
        self.window.settle().unwrap();
        v
    }

    fn found(&self) -> (i32, i32) {
        (
            self.engine.get(self.t, Terminal::SEARCH_CURRENT).unwrap(),
            self.engine.get(self.t, Terminal::SEARCH_TOTAL).unwrap(),
        )
    }
}

#[test]
fn finding_says_how_many_and_which() {
    let mut f = shown();
    f.write("cat\r\ndog\r\ncat\r\n");
    let total = f.invoke("find", &[Value::from("cat")]);
    assert_eq!(total.to_number(), 2.0);
    assert!(f.engine.get(f.t, Terminal::SEARCHING).unwrap());
    assert_eq!(f.found(), (1, 2), "1 of 2: the newest");
    f.invoke("findNext", &[]);
    assert_eq!(f.found(), (2, 2));
    f.invoke("findPrevious", &[]);
    assert_eq!(f.found(), (1, 2));
}

#[test]
fn matches_are_painted_and_the_current_one_apart() {
    let mut f = shown();
    let every = Color::from_rgba8(200, 10, 10, 255);
    let current = Color::from_rgba8(10, 10, 200, 255);
    f.engine.set(f.t, Terminal::MATCH_COLOR, every).unwrap();
    f.engine
        .set(f.t, Terminal::CURRENT_MATCH_COLOR, current)
        .unwrap();
    f.write("ox ox");
    f.invoke("find", &[Value::from("ox")]);
    let (w, h) = f.cell();
    let list = f.painted();
    let red = rects(&list, Rgba::from_straight_u8(200, 10, 10, 255));
    let blue = rects(&list, Rgba::from_straight_u8(10, 10, 200, 255));
    assert_eq!(red, vec![kurbo::Rect::new(0.0, 0.0, 2.0 * w, h)]);
    assert_eq!(blue, vec![kurbo::Rect::new(3.0 * w, 0.0, 5.0 * w, h)]);
}

#[test]
fn finding_in_history_scrolls_back_to_it() {
    let mut f = shown();
    let rows = f.engine.get(f.t, Terminal::ROWS).unwrap();
    let lines: String = (0..rows * 3).map(|i| format!("line {i}\r\n")).collect();
    f.write(&format!("the needle\r\n{lines}"));
    f.invoke("find", &[Value::from("needle")]);
    assert_eq!(f.found(), (1, 1));
    assert!(!f.engine.get(f.t, Terminal::AT_BOTTOM).unwrap());
}

#[test]
fn ending_the_search_clears_what_it_painted() {
    let mut f = shown();
    let every = Color::from_rgba8(200, 10, 10, 255);
    f.engine.set(f.t, Terminal::MATCH_COLOR, every).unwrap();
    f.engine
        .set(f.t, Terminal::CURRENT_MATCH_COLOR, every)
        .unwrap();
    f.write("gone");
    f.invoke("find", &[Value::from("gone")]);
    let red = Rgba::from_straight_u8(200, 10, 10, 255);
    assert_eq!(rects(&f.painted(), red).len(), 1);
    f.invoke("endFind", &[]);
    assert!(rects(&f.painted(), red).is_empty());
    assert!(!f.engine.get(f.t, Terminal::SEARCHING).unwrap());
    assert_eq!(f.found(), (0, 0));
}

#[test]
fn output_after_finding_is_counted() {
    let mut f = shown();
    f.write("one hit\r\n");
    f.invoke("find", &[Value::from("hit")]);
    f.write("two hit\r\n");
    assert_eq!(f.found().1, 2);
}

#[test]
fn a_paste_that_could_run_commands_is_held_back_unless_told_not_to() {
    let mut f = shown();
    let blocked = f.record("pasteBlocked");
    let held = f.invoke("pasteText", &[Value::from("ls\nrm x\n")]);
    assert!(!held.to_boolean());
    assert_eq!(blocked.borrow().len(), 1, "the user is asked");

    f.engine.set(f.t, Terminal::CONFIRM_PASTE, false).unwrap();
    let pasted = f.invoke("pasteText", &[Value::from("ls\nrm x\n")]);
    assert!(pasted.to_boolean());
    assert_eq!(blocked.borrow().len(), 1, "nobody is asked again");
}

impl Fixture {
    fn click(&mut self) {
        let p = self.at(0, 0);
        let now = self.window.now();
        self.window.pointer(&PointerEvent::press(p.x, p.y, now));
        self.window.pointer(&PointerEvent::release(p.x, p.y, now));
    }

    /// The rects the cursor is painted as.
    fn cursor(&self) -> Vec<kurbo::Rect> {
        rects(&self.painted(), Rgba::from_straight_u8(12, 200, 34, 255))
    }
}

fn with_cursor(shape: &str, blink: bool) -> Fixture {
    let f = shown();
    f.engine
        .set(
            f.t,
            Terminal::CURSOR_COLOR,
            Color::from_rgba8(12, 200, 34, 255),
        )
        .unwrap();
    f.engine
        .set(f.t, Terminal::CURSOR_SHAPE, shape.to_owned())
        .unwrap();
    f.engine.set(f.t, Terminal::CURSOR_BLINK, blink).unwrap();
    f
}

#[test]
fn a_focused_blinking_cursor_comes_and_goes_and_typing_brings_it_back() {
    let mut f = with_cursor("block", true);
    f.click();
    f.window.settle().unwrap();
    assert_eq!(f.cursor().len(), 1, "shown at first");
    f.window.advance(650).unwrap();
    assert!(f.cursor().is_empty(), "hidden after a while");
    f.window.advance(600).unwrap();
    assert_eq!(f.cursor().len(), 1, "and back");
    f.window.advance(600).unwrap();
    assert!(f.cursor().is_empty());
    // Output moves the cursor, which shows it at once.
    f.write("x");
    assert_eq!(f.cursor().len(), 1, "shown while typed at");
}

#[test]
fn an_unfocused_or_steady_cursor_stays() {
    let mut f = with_cursor("block", true);
    f.window.settle().unwrap();
    for _ in 0..3 {
        f.window.advance(600).unwrap();
        // Unfocused, a hollow outline: four edges.
        assert_eq!(f.cursor().len(), 4);
    }
    let mut f = with_cursor("block", false);
    f.click();
    for _ in 0..3 {
        f.window.advance(600).unwrap();
        assert_eq!(f.cursor().len(), 1);
    }
}

#[test]
fn the_cursor_has_the_configured_shape_until_a_program_asks_for_another() {
    let mut f = with_cursor("underline", false);
    f.click();
    f.window.settle().unwrap();
    let (w, h) = f.cell();
    let underline = f.cursor();
    assert_eq!(underline.len(), 1);
    assert!(underline[0].height() < h / 2.0 && underline[0].width() == w);

    f.write("\x1b[6 q");
    let bar = f.cursor();
    assert!(bar[0].width() < w / 2.0 && bar[0].height() == h, "{bar:?}");

    // A new config replaces it.
    f.engine
        .set(f.t, Terminal::CURSOR_SHAPE, "block".to_owned())
        .unwrap();
    f.window.settle().unwrap();
    assert_eq!(f.cursor(), vec![kurbo::Rect::new(0.0, 0.0, w, h)]);
}

fn clipboard(f: &Fixture) -> Option<String> {
    Platform::of(&f.engine)
        .clipboard()
        .text(ClipboardMode::Clipboard)
}

#[test]
fn a_program_puts_text_on_the_clipboard_unless_it_may_not() {
    let mut f = shown();
    // "ljós", in base64.
    f.write("\x1b]52;c;bGrDs3M=\x07");
    assert_eq!(clipboard(&f).as_deref(), Some("ljós"));

    f.engine.set(f.t, Terminal::CLIPBOARD_WRITE, false).unwrap();
    f.write("\x1b]52;c;bm8=\x07");
    assert_eq!(clipboard(&f).as_deref(), Some("ljós"), "\"no\" is kept off");
}

/// What the terminal sent the program, as `input`.
fn sent(input: &RefCell<Vec<Vec<Value>>>) -> String {
    input
        .borrow()
        .iter()
        .map(|a| a[0].to_js_string().as_str().to_owned())
        .collect()
}

#[test]
fn a_program_reads_the_clipboard_once_the_user_lets_it() {
    let mut f = shown();
    let asked = f.record("clipboardRequested");
    let input = f.record("input");
    Platform::of(&f.engine)
        .clipboard()
        .set_text(ClipboardMode::Clipboard, "ljós")
        .unwrap();

    f.write("\x1b]52;c;?\x07\x1b]52;c;?\x07");
    assert_eq!(asked.borrow().len(), 1, "asked once for both");
    assert_eq!(asked.borrow()[0][0].to_js_string().as_str(), "clipboard");
    assert_eq!(sent(&input), "", "nothing is said before the user says");

    f.invoke("answerClipboard", &[Value::from(true), Value::from(false)]);
    assert_eq!(sent(&input), "\x1b]52;c;bGrDs3M=\x1b\\".repeat(2));

    input.borrow_mut().clear();
    f.write("\x1b]52;c;?\x07");
    assert_eq!(asked.borrow().len(), 2, "allowing once is not always");
    f.invoke("answerClipboard", &[Value::from(false), Value::from(false)]);
    assert_eq!(
        sent(&input),
        "\x1b]52;c;\x1b\\",
        "a denied read is answered empty"
    );
}

#[test]
fn a_onetime_consent_answers_only_the_clipboard_it_names() {
    let mut f = shown();
    let asked = f.record("clipboardRequested");
    let input = f.record("input");
    let clipboard = Platform::of(&f.engine).clipboard();
    clipboard
        .set_text(ClipboardMode::Clipboard, "standard")
        .unwrap();
    clipboard
        .set_text(ClipboardMode::Selection, "selection")
        .unwrap();

    f.write("\x1b]52;c;?\x07\x1b]52;p;?\x07");
    f.invoke("answerClipboard", &[Value::from(true), Value::from(false)]);
    let standard =
        String::from_utf8(hof_vt::carrying(hof_vt::Clipboard::Standard, "standard")).unwrap();
    assert_eq!(sent(&input), standard);
    assert_eq!(asked.borrow().len(), 2, "the selection is a new question");
    assert_eq!(asked.borrow()[1][0].to_js_string().as_str(), "selection");

    input.borrow_mut().clear();
    f.invoke("answerClipboard", &[Value::from(true), Value::from(false)]);
    let selection =
        String::from_utf8(hof_vt::carrying(hof_vt::Clipboard::Selection, "selection")).unwrap();
    assert_eq!(sent(&input), selection);
}

#[test]
fn clipboard_reads_waiting_for_consent_are_bounded() {
    let mut f = shown();
    let input = f.record("input");
    f.write(&"\x1b]52;c;?\x07".repeat(20));
    f.invoke("answerClipboard", &[Value::from(true), Value::from(false)]);
    let answer = String::from_utf8(hof_vt::carrying(hof_vt::Clipboard::Standard, "")).unwrap();
    assert_eq!(sent(&input), answer.repeat(8));
}

#[test]
fn a_read_let_always_or_by_policy_is_answered_unasked() {
    let mut f = shown();
    let asked = f.record("clipboardRequested");
    let input = f.record("input");
    Platform::of(&f.engine)
        .clipboard()
        .set_text(ClipboardMode::Clipboard, "hi")
        .unwrap();

    f.write("\x1b]52;c;?\x07");
    f.invoke("answerClipboard", &[Value::from(true), Value::from(true)]);
    input.borrow_mut().clear();
    f.write("\x1b]52;c;?\x07");
    assert_eq!(asked.borrow().len(), 1);
    assert_eq!(sent(&input), "\x1b]52;c;aGk=\x1b\\");

    let mut g = shown();
    let asked = g.record("clipboardRequested");
    let input = g.record("input");
    g.engine
        .set(g.t, Terminal::CLIPBOARD_READ, "deny".to_owned())
        .unwrap();
    g.write("\x1b]52;c;?\x07");
    assert!(asked.borrow().is_empty());
    assert_eq!(sent(&input), "\x1b]52;c;\x1b\\");
}

fn recording_opens(f: &Fixture) -> RecordedUrls {
    let urls = RecordedUrls::default();
    Platform::of(&f.engine).set_open_url(Rc::new(urls.clone()));
    urls
}

fn open(url: &str) -> String {
    String::from_utf8(hof_vt::open_request(url)).unwrap()
}

#[test]
fn a_url_the_program_asks_to_open_opens_once_the_user_lets_it() {
    let mut f = shown();
    let urls = recording_opens(&f);
    let asked = f.record("openRequested");
    f.write(&format!(
        "{}{}",
        open("https://a.example"),
        open("https://b.example")
    ));
    assert_eq!(asked.borrow().len(), 1, "one at a time");
    assert_eq!(
        asked.borrow()[0][0].to_js_string().as_str(),
        "https://a.example"
    );
    assert!(urls.urls().is_empty(), "nothing opens before the user says");

    f.invoke("answerOpen", &[Value::from(false), Value::from(false)]);
    assert!(urls.urls().is_empty(), "turned down");
    assert_eq!(asked.borrow().len(), 2, "then the next is asked");
    assert_eq!(
        asked.borrow()[1][0].to_js_string().as_str(),
        "https://b.example"
    );

    f.invoke("answerOpen", &[Value::from(true), Value::from(true)]);
    f.write(&open("https://c.example"));
    assert_eq!(urls.urls(), ["https://b.example", "https://c.example"]);
    assert_eq!(asked.borrow().len(), 2, "always is not asked again");
}

#[test]
fn a_program_may_open_urls_or_not_as_the_terminal_says() {
    let mut f = shown();
    let urls = recording_opens(&f);
    let asked = f.record("openRequested");
    f.engine
        .set(f.t, Terminal::OPEN_URLS, "deny".to_owned())
        .unwrap();
    f.write(&open("https://a.example"));
    f.engine
        .set(f.t, Terminal::OPEN_URLS, "allow".to_owned())
        .unwrap();
    f.write(&open("file:///tmp/x"));
    assert!(asked.borrow().is_empty());
    assert_eq!(urls.urls(), ["file:///tmp/x"]);
}
