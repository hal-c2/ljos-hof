//! The `Terminal` item: its properties, its session, and how news from the
//! session reaches them.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hof_remote::client::Start;

use hof_vt::{
    Changes, CursorShape, Event, Found, Link, Options, PasteError, Progress, PromptMark, Rgb,
    Scroll, Session, Size, Spawn, Theme,
};
use ljos::item::{self, Font, KeyEvent, Surrounding, TextInputClient, kurbo};
use ljos::prelude::*;

use crate::agents::{self, Said, State};
use crate::fonts;
use crate::keys;
use crate::paint::{Metrics, View};
use crate::pointer::PointerState;
use crate::remote::{self, Begin, Out, Remote};

/// A terminal: a program on a pseudo-terminal, drawn as a grid of cells as
/// large as its font allows, as many as fit inside its padding.
///
/// It starts the program (`program` and `arguments`, or the user's shell)
/// once it is laid out and knows how many cells it has. With `external`
/// it starts nothing: output comes in through `write()`, and what the user
/// types leaves through `input`.
#[derive(QmlObject)]
#[qml(module = "Hof", name = "Terminal", extends = Item)]
pub struct Terminal {
    #[qml(base)]
    pub item: Item,
    #[qml(notify)]
    pub font: Font,
    /// Space between the edges and the cells, in logical pixels.
    #[qml(notify)]
    pub padding: f64,
    /// The colours programs start with and get back on reset. An invalid
    /// colour keeps libghostty's own.
    #[qml(notify)]
    pub background_color: Color,
    #[qml(notify)]
    pub foreground_color: Color,
    #[qml(notify)]
    pub cursor_color: Color,
    #[qml(notify)]
    pub selection_color: Color,
    /// What `find` highlights: every match, and the current one. Invalid
    /// is the palette's yellow and bright yellow.
    #[qml(notify)]
    pub match_color: Color,
    #[qml(notify)]
    pub current_match_color: Color,
    /// The first entries of the 256-colour palette, in order; the usual 16.
    #[qml(notify)]
    pub palette: Vec<Color>,
    #[qml(notify)]
    pub bold_is_bright: bool,
    /// Treat Command as Command and Control as Control, rather than as Qt
    /// swaps them. On by default on macOS, where the swap happens.
    #[qml(notify)]
    pub mac_modifiers: bool,
    /// The cursor programs start with and get back on reset: "block",
    /// "bar", "underline" or "hollow". Programs may ask for another.
    #[qml(notify)]
    pub cursor_shape: String,
    /// Whether that cursor blinks, while the terminal has the keys.
    #[qml(notify)]
    pub cursor_blink: bool,
    /// The terminal's own shortcuts: copy and paste with Ctrl+Shift+C/V,
    /// scrolling with Shift+PageUp/PageDown/Home/End.
    #[qml(notify)]
    pub shortcuts: bool,
    /// Lines of history kept above the screen.
    #[qml(notify)]
    pub scrollback: i32,
    /// Put what the mouse selects on the clipboard, not only in the
    /// primary selection.
    #[qml(notify)]
    pub copy_on_select: bool,
    /// Hold back a paste that could run commands (a newline, control
    /// codes) and say so through `pasteBlocked`, for the user to confirm.
    #[qml(notify)]
    pub confirm_paste: bool,
    /// Whether a program may read the clipboard (OSC 52): "ask", through
    /// `clipboardRequested`; "allow"; or "deny", which answers it empty.
    #[qml(notify)]
    pub clipboard_read: String,
    /// Whether a program may put text on the clipboard (OSC 52).
    #[qml(notify)]
    pub clipboard_write: bool,
    /// Whether a program may have a URL opened here (`hof open`): "ask",
    /// through `openRequested`; "allow"; or "deny". A remote program's
    /// `file:` URL names a file there, which is not opened here.
    #[qml(notify)]
    pub open_urls: String,

    /// What to run; empty for the user's shell.
    #[qml(notify)]
    pub program: String,
    #[qml(notify)]
    pub arguments: Vec<String>,
    /// Where the program starts; empty for ours.
    #[qml(notify)]
    pub start_directory: String,
    /// Extra variables for the program, each `NAME=value`.
    #[qml(notify)]
    pub environment: Vec<String>,
    /// Start the program as soon as the size is known.
    #[qml(notify)]
    pub auto_start: bool,
    /// Start nothing: a session fed by `write()`, for a program run elsewhere.
    #[qml(notify)]
    pub external: bool,
    /// Answer what the program asks the terminal (its attributes, colours,
    /// size). Off for a remote pane's copy here: `hof agent`, beside the
    /// program, answers, and two answers would be one too many. Read when
    /// the session opens.
    #[qml(notify)]
    pub answers: bool,
    /// Output shown before the program's own, once: a `snapshot()` from a
    /// saved session, so the scrollback comes back with the pane.
    #[qml(notify)]
    pub replay: String,
    /// The `[hosts]` name of the machine the program runs on, under
    /// `hof agent` there; empty for this one. Read when the session opens.
    #[qml(notify)]
    pub host: String,
    /// The program's pane under that agent: set before the session opens
    /// to pick up one that runs already, as a restored pane does; -1 to
    /// start the program anew. Once it runs it says which it is, and -1
    /// again once it has ended.
    #[qml(notify)]
    pub remote_pane: i32,

    #[qml(notify, readonly)]
    pub columns: i32,
    #[qml(notify, readonly)]
    pub rows: i32,
    /// One cell, in logical pixels.
    #[qml(notify, readonly)]
    pub cell_width: f64,
    #[qml(notify, readonly)]
    pub cell_height: f64,
    /// The scrollback, in lines: all of it, where the view starts, and how
    /// many the view shows.
    #[qml(notify, readonly)]
    pub scroll_total: i32,
    #[qml(notify, readonly)]
    pub scroll_offset: i32,
    #[qml(notify, readonly)]
    pub scroll_visible: i32,
    #[qml(notify, readonly)]
    pub at_bottom: bool,
    #[qml(notify, readonly)]
    pub title: String,
    /// Where the shell says it is (OSC 7), as a path.
    #[qml(notify, readonly)]
    pub working_directory: String,
    #[qml(notify, readonly)]
    pub has_selection: bool,
    /// The program reads the mouse; Shift still selects.
    #[qml(notify, readonly)]
    pub mouse_tracking: bool,
    #[qml(notify, readonly)]
    pub running: bool,
    /// The program's exit status once it has exited; -1 for a signal.
    #[qml(notify, readonly)]
    pub exit_code: i32,
    /// Why the program could not start, or the last thing that failed.
    #[qml(notify, readonly)]
    pub error: String,
    /// How the connection to `host` is: "connecting", "up", "down" (being
    /// made again; the program runs on) or "gone" (it won't come back);
    /// empty for a program on this machine.
    #[qml(notify, readonly)]
    pub connection: String,
    /// Why it is down or gone.
    #[qml(notify, readonly)]
    pub connection_message: String,
    /// What the program says of its progress (OSC 9;4): "", "normal",
    /// "error", "indeterminate" or "paused", and how far, 0 to 100.
    #[qml(notify, readonly)]
    pub progress_state: String,
    #[qml(notify, readonly)]
    pub progress_value: i32,
    /// The shell marks its prompts (OSC 133), so Hof knows where commands
    /// start and end.
    #[qml(notify, readonly)]
    pub shell_integration: bool,
    /// A command is running: its output started and it has not ended, as
    /// shell integration tells.
    #[qml(notify, readonly)]
    pub command_running: bool,
    /// A search is on: `find` was called and `endFind` not yet.
    #[qml(notify, readonly)]
    pub searching: bool,
    /// How many matches the search has, and which is current, counted
    /// from the newest at 1; 0 when none is.
    #[qml(notify, readonly)]
    pub search_total: i32,
    #[qml(notify, readonly)]
    pub search_current: i32,
    /// The link under the pointer: the program's own (OSC 8), or a URL in
    /// the text. Ctrl+click opens it.
    #[qml(notify, readonly)]
    pub hovered_link: String,
    /// The AI agent the terminal runs, by its `[agents]` table's name or
    /// the name it gave; empty when there is none.
    #[qml(notify, readonly)]
    pub agent: String,
    /// How it is: "working", "waiting" (for the user), "idle", or "none".
    #[qml(notify, readonly)]
    pub agent_state: String,

    #[qml(skip)]
    pub(crate) shared: Rc<Shared>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            item: Item::default(),
            font: Font {
                family: Atom::from("monospace"),
                pixel_size: 14,
                ..Font::default()
            },
            padding: 0.0,
            background_color: Color::INVALID,
            foreground_color: Color::INVALID,
            cursor_color: Color::INVALID,
            selection_color: Color::INVALID,
            match_color: Color::INVALID,
            current_match_color: Color::INVALID,
            palette: Vec::new(),
            bold_is_bright: false,
            // Qt swaps them there, and Ljós UI with it.
            mac_modifiers: cfg!(target_os = "macos"),
            cursor_shape: "block".into(),
            cursor_blink: false,
            shortcuts: true,
            scrollback: 10_000,
            copy_on_select: false,
            confirm_paste: true,
            clipboard_read: "ask".into(),
            clipboard_write: true,
            open_urls: "ask".into(),
            program: String::new(),
            arguments: Vec::new(),
            start_directory: String::new(),
            environment: Vec::new(),
            auto_start: true,
            external: false,
            answers: true,
            replay: String::new(),
            host: String::new(),
            remote_pane: -1,
            columns: 0,
            rows: 0,
            cell_width: 0.0,
            cell_height: 0.0,
            scroll_total: 0,
            scroll_offset: 0,
            scroll_visible: 0,
            at_bottom: true,
            title: String::new(),
            working_directory: String::new(),
            has_selection: false,
            mouse_tracking: false,
            running: false,
            exit_code: 0,
            error: String::new(),
            connection: String::new(),
            connection_message: String::new(),
            progress_state: String::new(),
            progress_value: 0,
            shell_integration: false,
            command_running: false,
            searching: false,
            search_total: 0,
            search_current: 0,
            hovered_link: String::new(),
            agent: String::new(),
            agent_state: "none".into(),
            shared: Rc::default(),
        }
    }
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal")
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

/// A terminal's state outside its properties. Dropping it drops the
/// session, which hangs up on the program.
#[derive(Default)]
pub(crate) struct Shared {
    pub session: RefCell<Option<Session>>,
    /// What paint reads; lent to the painter, which outlives a read.
    pub view: Rc<RefCell<View>>,
    pub pointer: RefCell<PointerState>,
    /// What the input method is composing, shown at the cursor.
    pub preedit: RefCell<String>,
    pub blink: RefCell<Blink>,
    /// The pane on another machine the session's program runs as, if it
    /// runs on one.
    pub remote: RefCell<Option<Arc<Remote>>>,
    /// The session began by attaching to a pane that ran already, and has
    /// not reached it yet: should that pane be gone, a new one starts.
    pub attaching: Cell<bool>,
    pub agent: RefCell<agents::Watch>,
    pub clipboard: RefCell<Asked<hof_vt::Clipboard>>,
    pub opens: RefCell<Asked<String>>,
}

/// What the program asked that waits for the user.
#[derive(Debug)]
pub(crate) struct Asked<T> {
    /// Each ask, in order; each gets its answer.
    pub waiting: Vec<T>,
    /// The user said to let this pane's program, from now on.
    pub always: bool,
}

impl<T> Default for Asked<T> {
    fn default() -> Self {
        Self {
            waiting: Vec::new(),
            always: false,
        }
    }
}

/// The most requests to open that may wait: a program asking for more is
/// up to no good, and the rest are dropped.
const MOST_OPENS: usize = 8;

/// How long a blinking cursor stays shown, and hidden.
const BLINK: Duration = Duration::from_millis(600);

/// A blinking cursor's phase. The timer runs only while a frame painted
/// a blinking cursor, so a terminal out of sight or out of focus wakes no
/// one.
#[derive(Debug, Default)]
pub(crate) struct Blink {
    /// In the hidden half.
    pub off: bool,
    due: Option<TimerId>,
}

/// Keeps the blinking going for a frame that paints a cursor that
/// `blinks`; one that `moved` is shown, its phase started again, so it is
/// never hidden while being typed at. Returns whether to hide the cursor.
pub(crate) fn blink(
    engine: &Engine,
    me: ObjectId,
    shared: &Shared,
    blinks: bool,
    moved: bool,
) -> bool {
    let mut b = shared.blink.borrow_mut();
    if !blinks || moved {
        if let Some(id) = b.due.take() {
            engine.cancel_call(id);
        }
        b.off = false;
    }
    if blinks && b.due.is_none() {
        b.due = Some(engine.call_at(engine.now() + BLINK, move |e| {
            let Some(shared) = shared_of(e, me) else {
                return;
            };
            let mut b = shared.blink.borrow_mut();
            b.due = None;
            b.off = !b.off;
            drop(b);
            item::repaint(e, me);
        }));
    }
    b.off
}

#[qml_impl]
impl Terminal {
    /// What the user typed, for the program, when the terminal is `external`.
    #[qml(signal)]
    fn input(data: String);

    #[qml(signal)]
    fn resized(columns: i32, rows: i32);

    #[qml(signal)]
    fn bell();

    /// A desktop notification the program asked for (OSC 9 or 777).
    #[qml(signal)]
    fn notification(title: String, body: String);

    /// The shell ran a command (shell integration, OSC 133).
    #[qml(signal)]
    fn command_started(command: String);

    /// The command finished; -1 when the shell didn't say how.
    #[qml(signal)]
    fn command_finished(exit_code: i32);

    #[qml(signal)]
    fn exited(exit_code: i32);

    /// A paste that could run commands unseen was held back; paste it with
    /// `forcePaste` once the user agrees.
    #[qml(signal)]
    fn paste_blocked(text: String);

    /// The agent, or how it is, changed: `agent` and `agentState` as they
    /// are now.
    #[qml(signal)]
    fn agent_status(agent: String, state: String);

    /// The program asks to read a clipboard ("clipboard" or "selection")
    /// and `clipboardRead` says to ask: the program waits for
    /// `answerClipboard`. Further reads wait with it, unsaid.
    #[qml(signal)]
    fn clipboard_requested(clipboard: String);

    /// The program asks to open `url` here and `openUrls` says to ask.
    /// `answerOpen` answers it; a request after it waits, and is asked
    /// for once this one is answered.
    #[qml(signal)]
    fn open_requested(url: String);

    /// Something to tell the user for a moment, of a file on the other
    /// machine being opened here: that it is being copied, or why it
    /// could not be. Empty once it is opened.
    #[qml(signal)]
    fn notice(text: String);

    #[qml(on_complete)]
    fn completed(cx: &Cx) -> Result<(), EngineError> {
        let (e, me) = (cx.engine, cx.this);
        if item::prop(e, me, Terminal::EXTERNAL) {
            open(e, me, None);
        }
        e.effect_in(me, move |e| measure(e, me))?;
        e.effect_in(me, move |e| {
            let theme = theme_of(e, me);
            let bright = item::prop(e, me, Terminal::BOLD_IS_BRIGHT);
            let lines = item::prop(e, me, Terminal::SCROLLBACK);
            let Some(shared) = shared_of(e, me) else {
                return;
            };
            shared.view.borrow_mut().bold_is_bright = bright;
            if let Some(s) = shared.session.borrow().as_ref() {
                let _ = s.set_theme(&theme);
                s.set_dark(is_dark(&theme));
                let _ = s.set_scrollback(lines.max(0) as usize);
            }
            item::repaint(e, me);
        })?;
        // Its own effect: a theme changing must not take away the cursor
        // a program asked for.
        e.effect_in(me, move |e| {
            let shape = shape_of(e, me);
            let blink = item::prop(e, me, Terminal::CURSOR_BLINK);
            with_session(e, me, |s| s.set_cursor(shape, blink));
        })?;
        e.connect_by_name(me, "activeFocusChanged", move |e, _| {
            let focused = item::prop(e, me, Item::ACTIVE_FOCUS);
            with_session(e, me, |s| s.focus(focused));
            flush_input(e, me);
        })?;
        Ok(())
    }

    /// Feeds program output in, as if the program had written it.
    #[qml(invokable)]
    fn write(cx: &Cx, data: String) {
        with_session(cx.engine, cx.this, |s| s.feed(data.as_bytes()));
        flush_input(cx.engine, cx.this);
    }

    /// Sends bytes to the program, as if typed.
    #[qml(invokable)]
    fn send(cx: &Cx, data: String) {
        with_session(cx.engine, cx.this, |s| s.write(data.as_bytes()));
        flush_input(cx.engine, cx.this);
    }

    /// Everything on the screen and in the scrollback, as plain text.
    #[qml(invokable)]
    fn text(cx: &Cx) -> String {
        with_session(cx.engine, cx.this, |s| s.text()).unwrap_or_default()
    }

    /// What is on the screen, as plain text, a line per row.
    #[qml(invokable)]
    fn screen_text(cx: &Cx) -> String {
        with_session(cx.engine, cx.this, |s| s.screen_text()).unwrap_or_default()
    }

    /// Marks the cursor's line as the one a command is about to be typed
    /// on, for `commandOutput`. Returns whether there is a session to mark.
    #[qml(invokable)]
    fn mark_command(cx: &Cx) -> bool {
        with_session(cx.engine, cx.this, |s| s.mark_command().is_ok()).unwrap_or(false)
    }

    /// What the command typed on the marked line has printed so far, its
    /// own line left out; empty with no mark.
    #[qml(invokable)]
    fn command_output(cx: &Cx) -> String {
        with_session(cx.engine, cx.this, |s| s.command_output())
            .flatten()
            .unwrap_or_default()
    }

    /// Forgets the command mark.
    #[qml(invokable)]
    fn end_command(cx: &Cx) {
        with_session(cx.engine, cx.this, |s| s.end_command());
    }

    #[qml(invokable)]
    fn selected_text(cx: &Cx) -> String {
        with_session(cx.engine, cx.this, |s| s.selected_text())
            .flatten()
            .unwrap_or_default()
    }

    /// Copies the selection to the clipboard. Returns whether there was one.
    #[qml(invokable)]
    fn copy(cx: &Cx) -> bool {
        copy_selection(cx.engine, cx.this, ClipboardMode::Clipboard)
    }

    /// Pastes the clipboard.
    #[qml(invokable)]
    fn paste(cx: &Cx) -> bool {
        paste_from(cx.engine, cx.this, ClipboardMode::Clipboard)
    }

    /// Pastes `text` the way the program asked for pastes. Text that could
    /// run commands unseen is held back and `pasteBlocked` says so.
    #[qml(invokable)]
    fn paste_text(cx: &Cx, text: String) -> bool {
        paste(cx.engine, cx.this, &text, false)
    }

    /// Answers the program's clipboard reads that wait: with what is on
    /// the clipboard if `allow`, else with nothing. A one-time answer covers
    /// only the clipboard named in the question. `always` covers both.
    #[qml(invokable)]
    fn answer_clipboard(cx: &Cx, allow: bool, always: bool) {
        let Some(shared) = shared_of(cx.engine, cx.this) else {
            return;
        };
        let (waiting, next) = {
            let mut reads = shared.clipboard.borrow_mut();
            reads.always |= allow && always;
            let Some(first) = reads.waiting.first().copied() else {
                return;
            };
            if reads.always {
                (std::mem::take(&mut reads.waiting), None)
            } else {
                let same = reads
                    .waiting
                    .iter()
                    .take_while(|&&from| from == first)
                    .count();
                let waiting = reads.waiting.drain(..same).collect();
                (waiting, reads.waiting.first().copied())
            }
        };
        for from in waiting {
            answer_read(cx.engine, cx.this, from, allow);
        }
        flush_input(cx.engine, cx.this);
        if let Some(from) = next {
            let _ = Terminal::emit_clipboard_requested(cx, clipboard_name(from).to_owned());
        }
    }

    /// Opens the URL `openRequested` asked for if `allow`, and lets the
    /// program open any from now on if `always` too.
    #[qml(invokable)]
    fn answer_open(cx: &Cx, allow: bool, always: bool) {
        let (e, me) = (cx.engine, cx.this);
        let Some(shared) = shared_of(e, me) else {
            return;
        };
        let (url, next) = {
            let mut opens = shared.opens.borrow_mut();
            if opens.waiting.is_empty() {
                return;
            }
            opens.always |= allow && always;
            let url = opens.waiting.remove(0);
            // Let always, those that wait go too.
            let rest = if opens.always {
                std::mem::take(&mut opens.waiting)
            } else {
                Vec::new()
            };
            (url, (rest, opens.waiting.first().cloned()))
        };
        if allow {
            open_wanted(e, me, &url);
        }
        let (rest, next) = next;
        for url in rest {
            open_wanted(e, me, &url);
        }
        if let Some(url) = next {
            let _ = Terminal::emit_open_requested(cx, url);
        }
    }

    /// Pastes `text` even if it could run commands.
    #[qml(invokable)]
    fn force_paste(cx: &Cx, text: String) -> bool {
        paste(cx.engine, cx.this, &text, true)
    }

    #[qml(invokable)]
    fn clear_selection(cx: &Cx) {
        with_session(cx.engine, cx.this, |s| s.clear_selection());
    }

    #[qml(invokable)]
    fn select_all(cx: &Cx) {
        with_session(cx.engine, cx.this, |s| s.select_all());
    }

    /// Moves the view by `lines`; up is negative.
    #[qml(invokable)]
    fn scroll_lines(cx: &Cx, lines: i32) {
        scroll(cx.engine, cx.this, Scroll::Lines(lines as isize));
    }

    /// Moves the view to start at line `offset` of the scrollback.
    #[qml(invokable)]
    fn scroll_to(cx: &Cx, offset: f64) {
        scroll(cx.engine, cx.this, Scroll::Row(offset.max(0.0) as usize));
    }

    #[qml(invokable)]
    fn scroll_to_top(cx: &Cx) {
        scroll(cx.engine, cx.this, Scroll::Top);
    }

    #[qml(invokable)]
    fn scroll_to_bottom(cx: &Cx) {
        scroll(cx.engine, cx.this, Scroll::Bottom);
    }

    /// Moves the view by shell prompts; back is negative.
    #[qml(invokable)]
    fn scroll_to_prompt(cx: &Cx, by: i32) {
        scroll(cx.engine, cx.this, Scroll::Prompts(by as isize));
    }

    /// Looks for `text` on the screen and in the scrollback, case aside for
    /// ASCII, and shows the newest match. Returns how many there are. Empty
    /// text ends the search.
    #[qml(invokable)]
    fn find(cx: &Cx, text: String) -> i32 {
        let found = with_session(cx.engine, cx.this, |s| s.find(&text).ok());
        let searching = !text.is_empty();
        show_found(cx.engine, cx.this, found.flatten().filter(|_| searching));
        update_scroll(cx.engine, cx.this);
        item::repaint(cx.engine, cx.this);
        clamp_count(found.flatten().unwrap_or_default().total)
    }

    /// Shows the next older match, wrapping around to the newest.
    #[qml(invokable)]
    fn find_next(cx: &Cx) {
        step_found(cx.engine, cx.this, Session::find_older);
    }

    /// Shows the next newer match, wrapping around to the oldest.
    #[qml(invokable)]
    fn find_previous(cx: &Cx) {
        step_found(cx.engine, cx.this, Session::find_newer);
    }

    #[qml(invokable)]
    fn end_find(cx: &Cx) {
        with_session(cx.engine, cx.this, |s| s.end_find());
        show_found(cx.engine, cx.this, None);
    }

    /// A full reset (RIS): screen, modes and colours.
    #[qml(invokable)]
    fn reset(cx: &Cx) {
        with_session(cx.engine, cx.this, |s| s.reset());
    }

    /// The screen and scrollback as escape sequences that draw them again:
    /// what `restore` takes.
    #[qml(invokable)]
    fn snapshot(cx: &Cx) -> String {
        with_session(cx.engine, cx.this, |s| s.snapshot().ok())
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    /// Draws a `snapshot` again, as output.
    #[qml(invokable)]
    fn restore(cx: &Cx, snapshot: String) {
        with_session(cx.engine, cx.this, |s| {
            s.reset();
            s.feed(snapshot.as_bytes());
        });
        flush_input(cx.engine, cx.this);
    }

    /// Starts the program again once it has exited. Returns whether it
    /// started.
    #[qml(invokable)]
    fn start(cx: &Cx) -> bool {
        let (e, me) = (cx.engine, cx.this);
        let alive = with_session(e, me, |s| s.is_alive()).unwrap_or(false);
        if alive || item::prop(e, me, Terminal::EXTERNAL) {
            return false;
        }
        let spawn = spawn_of(e, me);
        open(e, me, Some(spawn))
    }

    /// Says how the terminal's agent is, as the agent would with Hof's
    /// escape sequence: `state` is "working", "waiting", "idle" or "none",
    /// and `name` the agent's, or empty to let the terminal tell. It holds
    /// while the program in the foreground now is. Returns whether `state`
    /// is one of those.
    #[qml(invokable)]
    fn report_agent(cx: &Cx, state: String, name: String) -> bool {
        let Some(state) = State::parse(state.trim()) else {
            return false;
        };
        said(cx.engine, cx.this, state, name.trim().to_owned());
        true
    }

    /// Hangs up on the program.
    #[qml(invokable)]
    fn stop(cx: &Cx) {
        if let Some(shared) = shared_of(cx.engine, cx.this) {
            shared.session.borrow_mut().take();
            shared.remote.borrow_mut().take();
        }
        let _ = cx.engine.set_internal(cx.this, Terminal::RUNNING, false);
    }
}

pub(crate) fn shared_of(engine: &Engine, me: ObjectId) -> Option<Rc<Shared>> {
    engine
        .with::<Terminal, _>(me, |t| Rc::clone(&t.shared))
        .ok()
}

/// Runs `f` on the session, if there is one. The session is borrowed only
/// for `f`: what `f` sets off may run code that wants it again.
pub(crate) fn with_session<R>(
    engine: &Engine,
    me: ObjectId,
    f: impl FnOnce(&Session) -> R,
) -> Option<R> {
    let shared = shared_of(engine, me)?;
    let session = shared.session.borrow();
    session.as_ref().map(f)
}

fn cx(engine: &Engine, me: ObjectId) -> Cx<'_> {
    Cx::new(engine, me)
}

/// Sends what the session collected for the program out through `input`,
/// when there is no program of our own to take it.
pub(crate) fn flush_input(engine: &Engine, me: ObjectId) {
    if !engine.untrack(|| item::prop(engine, me, Terminal::EXTERNAL)) {
        return;
    }
    let bytes = with_session(engine, me, |s| s.take_input()).unwrap_or_default();
    if !bytes.is_empty() {
        let data = String::from_utf8_lossy(&bytes).into_owned();
        let _ = Terminal::emit_input(&cx(engine, me), data);
    }
}

fn spawn_of(engine: &Engine, me: ObjectId) -> Spawn {
    engine.untrack(|| {
        let program = item::prop(engine, me, Terminal::PROGRAM);
        let args = item::prop(engine, me, Terminal::ARGUMENTS);
        let cwd = item::prop(engine, me, Terminal::START_DIRECTORY);
        let environment = item::prop(engine, me, Terminal::ENVIRONMENT);
        Spawn {
            program: (!program.is_empty()).then(|| PathBuf::from(program)),
            args: args.into_iter().map(Into::into).collect(),
            cwd: (!cwd.is_empty()).then(|| PathBuf::from(cwd)),
            env: variables(&environment),
            ..Spawn::default()
        }
    })
}

/// `NAME=value` pairs as variables; an entry without `=` or a name is
/// skipped.
fn variables(entries: &[String]) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    entries
        .iter()
        .filter_map(|e| e.split_once('='))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
}

/// Opens the session: `spawn`'s program, or a detached one for `None`.
/// Returns whether it opened.
fn open(engine: &Engine, me: ObjectId, spawn: Option<Spawn>) -> bool {
    let Some(shared) = shared_of(engine, me) else {
        return false;
    };
    let options = engine.untrack(|| {
        let theme = theme_of(engine, me);
        Options {
            size: size_of(engine, me, &shared.view.borrow().metrics),
            scrollback: item::prop(engine, me, Terminal::SCROLLBACK).max(0) as usize,
            dark: is_dark(&theme),
            theme: Some(theme),
            cursor: shape_of(engine, me),
            cursor_blink: item::prop(engine, me, Terminal::CURSOR_BLINK),
            replies: item::prop(engine, me, Terminal::ANSWERS),
        }
    });
    let host = engine.untrack(|| item::prop(engine, me, Terminal::HOST));
    let elsewhere = spawn.is_some() && !host.is_empty();
    // The reader thread wakes the item through the inbox; one job is
    // posted per batch of news, however much the program writes.
    let post = Platform::of(engine).poster();
    let waker = move || post.send(move |e: &Engine| drain(e, me));
    let opened = match &spawn {
        _ if elsewhere => open_remote(engine, me, &shared, &host, options, waker),
        Some(spawn) => Session::spawn(spawn, options, waker).map_err(|e| e.to_string()),
        None => Session::detached(options, waker).map_err(|e| e.to_string()),
    };
    // Dropping the old session (if any) hangs up on its program.
    let session = match opened {
        Ok(s) => s,
        Err(err) => {
            let _ = engine.set_internal(me, Terminal::ERROR, err);
            return false;
        }
    };
    if !elsewhere {
        shared.remote.borrow_mut().take();
        let _ = engine.set_internal(me, Terminal::CONNECTION, String::new());
        let _ = engine.set_internal(me, Terminal::CONNECTION_MESSAGE, String::new());
    }
    let pid_running = spawn.is_some();
    // Before the program has had time to draw: its prompt lands after it.
    let replay = engine.untrack(|| item::prop(engine, me, Terminal::REPLAY));
    if !replay.is_empty() {
        session.feed(replay.as_bytes());
        // Plain text, on a line of its own: the old prompt stays above the
        // new one rather than under it.
        session.feed(b"\x1b[0m\r\n");
        let _ = engine.set_internal(me, Terminal::REPLAY, String::new());
    }
    *shared.session.borrow_mut() = Some(session);
    shared.view.borrow_mut().reset();
    let _ = engine.set_internal(me, Terminal::ERROR, String::new());
    let _ = engine.set_internal(me, Terminal::RUNNING, pid_running);
    let _ = engine.set_internal(me, Terminal::EXIT_CODE, 0);
    item::repaint(engine, me);
    true
}

/// Opens a session whose program runs on `host`: the pane `remotePane` is
/// there already, else a new one.
fn open_remote(
    engine: &Engine,
    me: ObjectId,
    shared: &Shared,
    host: &str,
    mut options: Options,
    waker: impl Fn() + Send + Sync + 'static,
) -> Result<Session, String> {
    let Some(to) = remote::host(host) else {
        return Err(format!("There is no host called \"{host}\" in [hosts]."));
    };
    // The agent beside the program answers it.
    options.replies = false;
    let size = options.size;
    let pane = engine.untrack(|| item::prop(engine, me, Terminal::REMOTE_PANE));
    let begin = match u32::try_from(pane) {
        Ok(pane) => Begin::Attach {
            pane,
            cols: size.cols,
            rows: size.rows,
        },
        Err(_) => Begin::Open(start_of(engine, me, size)),
    };
    let mut made = None;
    let session = Session::remote(options, waker, |feed| {
        let r = Remote::new(to, feed, begin);
        made = Some(Arc::clone(&r));
        Arc::new(Out(r))
    })
    .map_err(|e| e.to_string())?;
    // The session it replaces keeps its own, and closes its pane as it goes.
    *shared.remote.borrow_mut() = made;
    shared.attaching.set(pane >= 0);
    let _ = engine.set_internal(me, Terminal::CONNECTION, "connecting".to_owned());
    let _ = engine.set_internal(me, Terminal::CONNECTION_MESSAGE, String::new());
    Ok(session)
}

/// What to start on another machine: the properties as they are, the
/// program's path and directory as that machine knows them.
fn start_of(engine: &Engine, me: ObjectId, size: Size) -> Start {
    engine.untrack(|| {
        let program = item::prop(engine, me, Terminal::PROGRAM);
        let mut command = Vec::new();
        if !program.is_empty() {
            command.push(program);
            command.extend(item::prop(engine, me, Terminal::ARGUMENTS));
        }
        Start {
            command,
            directory: item::prop(engine, me, Terminal::START_DIRECTORY),
            environment: item::prop(engine, me, Terminal::ENVIRONMENT),
            cols: size.cols,
            rows: size.rows,
        }
    })
}

/// Hears how the connection to a remote pane is.
fn linked(engine: &Engine, me: ObjectId, link: Link) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let (state, why) = match link {
        Link::Connecting => ("connecting", String::new()),
        Link::Up { pane } => {
            shared.attaching.set(false);
            let _ = engine.set_internal(me, Terminal::REMOTE_PANE, pane as i32);
            ("up", String::new())
        }
        Link::Down(why) => ("down", why),
        Link::Gone(why) => {
            let _ = engine.set_internal(me, Terminal::REMOTE_PANE, -1);
            let remote = shared.remote.borrow().clone();
            match remote {
                // A restored pane whose machine forgot its program (it
                // restarted, say): a new one, under what the old showed.
                Some(remote) if shared.attaching.replace(false) => {
                    let size =
                        engine.untrack(|| size_of(engine, me, &shared.view.borrow().metrics));
                    remote.reopen(start_of(engine, me, size));
                    ("connecting", String::new())
                }
                _ => {
                    let _ = engine.set_internal(me, Terminal::RUNNING, false);
                    let _ = engine.set_internal(me, Terminal::ERROR, why.clone());
                    ("gone", why)
                }
            }
        }
    };
    let _ = engine.set_internal(me, Terminal::CONNECTION, state.to_owned());
    let _ = engine.set_internal(me, Terminal::CONNECTION_MESSAGE, why);
}

/// Tracked: `cursorShape`, a block for what it doesn't know.
fn shape_of(engine: &Engine, me: ObjectId) -> CursorShape {
    match item::prop(engine, me, Terminal::CURSOR_SHAPE).as_str() {
        "bar" | "beam" => CursorShape::Bar,
        "underline" => CursorShape::Underline,
        "hollow" => CursorShape::BlockHollow,
        _ => CursorShape::Block,
    }
}

fn size_of(engine: &Engine, me: ObjectId, m: &Metrics) -> Size {
    let cols = item::prop(engine, me, Terminal::COLUMNS).clamp(1, u16::MAX as i32) as u16;
    let rows = item::prop(engine, me, Terminal::ROWS).clamp(1, u16::MAX as i32) as u16;
    Size::new(cols, rows).with_cell(m.device_cell.0, m.device_cell.1)
}

fn rgb(c: Color) -> Option<Rgb> {
    c.is_valid().then(|| {
        let [r, g, b, _] = c.to_rgba8();
        Rgb::new(r, g, b)
    })
}

/// Tracked: the colours in the properties, as a theme for the session.
fn theme_of(engine: &Engine, me: ObjectId) -> Theme {
    Theme {
        foreground: rgb(item::prop(engine, me, Terminal::FOREGROUND_COLOR)),
        background: rgb(item::prop(engine, me, Terminal::BACKGROUND_COLOR)),
        cursor: rgb(item::prop(engine, me, Terminal::CURSOR_COLOR)),
        palette: item::prop(engine, me, Terminal::PALETTE)
            .into_iter()
            .take(256)
            .enumerate()
            .filter_map(|(i, c)| Some((i as u8, rgb(c)?)))
            .collect(),
    }
}

/// Without a background of our own, libghostty's default is dark.
fn is_dark(theme: &Theme) -> bool {
    theme.background.is_none_or(|b| {
        let luma = 0.2126 * f64::from(b.r) + 0.7152 * f64::from(b.g) + 0.0722 * f64::from(b.b);
        luma < 128.0
    })
}

/// Tracked: the cell follows the font, the system's text size and the
/// screen's pixel density; the counts follow the cell, the size and the
/// padding. Snapped to device pixels so that cells tile without seams.
fn measure(engine: &Engine, me: ObjectId) {
    let font = item::prop(engine, me, Terminal::FONT);
    let mut style = item::text_style(engine, &font);
    let dpr = item::device_pixel_ratio(engine, me).max(0.1);
    let padding = item::prop(engine, me, Terminal::PADDING).max(0.0);
    let width = item::prop(engine, me, Item::WIDTH);
    let height = item::prop(engine, me, Item::HEIGHT);
    // Tracked: either, set once the item is complete (from Rust, or by a
    // binding that resolves late), still opens the session.
    let external = item::prop(engine, me, Terminal::EXTERNAL);
    let auto_start = item::prop(engine, me, Terminal::AUTO_START);
    let Some(metrics) = item::with_text_engine(|te| {
        style.family = fonts::terminal_family(te, &style.family);
        Metrics::measure(te, &style, dpr)
    }) else {
        return;
    };
    let fit = |length: f64, by: f64| ((length - 2.0 * padding) / by).floor().max(0.0) as i32;
    let cols = fit(width, metrics.cell.width);
    let rows = fit(height, metrics.cell.height);
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    shared.view.borrow_mut().set_metrics(metrics.clone());
    engine.untrack(|| {
        let _ = engine.set_internal(me, Terminal::CELL_WIDTH, metrics.cell.width);
        let _ = engine.set_internal(me, Terminal::CELL_HEIGHT, metrics.cell.height);
        let was = (
            item::prop(engine, me, Terminal::COLUMNS),
            item::prop(engine, me, Terminal::ROWS),
        );
        let _ = engine.set_internal(me, Terminal::COLUMNS, cols);
        let _ = engine.set_internal(me, Terminal::ROWS, rows);
        if cols < 1 || rows < 1 {
            return;
        }
        let size = size_of(engine, me, &metrics);
        let resized = with_session(engine, me, |s| s.resize(size));
        if let Some(Err(err)) = &resized {
            let _ = engine.set_internal(me, Terminal::ERROR, err.to_string());
        }
        if resized.is_none() {
            if external {
                open(engine, me, None);
            } else if auto_start {
                open(engine, me, Some(spawn_of(engine, me)));
            }
        }
        if was != (cols, rows) {
            let _ = Terminal::emit_resized(&cx(engine, me), cols, rows);
        }
        update_scroll(engine, me);
    });
    item::repaint(engine, me);
}

pub(crate) fn scroll(engine: &Engine, me: ObjectId, to: Scroll) {
    with_session(engine, me, |s| s.scroll(to));
    update_scroll(engine, me);
    item::repaint(engine, me);
}

fn update_scroll(engine: &Engine, me: ObjectId) {
    let Some(bar) = with_session(engine, me, |s| s.scrollbar()) else {
        return;
    };
    let clamp = |n: u64| n.min(i32::MAX as u64) as i32;
    let _ = engine.set_internal(me, Terminal::SCROLL_TOTAL, clamp(bar.total));
    let _ = engine.set_internal(me, Terminal::SCROLL_OFFSET, clamp(bar.offset));
    let _ = engine.set_internal(me, Terminal::SCROLL_VISIBLE, clamp(bar.visible));
    let _ = engine.set_internal(me, Terminal::AT_BOTTOM, bar.at_bottom());
}

/// Hears what the session has news of: runs on the Engine thread, at most
/// once per batch, however much the program wrote.
fn drain(engine: &Engine, me: ObjectId) {
    let Some(news) = with_session(engine, me, |s| {
        let changes = s.take_changes();
        let events = if changes.contains(Changes::EVENTS) {
            s.take_events()
        } else {
            Vec::new()
        };
        let title = changes.contains(Changes::TITLE).then(|| s.title());
        let pwd = changes.contains(Changes::PWD).then(|| s.pwd());
        let tracking = s.mouse_tracking();
        let selected = s.has_selection();
        // New output can bring matches, or push old ones out.
        let found = changes.contains(Changes::SCREEN).then(|| s.found());
        (changes, events, title, pwd, tracking, selected, found)
    }) else {
        return;
    };
    let (changes, events, title, pwd, tracking, selected, found) = news;
    if changes.contains(Changes::SCREEN) {
        item::repaint(engine, me);
        update_scroll(engine, me);
        look_soon(engine, me);
        show_link(engine, me);
    }
    if let Some(found) = found {
        show_found(engine, me, found);
    }
    let _ = engine.set_internal(me, Terminal::HAS_SELECTION, selected);
    let _ = engine.set_internal(me, Terminal::MOUSE_TRACKING, tracking);
    if let Some(title) = title {
        let _ = engine.set_internal(me, Terminal::TITLE, title);
    }
    if let Some(pwd) = pwd {
        let _ = engine.set_internal(me, Terminal::WORKING_DIRECTORY, path_of(&pwd));
    }
    let cx = cx(engine, me);
    if changes.contains(Changes::BELL) {
        let _ = Terminal::emit_bell(&cx);
    }
    for event in events {
        match event {
            Event::Notification { title, body } => {
                let _ = Terminal::emit_notification(&cx, title, body);
            }
            Event::Progress(p) => {
                let (state, value) = match p {
                    Progress::Cleared => ("", 0),
                    Progress::Set(v) => ("normal", v),
                    Progress::Error(v) => ("error", v.unwrap_or(0)),
                    Progress::Indeterminate => ("indeterminate", 0),
                    Progress::Paused(v) => ("paused", v.unwrap_or(0)),
                };
                let _ = engine.set_internal(me, Terminal::PROGRESS_STATE, state.to_owned());
                let _ = engine.set_internal(me, Terminal::PROGRESS_VALUE, i32::from(value));
            }
            Event::Prompt(mark) => {
                let _ = engine.set_internal(me, Terminal::SHELL_INTEGRATION, true);
                match mark {
                    PromptMark::OutputStart { command } => {
                        let _ = engine.set_internal(me, Terminal::COMMAND_RUNNING, true);
                        let _ = Terminal::emit_command_started(&cx, command.unwrap_or_default());
                    }
                    PromptMark::CommandEnd { exit_code } => {
                        // Before the signal, so its handlers see the command done.
                        let _ = engine.set_internal(me, Terminal::COMMAND_RUNNING, false);
                        let _ = Terminal::emit_command_finished(&cx, exit_code.unwrap_or(-1));
                    }
                    _ => {}
                }
            }
            Event::Link(link) => linked(engine, me, link),
            Event::Agent(report) => {
                // A state Hof does not know, from a newer agent, says nothing.
                if let Some(state) = State::parse(&report.state) {
                    said(engine, me, state, report.name);
                }
            }
            Event::Copy { to, text } => {
                if engine.untrack(|| item::prop(engine, me, Terminal::CLIPBOARD_WRITE)) {
                    let _ = Platform::of(engine)
                        .clipboard()
                        .set_text(mode_of(to), &text);
                }
            }
            Event::ClipboardRead(from) => read_asked(engine, me, from),
            Event::Open(url) => open_asked(engine, me, url),
            Event::Exited(code) => {
                let code = code.unwrap_or(-1);
                forget_agent(engine, me);
                let _ = engine.set_internal(me, Terminal::RUNNING, false);
                if shared_of(engine, me).is_some_and(|s| s.remote.borrow().is_some()) {
                    // Its pane there is gone with it.
                    let _ = engine.set_internal(me, Terminal::REMOTE_PANE, -1);
                }
                let _ = engine.set_internal(me, Terminal::EXIT_CODE, code);
                let _ = Terminal::emit_exited(&cx, code);
            }
        }
    }
    flush_input(engine, me);
}

/// Takes what the terminal's agent said of itself, and looks at once.
fn said(engine: &Engine, me: ObjectId, state: State, name: String) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let group = shared
        .session
        .borrow()
        .as_ref()
        .and_then(Session::foreground)
        .map(|p| p.pid);
    shared.agent.borrow_mut().said = Some(Said { name, state, group });
    look(engine, me);
}

/// Looks at the agent a moment from now, unless a look is coming already.
fn look_soon(engine: &Engine, me: ObjectId) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let mut watch = shared.agent.borrow_mut();
    if watch.due.is_some() {
        return;
    }
    watch.due = Some(engine.call_at(engine.now() + agents::LOOK_AFTER, move |e| {
        if let Some(shared) = shared_of(e, me) {
            shared.agent.borrow_mut().due = None;
        }
        look(e, me);
    }));
}

/// Tells which agent the terminal runs and how it is, now.
fn look(engine: &Engine, me: ObjectId) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let judged = {
        let session = shared.session.borrow();
        let Some(s) = session.as_ref() else {
            return;
        };
        if !s.is_alive() {
            return;
        }
        let fg = s.foreground();
        let watch = shared.agent.borrow();
        agents::judge_now(fg.as_ref(), watch.said.as_ref(), || s.screen_text())
    };
    show_agent(engine, me, judged);
}

/// The program is gone, and its agent with it.
fn forget_agent(engine: &Engine, me: ObjectId) {
    if let Some(shared) = shared_of(engine, me) {
        let mut watch = shared.agent.borrow_mut();
        if let Some(id) = watch.due.take() {
            engine.cancel_call(id);
        }
        watch.said = None;
    }
    show_agent(engine, me, (String::new(), State::None));
}

fn show_agent(engine: &Engine, me: ObjectId, (name, state): (String, State)) {
    let (was, was_state) = engine.untrack(|| {
        (
            item::prop(engine, me, Terminal::AGENT),
            item::prop(engine, me, Terminal::AGENT_STATE),
        )
    });
    if was == name && was_state == state.name() {
        return;
    }
    let _ = engine.set_internal(me, Terminal::AGENT, name.clone());
    let _ = engine.set_internal(me, Terminal::AGENT_STATE, state.name().to_owned());
    let _ = Terminal::emit_agent_status(&cx(engine, me), name, state.name().to_owned());
}

fn clamp_count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Sets what QML reads of the search; `None` when there is none.
fn show_found(engine: &Engine, me: ObjectId, found: Option<Found>) {
    let f = found.unwrap_or_default();
    let current = f.current.map_or(0, |i| clamp_count(i).saturating_add(1));
    let _ = engine.set_internal(me, Terminal::SEARCHING, found.is_some());
    let _ = engine.set_internal(me, Terminal::SEARCH_TOTAL, clamp_count(f.total));
    let _ = engine.set_internal(me, Terminal::SEARCH_CURRENT, current);
}

fn step_found(engine: &Engine, me: ObjectId, step: fn(&Session) -> Result<Found, hof_vt::Error>) {
    let Some(found) = with_session(engine, me, |s| {
        let _ = step(s);
        s.found()
    }) else {
        return;
    };
    show_found(engine, me, found);
    // Stepping can scroll the view to the match.
    update_scroll(engine, me);
    item::repaint(engine, me);
}

/// OSC 7 says `file://host/path`, with the path percent-encoded.
fn path_of(url: &str) -> String {
    let Some(rest) = url.strip_prefix("file://") else {
        return url.to_owned();
    };
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `url` could name a file on this machine rather than a resource
/// with a scheme. Scheme names are ASCII case-insensitive.
fn names_a_file(url: &str) -> bool {
    let url = url.trim_start();
    match url.split_once(':') {
        Some((scheme, _))
            if scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) =>
        {
            scheme.eq_ignore_ascii_case("file")
        }
        _ => true,
    }
}

/// The program asked to open `url`: opens it, or asks the user.
fn open_asked(engine: &Engine, me: ObjectId, url: String) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    if names_a_file(&url)
        && shared.remote.borrow().is_some()
        && hof_remote::files::path_of_url(&url).is_none()
    {
        // A relative path on the other machine: what it is relative to,
        // this one can't tell.
        return;
    }
    let policy: String = engine.untrack(|| item::prop(engine, me, Terminal::OPEN_URLS));
    let first = {
        let mut opens = shared.opens.borrow_mut();
        match policy.as_str() {
            "deny" => return,
            "allow" => None,
            _ if opens.always => None,
            _ if opens.waiting.len() >= MOST_OPENS => return,
            _ => {
                opens.waiting.push(url.clone());
                Some(opens.waiting.len() == 1)
            }
        }
    };
    match first {
        None => open_wanted(engine, me, &url),
        Some(true) => {
            let _ = Terminal::emit_open_requested(&cx(engine, me), url);
        }
        Some(false) => {}
    }
}

/// Underlines the link under the pointer, if there is one, and says
/// which it is.
pub(crate) fn show_link(engine: &Engine, me: ObjectId) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let at = shared.pointer.borrow().hover_at;
    let link =
        at.and_then(|(col, row)| with_session(engine, me, |s| s.link_at(col, row)).flatten());
    let changed = {
        let mut view = shared.view.borrow_mut();
        let changed = view.link != link;
        if changed {
            view.link = link.clone();
        }
        changed
    };
    if changed {
        let url = link.map(|l| l.url).unwrap_or_default();
        let _ = engine.set_internal(me, Terminal::HOVERED_LINK, url);
        item::repaint(engine, me);
    }
}

/// The user clicked a link. A remote program's `file:` link names a file
/// on that machine: a copy of it is opened.
pub(crate) fn open_link(engine: &Engine, me: ObjectId, url: &str) {
    open_wanted(engine, me, url);
}

/// Opens `url` here; or, for a program on another machine naming a file
/// there, a copy of it here, kept the same as it (see
/// [`hof_remote::files::Mirror`]).
fn open_wanted(engine: &Engine, me: ObjectId, url: &str) {
    let remote = shared_of(engine, me).is_some_and(|s| s.remote.borrow().is_some());
    if !(remote && names_a_file(url)) {
        open_url(engine, url);
        return;
    }
    let Some(path) = hof_remote::files::path_of_url(url) else {
        return;
    };
    let host: String = engine.untrack(|| item::prop(engine, me, Terminal::HOST));
    let shown = String::from_utf8_lossy(&path).into_owned();
    let Some(mirror) = remote::mirror(&host) else {
        let why = format!("Could not open {shown}: {host} is not connected");
        let _ = Terminal::emit_notice(&cx(engine, me), why);
        return;
    };
    let _ = Terminal::emit_notice(&cx(engine, me), format!("Copying {shown} from {host}…"));
    let post = Platform::of(engine).poster();
    let fetching = std::thread::Builder::new()
        .name("hof-files-open".into())
        .spawn(move || {
            let fetched = mirror.fetch(&path);
            post.send(move |e: &Engine| match fetched {
                Ok(local) => {
                    let _ = Terminal::emit_notice(&cx(e, me), String::new());
                    open_url(e, &hof_remote::files::file_url(&local));
                }
                Err(why) => {
                    let why = format!("Could not open {shown} from {host}: {why}");
                    let _ = Terminal::emit_notice(&cx(e, me), why);
                }
            });
        });
    if fetching.is_err() {
        let _ = Terminal::emit_notice(&cx(engine, me), String::new());
    }
}

fn open_url(engine: &Engine, url: &str) {
    Platform::of(engine).open_url().open(url);
}

fn mode_of(clipboard: hof_vt::Clipboard) -> ClipboardMode {
    match clipboard {
        hof_vt::Clipboard::Standard => ClipboardMode::Clipboard,
        hof_vt::Clipboard::Selection => ClipboardMode::Selection,
    }
}

fn clipboard_name(clipboard: hof_vt::Clipboard) -> &'static str {
    match clipboard {
        hof_vt::Clipboard::Standard => "clipboard",
        hof_vt::Clipboard::Selection => "selection",
    }
}

/// As with opens, more waiting reads than this are dropped rather than
/// letting a program turn them into unbounded memory or answers.
const MOST_READS: usize = 8;

/// The program asked to read a clipboard: answers it, or asks the user.
fn read_asked(engine: &Engine, me: ObjectId, from: hof_vt::Clipboard) {
    let Some(shared) = shared_of(engine, me) else {
        return;
    };
    let policy: String = engine.untrack(|| item::prop(engine, me, Terminal::CLIPBOARD_READ));
    let allow = match policy.as_str() {
        "allow" => Some(true),
        "deny" => Some(false),
        _ if shared.clipboard.borrow().always => Some(true),
        _ => None,
    };
    if let Some(allow) = allow {
        answer_read(engine, me, from, allow);
        return;
    }
    let first = {
        let mut reads = shared.clipboard.borrow_mut();
        if reads.waiting.len() >= MOST_READS {
            return;
        }
        reads.waiting.push(from);
        reads.waiting.len() == 1
    };
    if first {
        let _ =
            Terminal::emit_clipboard_requested(&cx(engine, me), clipboard_name(from).to_owned());
    }
}

/// Tells the program what is on the clipboard it asked for, or that
/// nothing is, for a read it may not make; a program that asks waits for
/// an answer either way.
fn answer_read(engine: &Engine, me: ObjectId, from: hof_vt::Clipboard, allow: bool) {
    let text = if allow {
        Platform::of(engine)
            .clipboard()
            .text(mode_of(from))
            .unwrap_or_default()
    } else {
        String::new()
    };
    with_session(engine, me, |s| s.write(&hof_vt::carrying(from, &text)));
}

pub(crate) fn copy_selection(engine: &Engine, me: ObjectId, mode: ClipboardMode) -> bool {
    let Some(text) = with_session(engine, me, |s| s.selected_text()).flatten() else {
        return false;
    };
    Platform::of(engine)
        .clipboard()
        .set_text(mode, &text)
        .is_ok()
}

pub(crate) fn paste_from(engine: &Engine, me: ObjectId, mode: ClipboardMode) -> bool {
    let Some(text) = Platform::of(engine).clipboard().text(mode) else {
        return false;
    };
    paste(engine, me, &text, false)
}

fn paste(engine: &Engine, me: ObjectId, text: &str, force: bool) -> bool {
    let force = force || !engine.untrack(|| item::prop(engine, me, Terminal::CONFIRM_PASTE));
    let done = with_session(engine, me, |s| s.paste(text, force));
    let ok = match done {
        Some(Ok(())) => true,
        Some(Err(PasteError::Unsafe)) => {
            let _ = Terminal::emit_paste_blocked(&cx(engine, me), text.to_owned());
            false
        }
        Some(Err(PasteError::Failed(err))) => {
            let _ = engine.set_internal(me, Terminal::ERROR, err.to_string());
            false
        }
        None => false,
    };
    if ok {
        scroll(engine, me, Scroll::Bottom);
    }
    flush_input(engine, me);
    ok
}

// Qt names letter keys by their capital's code.
const KEY_C: i32 = 'C' as i32;
const KEY_V: i32 = 'V' as i32;

/// The terminal's own shortcuts, before the program sees the key.
fn shortcut(engine: &Engine, me: ObjectId, ev: &KeyEvent) -> bool {
    use ljos::item::key;
    if !item::prop(engine, me, Terminal::SHORTCUTS) {
        return false;
    }
    let m = ev.modifiers & !ljos::item::Modifiers::KEYPAD;
    let ctrl_shift = ljos::item::Modifiers::CONTROL | ljos::item::Modifiers::SHIFT;
    let shift = ljos::item::Modifiers::SHIFT;
    let page = item::prop(engine, me, Terminal::ROWS).max(1) as isize;
    match (m, ev.key) {
        (m, KEY_C) if m == ctrl_shift => {
            copy_selection(engine, me, ClipboardMode::Clipboard);
        }
        (m, KEY_V) if m == ctrl_shift => {
            paste_from(engine, me, ClipboardMode::Clipboard);
        }
        (m, key::INSERT) if m == shift => {
            paste_from(engine, me, ClipboardMode::Selection);
        }
        (m, key::PAGE_UP) if m == shift => scroll(engine, me, Scroll::Lines(-page)),
        (m, key::PAGE_DOWN) if m == shift => scroll(engine, me, Scroll::Lines(page)),
        (m, key::HOME) if m == shift => scroll(engine, me, Scroll::Top),
        (m, key::END) if m == shift => scroll(engine, me, Scroll::Bottom),
        (_, key::COPY) => {
            copy_selection(engine, me, ClipboardMode::Clipboard);
        }
        (_, key::PASTE) => {
            paste_from(engine, me, ClipboardMode::Clipboard);
        }
        _ => return false,
    }
    true
}

pub(crate) fn key_press(engine: &Engine, me: ObjectId, ev: &KeyEvent) -> bool {
    if shortcut(engine, me, ev) {
        return true;
    }
    let mac = item::prop(engine, me, Terminal::MAC_MODIFIERS);
    let Some(input) = keys::translate(ev, mac) else {
        return false;
    };
    let sent = with_session(engine, me, |s| s.key(&input));
    let took = matches!(sent, Some(Ok(true)));
    if took {
        // Typing brings the view back to the prompt, as in every terminal.
        let behind = with_session(engine, me, |s| !s.scrollbar().at_bottom()).unwrap_or(false);
        if behind {
            scroll(engine, me, Scroll::Bottom);
        }
        with_session(engine, me, |s| s.clear_selection());
    }
    flush_input(engine, me);
    took || sent.is_some()
}

pub(crate) fn key_release(engine: &Engine, me: ObjectId, ev: &KeyEvent) -> bool {
    let mac = item::prop(engine, me, Terminal::MAC_MODIFIERS);
    let Some(input) = keys::translate(ev, mac) else {
        return false;
    };
    // Only the Kitty protocol reports releases; elsewhere this sends nothing.
    let sent = with_session(engine, me, |s| s.key(&input));
    flush_input(engine, me);
    sent.is_some()
}

/// A terminal types into a program, not into a text it edits: the input
/// method sees an empty text with the cursor at its start, and what it
/// commits is sent on.
pub(crate) struct Typing;

impl TextInputClient for Typing {
    fn surrounding(&self, _: &Engine, _: ObjectId) -> Surrounding {
        Surrounding {
            text: String::new(),
            anchor: 0,
            cursor: 0,
        }
    }

    fn replace(&self, engine: &Engine, me: ObjectId, _: Range<usize>, text: &str) {
        if let Some(shared) = shared_of(engine, me) {
            shared.preedit.borrow_mut().clear();
        }
        with_session(engine, me, |s| s.write(text.as_bytes()));
        flush_input(engine, me);
        item::repaint(engine, me);
    }

    fn set_selection(&self, _: &Engine, _: ObjectId, _: usize, _: usize) {}

    fn cursor_rect(&self, engine: &Engine, me: ObjectId) -> kurbo::Rect {
        let padding = item::prop(engine, me, Terminal::PADDING).max(0.0);
        let Some(shared) = shared_of(engine, me) else {
            return kurbo::Rect::ZERO;
        };
        let view = shared.view.borrow();
        view.cursor_rect(padding)
    }

    fn hints(&self, _: &Engine, _: ObjectId) -> i32 {
        // Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText | Qt.ImhPreferLatin:
        // a shell is no place for guessed words.
        0x2 | 0x40 | 0x200
    }

    fn set_preedit(&self, engine: &Engine, me: ObjectId, text: &str, _: Option<usize>) -> bool {
        let Some(shared) = shared_of(engine, me) else {
            return false;
        };
        text.clone_into(&mut shared.preedit.borrow_mut());
        item::repaint(engine, me);
        true
    }

    fn preedit(&self, engine: &Engine, me: ObjectId) -> String {
        shared_of(engine, me)
            .map(|s| s.preedit.borrow().clone())
            .unwrap_or_default()
    }

    fn commit(&self, engine: &Engine, me: ObjectId, text: &str) -> bool {
        self.replace(engine, me, 0..0, text);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_directories_are_paths() {
        assert_eq!(path_of("file://host/home/me/a%20b"), "/home/me/a b");
        assert_eq!(path_of("file:///tmp"), "/tmp");
        assert_eq!(path_of("/already"), "/already");
    }

    #[test]
    fn local_files_are_known_whatever_their_scheme_case() {
        for url in [
            "/etc/passwd",
            "~/a",
            "relative.desktop",
            " file:///tmp/x",
            "file:///tmp/x",
            "FILE:///tmp/x",
            "File:/tmp/x",
            ":bad",
        ] {
            assert!(names_a_file(url), "{url}");
        }
        for url in ["https://a.example", "mailto:me@example.com", "a1+b.c:x"] {
            assert!(!names_a_file(url), "{url}");
        }
    }
}
