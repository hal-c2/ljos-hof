//! A terminal session: the terminal state, the threads that feed it, and the
//! operations the UI performs on it.

use std::io::Write as _;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::paste;
use libghostty_vt::screen::{CellWide, GridRef, RowSemanticPrompt};
use libghostty_vt::selection::{FormatOptions, SelectLineOptions, SelectWordOptions, Selection};
use libghostty_vt::terminal::{
    ClipboardMime, CursorStyle, Mode, Point, PointCoordinate, PointSpace, ScrollViewport,
};
use libghostty_vt::{Terminal, focus};

use crate::find::{Finder, Found, Highlight};
use crate::links::{self, Hyperlink, LineText};
use crate::pty::{self, Pty, Spawn};
use crate::remote::{Feed, Outlet, Slot};
use crate::signals::{Changes, Event, Signals};
pub use crate::vt::Selecting;
pub use crate::vt::Tap;
use crate::vt::{Anchor, Identity, Sink, Vt};
use crate::{CursorShape, Error, KeyInput, MouseInput, Rgb, Size};

/// How long synchronized output may hold the screen before we show it
/// anyway, so a program that never ends its frame can't freeze the view.
const HOLD_LIMIT: Duration = Duration::from_secs(1);

/// The most output parsed under one lock. Small enough that the UI waits at
/// most about a millisecond to read the screen, large enough that a flood is
/// parsed in few passes.
pub(crate) const READ_CHUNK: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Options {
    pub size: Size,
    /// Lines of history kept above the screen.
    pub scrollback: usize,
    /// Whether to tell programs that ask (mode 2031) that the theme is dark.
    pub dark: bool,
    pub theme: Option<Theme>,
    /// The cursor programs get until they ask for another, and get back
    /// when they reset it (DECSCUSR 0).
    pub cursor: CursorShape,
    pub cursor_blink: bool,
    /// Answer the program's queries (device attributes, colours, sizes).
    /// The local copy of a remote pane doesn't: the agent's copy, beside
    /// the program, answers, and two answers would be one too many.
    pub replies: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: Size::default(),
            scrollback: 10_000,
            dark: true,
            theme: None,
            cursor: CursorShape::Block,
            cursor_blink: false,
            replies: true,
        }
    }
}

/// The colours a session starts with; programs may change them later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette entries to replace, by index.
    pub palette: Vec<(u8, Rgb)>,
}

/// Where to move the view of the scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Up is negative.
    Lines(isize),
    Top,
    Bottom,
    /// To an absolute line, counted from the top of the scrollback.
    Row(usize),
    /// By shell prompts (needs shell integration); back is negative.
    Prompts(isize),
}

/// Where the view is in the scrollback, in lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub visible: u64,
}

impl Scrollbar {
    pub fn at_bottom(self) -> bool {
        self.offset + self.visible >= self.total
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PasteError {
    /// The text could run commands the user didn't see (a newline while the
    /// program hasn't enabled bracketed paste). Ask, then paste with
    /// `allow_unsafe`.
    #[error("the pasted text could run commands")]
    Unsafe,
    #[error(transparent)]
    Failed(#[from] Error),
}

pub(crate) struct Shared {
    vt: Mutex<Vt>,
    pub(crate) signals: Arc<Signals>,
    pub(crate) alive: AtomicBool,
    /// How the program ended, once it has.
    exit: Mutex<Option<Option<i32>>>,
    /// A remote session's foreground, as its machine last said.
    pub(crate) remote_foreground: Mutex<Option<crate::Process>>,
}

impl Shared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Vt> {
        // A panic while parsing leaves the terminal as consistent as
        // libghostty left it; keep going rather than poison the UI.
        self.vt.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct Session {
    shared: Arc<Shared>,
    sink: Sink,
    child: Option<Child>,
    /// Where a remote session's program is.
    outlet: Option<Arc<Slot>>,
    dark: Arc<AtomicBool>,
    /// Whether replies reach the program now; see [`Options::replies`].
    replying: Arc<AtomicBool>,
    replies: bool,
}

struct Child {
    pid: u32,
    master: Arc<OwnedFd>,
}

/// A program on a pty, handed from another process: see
/// [`Session::adopt_tapped`].
pub struct Handed {
    pub master: OwnedFd,
    pub pid: u32,
    /// Its `TERM`.
    pub term: String,
    pub screen: Vec<u8>,
}

/// How a session's program is waited for once its pty is done.
enum Reap {
    /// One this process started.
    Child(std::process::Child),
    /// One started by the process this one was before it became this
    /// program: still a child, by its number.
    Pid(u32),
}

impl Reap {
    /// Its exit code, `None` for a signal (or when it can't be known).
    fn wait(self) -> Option<i32> {
        match self {
            Reap::Child(mut child) => child.wait().ok().and_then(|status| status.code()),
            Reap::Pid(pid) => {
                let pid = rustix::process::Pid::from_raw(i32::try_from(pid).ok()?)?;
                loop {
                    match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty())
                    {
                        Ok(Some((_, status))) => return status.exit_status(),
                        Ok(None) => return None,
                        Err(rustix::io::Errno::INTR) => continue,
                        Err(_) => return None,
                    }
                }
            }
        }
    }
}

impl Session {
    /// Run a program on a new pseudo-terminal.
    ///
    /// `waker` is called, from the session's own thread, when there are
    /// [`Changes`] to take; it should only post a message to the UI.
    pub fn spawn(
        spawn: &Spawn,
        options: Options,
        waker: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Error> {
        Self::spawn_tapped(spawn, options, waker, None)
    }

    /// [`Session::spawn`], with `tap` seeing the program's output as it is
    /// parsed: what `hof agent` sends on to the machine showing the pane.
    pub fn spawn_tapped(
        spawn: &Spawn,
        options: Options,
        waker: impl Fn() + Send + Sync + 'static,
        tap: Option<Tap>,
    ) -> Result<Self, Error> {
        let input = Arc::new(crate::vt::PtyInput::default());
        let sink = Sink::Pty(input.clone());
        let identity = Identity::of(&options);
        let shared = Self::shared(
            &options,
            spawn.term_name(),
            sink.clone(),
            Box::new(waker),
            identity.clone(),
        )?;
        let (dark, replying) = (identity.scheme, identity.replies);
        shared.lock().tap = tap;

        let Pty { master, child } = Pty::spawn(spawn, options.size.clamped())?;
        let pid = child.id();
        let child = Self::run_pty(&shared, input, master, pid, Reap::Child(child))?;
        Ok(Self {
            shared,
            sink,
            child: Some(child),
            outlet: None,
            dark,
            replying,
            replies: options.replies,
        })
    }

    /// A session for a program another process started on a pty and
    /// handed over, as it showed it: `master`, the pty's master side,
    /// `pid`, the program, a child of this process, and `screen`, a
    /// [`Session::snapshot`] of its terminal there. `tap` sees what the
    /// program writes from now on.
    pub fn adopt_tapped(
        handed: Handed,
        options: Options,
        waker: impl Fn() + Send + Sync + 'static,
        tap: Option<Tap>,
    ) -> Result<Self, Error> {
        let input = Arc::new(crate::vt::PtyInput::default());
        let sink = Sink::Pty(input.clone());
        let identity = Identity::of(&options);
        let shared = Self::shared(
            &options,
            &handed.term,
            sink.clone(),
            Box::new(waker),
            identity.clone(),
        )?;
        let (dark, replying) = (identity.scheme, identity.replies);
        {
            let mut vt = shared.lock();
            vt.output(&handed.screen);
            vt.tap = tap;
        }
        let child = Self::run_pty(
            &shared,
            input,
            handed.master,
            handed.pid,
            Reap::Pid(handed.pid),
        )?;
        Ok(Self {
            shared,
            sink,
            child: Some(child),
            outlet: None,
            dark,
            replying,
            replies: options.replies,
        })
    }

    /// The pty's master side, for handing the program to another process.
    pub fn master(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        use std::os::fd::AsFd;
        self.child.as_ref().map(|c| c.master.as_fd())
    }

    /// How the program ended, once it has: its code, `None` for a signal.
    pub fn exit_status(&self) -> Option<Option<i32>> {
        *self.shared.exit.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads and writes the program's pty, on threads of their own, until
    /// it ends, and then waits for it with `reap`.
    fn run_pty(
        shared: &Arc<Shared>,
        input: Arc<crate::vt::PtyInput>,
        master: OwnedFd,
        pid: u32,
        reap: Reap,
    ) -> Result<Child, Error> {
        let master = Arc::new(master);
        let reader = master.try_clone().map_err(|e| Error::Pty("dup", e))?;
        let writer = master.try_clone().map_err(|e| Error::Pty("dup", e))?;

        std::thread::Builder::new()
            .name("hof-pty-write".into())
            .spawn(move || {
                let mut out = std::fs::File::from(writer);
                // Ends when the session closes the input, or the program
                // stops reading and its pty goes away.
                while let Some(bytes) = input.read() {
                    if out.write_all(&bytes).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| Error::Thread("writer", e))?;

        let io = shared.clone();
        crate::freeze::reader_started();
        let started = std::thread::Builder::new()
            .name("hof-pty-read".into())
            .spawn(move || {
                read_loop(&io, &reader);
                // Done with the output: a freeze need not wait for the
                // program's end, which may be a while off.
                crate::freeze::reader_ended();
                let code = reap.wait();
                *io.exit.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);
                io.alive.store(false, Ordering::Release);
                io.signals.set_held(false);
                io.signals.push(Event::Exited(code));
                io.signals.mark(Changes::EXITED | Changes::SCREEN);
                io.signals.kick();
            });
        if let Err(e) = started {
            crate::freeze::reader_ended();
            return Err(Error::Thread("reader", e));
        }
        Ok(Child { pid, master })
    }

    /// A session with no program. Output comes in through [`Session::feed`];
    /// the terminal's replies and the user's input collect for
    /// [`Session::take_input`].
    pub fn detached(
        options: Options,
        waker: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, Error> {
        let sink = Sink::Buffer(Arc::default());
        let identity = Identity::of(&options);
        let shared = Self::shared(
            &options,
            "xterm-256color",
            sink.clone(),
            Box::new(waker),
            identity.clone(),
        )?;
        let (dark, replying) = (identity.scheme, identity.replies);
        Ok(Self {
            shared,
            sink,
            child: None,
            outlet: None,
            dark,
            replying,
            replies: options.replies,
        })
    }

    /// A session whose program runs elsewhere, under `hof agent` on its
    /// machine. `connect` gets the [`Feed`] the program's output is to be
    /// written into, from any thread, and gives back where input and the
    /// size go.
    ///
    /// Such a session usually doesn't answer queries (`options.replies`
    /// off): the agent beside the program has.
    pub fn remote(
        options: Options,
        waker: impl Fn() + Send + Sync + 'static,
        connect: impl FnOnce(Feed) -> Arc<dyn Outlet>,
    ) -> Result<Self, Error> {
        let slot = Arc::new(Slot::default());
        let sink = Sink::Outlet(slot.clone());
        let identity = Identity::of(&options);
        let shared = Self::shared(
            &options,
            "xterm-256color",
            sink.clone(),
            Box::new(waker),
            identity.clone(),
        )?;
        slot.set(connect(Feed {
            shared: Arc::downgrade(&shared),
        }));
        Ok(Self {
            shared,
            sink,
            child: None,
            outlet: Some(slot),
            dark: identity.scheme,
            replying: identity.replies,
            replies: options.replies,
        })
    }

    fn shared(
        options: &Options,
        term_name: &str,
        sink: Sink,
        waker: crate::signals::Waker,
        identity: Identity,
    ) -> Result<Arc<Shared>, Error> {
        let signals = Arc::new(Signals::new(waker));
        let mut vt = Vt::new(
            options.size,
            options.scrollback,
            term_name,
            sink,
            signals.clone(),
            identity,
        )?;
        if let Some(theme) = &options.theme {
            apply_theme(&mut vt.term, theme)?;
        }
        apply_cursor(&mut vt.term, options.cursor, options.cursor_blink)?;
        let shared = Arc::new(Shared {
            vt: Mutex::new(vt),
            signals,
            alive: AtomicBool::new(true),
            exit: Mutex::new(None),
            remote_foreground: Mutex::new(None),
        });
        Ok(shared)
    }

    pub(crate) fn shared_state(&self) -> &Shared {
        &self.shared
    }

    /// Mark changes made from the UI side and wake for them as for any other.
    fn changed(&self, changes: Changes) {
        self.shared.signals.mark(changes);
        self.shared.signals.kick();
    }

    // ---- What happened ----------------------------------------------------

    /// What changed since the last call. Taking re-arms the waker.
    pub fn take_changes(&self) -> Changes {
        self.shared.signals.take()
    }

    pub fn take_events(&self) -> Vec<Event> {
        self.shared.signals.take_events()
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }

    /// The program's process id, for a session with a program.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.pid)
    }

    /// The directory the program (the shell) works in now, as the system
    /// says; `None` for a remote session or when it won't say.
    pub fn directory(&self) -> Option<std::path::PathBuf> {
        crate::process::directory_of(self.pid()?)
    }

    /// What runs in the foreground now: the shell's command, if it is
    /// running one, else the shell. A remote session's is what its machine
    /// last said. `None` for a session whose program is nowhere Hof can
    /// see, or when the system won't say.
    pub fn foreground(&self) -> Option<crate::Process> {
        match &self.child {
            Some(child) => crate::process::foreground(&*child.master),
            None => self
                .shared
                .remote_foreground
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    // ---- Output -----------------------------------------------------------

    /// Parse output as if the program had written it.
    pub fn feed(&self, bytes: &[u8]) {
        self.shared.lock().output(bytes);
        self.changed(Changes::SCREEN);
    }

    /// For a detached session: what was typed and what the terminal replied,
    /// since the last call. Always empty for a session with a program.
    pub fn take_input(&self) -> Vec<u8> {
        match &self.sink {
            Sink::Buffer(buf) => {
                std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()))
            }
            Sink::Pty(_) | Sink::Outlet(_) => Vec::new(),
        }
    }

    // ---- Input ------------------------------------------------------------

    /// Send bytes to the program as they are.
    pub fn write(&self, bytes: &[u8]) {
        self.sink.write(bytes);
    }

    /// Send a key in the program's keyboard protocol. Returns whether the key
    /// meant anything to the terminal; a modifier on its own usually doesn't.
    pub fn key(&self, input: &KeyInput) -> Result<bool, Error> {
        let mut vt = self.shared.lock();
        let bytes = vt.encode_key(input)?;
        self.sink.write(bytes);
        Ok(!bytes.is_empty())
    }

    /// Report a mouse event, if the program tracks the mouse. Returns whether
    /// it did, so the UI can handle the event itself (selection) otherwise.
    pub fn mouse(&self, input: &MouseInput) -> Result<bool, Error> {
        let mut vt = self.shared.lock();
        if !vt.term.is_mouse_tracking()? {
            return Ok(false);
        }
        let bytes = vt.encode_mouse(input)?;
        self.sink.write(bytes);
        Ok(true)
    }

    pub fn mouse_tracking(&self) -> bool {
        self.shared.lock().term.is_mouse_tracking().unwrap_or(false)
    }

    /// Paste text the way the program asked for pastes (bracketed or not).
    pub fn paste(&self, text: &str, allow_unsafe: bool) -> Result<(), PasteError> {
        let mimes = [ClipboardMime::new("text/plain")];
        let options = paste::Options::new().with_allow_unsafe(allow_unsafe);
        let mut vt = self.shared.lock();
        // The paste leaves through the replies' callback. Nothing else can
        // reply while the lock is held, so this lets only the paste out.
        self.replying.store(true, Ordering::Relaxed);
        let result = vt
            .term
            .paste(options, &mimes, |_, out| out.write_all(text.as_bytes()));
        self.replying.store(self.replies, Ordering::Relaxed);
        drop(vt);
        match result {
            Ok(_) => Ok(()),
            Err(libghostty_vt::Error::Rejected) => Err(PasteError::Unsafe),
            Err(e) => Err(PasteError::Failed(e.into())),
        }
    }

    /// Tell the program the window gained or lost focus, if it asked to know.
    pub fn focus(&self, focused: bool) {
        let vt = self.shared.lock();
        if !vt.term.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
            return;
        }
        let mut buf = [0u8; 8];
        let event = if focused {
            focus::Event::Gained
        } else {
            focus::Event::Lost
        };
        if let Ok(n) = event.encode(&mut buf) {
            self.sink.write(&buf[..n]);
        }
    }

    // ---- Geometry and view ------------------------------------------------

    pub fn size(&self) -> Size {
        self.shared.lock().size
    }

    pub fn resize(&self, size: Size) -> Result<(), Error> {
        let size = size.clamped();
        {
            let mut vt = self.shared.lock();
            if vt.size == size {
                return Ok(());
            }
            vt.resize(size)?;
        }
        if let Some(child) = &self.child {
            // Also delivers SIGWINCH to the program in the foreground.
            pty::set_winsize(&*child.master, size)?;
        }
        if let Some(outlet) = self.outlet.as_ref().and_then(|o| o.get()) {
            outlet.resize(size);
        }
        self.changed(Changes::SCREEN);
        Ok(())
    }

    pub fn scroll(&self, scroll: Scroll) {
        let viewport = match scroll {
            Scroll::Lines(n) => ScrollViewport::Delta(n),
            Scroll::Top => ScrollViewport::Top,
            Scroll::Bottom => ScrollViewport::Bottom,
            Scroll::Row(n) => ScrollViewport::Row(n),
            Scroll::Prompts(n) => ScrollViewport::DeltaPrompt(n),
        };
        self.shared.lock().term.scroll_viewport(viewport);
        self.changed(Changes::SCREEN);
    }

    pub fn scrollbar(&self) -> Scrollbar {
        let vt = self.shared.lock();
        vt.term
            .scrollbar()
            .map(|s| Scrollbar {
                total: s.total,
                offset: s.offset,
                visible: s.len,
            })
            .unwrap_or_default()
    }

    // ---- What the program said about itself ------------------------------

    pub fn title(&self) -> String {
        let vt = self.shared.lock();
        vt.term
            .title()
            .map(|t| String::from_utf8_lossy(t).into_owned())
            .unwrap_or_default()
    }

    /// The working directory the shell reported, as it reported it (usually
    /// a `file://host/path` URL).
    pub fn pwd(&self) -> String {
        let vt = self.shared.lock();
        vt.term
            .pwd()
            .map(|t| String::from_utf8_lossy(t).into_owned())
            .unwrap_or_default()
    }

    // ---- Appearance -------------------------------------------------------

    pub fn set_theme(&self, theme: &Theme) -> Result<(), Error> {
        apply_theme(&mut self.shared.lock().term, theme)?;
        self.changed(Changes::SCREEN);
        Ok(())
    }

    /// The cursor programs get back when they reset theirs, and have now:
    /// a program that set its own loses it until it sets it again, which
    /// editors do as they change modes.
    pub fn set_cursor(&self, shape: CursorShape, blink: bool) -> Result<(), Error> {
        apply_cursor(&mut self.shared.lock().term, shape, blink)?;
        self.changed(Changes::SCREEN);
        Ok(())
    }

    pub fn set_dark(&self, dark: bool) {
        self.dark.store(dark, Ordering::Relaxed);
    }

    pub fn set_scrollback(&self, lines: usize) -> Result<(), Error> {
        self.shared
            .lock()
            .term
            .set_scrollback_max_lines(Some(lines))?;
        Ok(())
    }

    /// A full reset (RIS), as if the terminal were new.
    pub fn reset(&self) {
        self.shared.lock().term.reset();
        self.changed(Changes::SCREEN | Changes::TITLE | Changes::MODES);
    }

    // ---- Selection and text -----------------------------------------------

    /// Start a selection at a cell of the view. Words and lines select their
    /// unit at once; cells wait for the drag.
    pub fn select_start(&self, col: u16, row: u16, unit: Selecting) -> Result<(), Error> {
        let mut guard = self.shared.lock();
        let Vt { term, anchor, .. } = &mut *guard;
        let point = viewport(col, row);
        *anchor = Some(Anchor {
            at: term.track_grid_ref(point)?,
            unit,
        });
        let first = match unit {
            Selecting::Cells | Selecting::Block => None,
            Selecting::Words => term.select_word(SelectWordOptions::new(term.grid_ref(point)?))?,
            Selecting::Lines => term.select_line(SelectLineOptions::new(term.grid_ref(point)?))?,
        };
        term.set_selection(first.as_ref())?;
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(())
    }

    /// Extend the selection from where it started to a cell of the view.
    pub fn select_extend(&self, col: u16, row: u16) -> Result<(), Error> {
        let guard = self.shared.lock();
        let Some(anchor) = &guard.anchor else {
            return Ok(());
        };
        let term = &guard.term;
        let Some(from) = anchor.at.snapshot(term)? else {
            return Ok(());
        };
        let to = term.grid_ref(viewport(col, row))?;
        let selection = match anchor.unit {
            Selecting::Cells => Some(Selection::new(from, to, false)),
            Selecting::Block => Some(Selection::new(from, to, true)),
            Selecting::Words => span(
                term,
                term.select_word(SelectWordOptions::new(from))?,
                term.select_word(SelectWordOptions::new(to))?,
            ),
            Selecting::Lines => span(
                term,
                term.select_line(SelectLineOptions::new(from))?,
                term.select_line(SelectLineOptions::new(to))?,
            ),
        };
        term.set_selection(selection.as_ref())?;
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(())
    }

    /// Select the output of the command at a cell of the view (needs shell
    /// integration).
    pub fn select_output(&self, col: u16, row: u16) -> Result<bool, Error> {
        let guard = self.shared.lock();
        let term = &guard.term;
        let selection = term.select_output(term.grid_ref(viewport(col, row))?)?;
        term.set_selection(selection.as_ref())?;
        let found = selection.is_some();
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(found)
    }

    /// The link at a cell of the view: the program's own (OSC 8), else a
    /// URL in the text of the line there.
    pub fn link_at(&self, col: u16, row: u16) -> Option<Hyperlink> {
        let guard = self.shared.lock();
        let term = &guard.term;
        let Size { cols, rows, .. } = guard.size;
        if col >= cols || row >= rows {
            return None;
        }
        let row_of = |r: u16| term.grid_ref(viewport(0, r)).and_then(|g| g.row()).ok();
        let mut top = row;
        while top > 0 && row_of(top).is_some_and(|r| r.is_wrap_continuation().unwrap_or(false)) {
            top -= 1;
        }
        let mut bottom = row;
        while bottom + 1 < rows && row_of(bottom).is_some_and(|r| r.is_wrapped().unwrap_or(false)) {
            bottom += 1;
        }
        let cells = (top..=bottom).flat_map(|r| (0..cols).map(move |c| (r, c)));

        let here = term.grid_ref(viewport(col, row)).ok()?;
        if here.cell().and_then(|c| c.has_hyperlink()).unwrap_or(false) {
            let url = uri_of(&here)?;
            let same = |&(r, c): &(u16, u16)| {
                term.grid_ref(viewport(c, r))
                    .ok()
                    .and_then(|g| uri_of(&g))
                    .is_some_and(|u| u == url)
            };
            let cells: Vec<_> = cells.collect();
            let at = cells.iter().position(|&cell| cell == (row, col))?;
            let start = cells[..at]
                .iter()
                .rposition(|c| !same(c))
                .map_or(0, |i| i + 1);
            let end = cells[at..]
                .iter()
                .position(|c| !same(c))
                .map_or(cells.len(), |i| at + i);
            return Some(Hyperlink {
                url,
                spans: links::spans_of(&cells[start..end]),
            });
        }

        let mut line = LineText::default();
        let mut chars = ['\0'; 16];
        for (r, c) in cells {
            if line.text.len() > links::MOST_LINE {
                break;
            }
            let Ok(cell) = term.grid_ref(viewport(c, r)) else {
                continue;
            };
            if matches!(
                cell.cell().and_then(|c| c.wide()),
                Ok(CellWide::SpacerTail | CellWide::SpacerHead)
            ) {
                continue;
            }
            match cell.graphemes(&mut chars) {
                Ok(0) | Err(_) => line.push(' ', r, c),
                Ok(n) => chars[..n].iter().for_each(|&ch| line.push(ch, r, c)),
            }
        }
        line.url_at(row, col)
    }

    pub fn select_all(&self) -> Result<(), Error> {
        let guard = self.shared.lock();
        let all = guard.term.select_all()?;
        guard.term.set_selection(all.as_ref())?;
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(())
    }

    pub fn clear_selection(&self) {
        let mut guard = self.shared.lock();
        guard.anchor = None;
        let had = matches!(guard.term.selection(), Ok(Some(_)));
        let _ = guard.term.set_selection(None);
        drop(guard);
        if had {
            self.changed(Changes::SCREEN);
        }
    }

    pub fn has_selection(&self) -> bool {
        matches!(self.shared.lock().term.selection(), Ok(Some(_)))
    }

    /// The selected text, with soft wraps joined and trailing blanks trimmed.
    pub fn selected_text(&self) -> Option<String> {
        let vt = self.shared.lock();
        let options = FormatOptions::new()
            .with_emit_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true);
        let bytes = vt.term.format_selection_alloc(None, options).ok()??;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    // ---- Finding ----------------------------------------------------------

    /// Look for `needle` on the screen and in the scrollback, and make the
    /// newest match current, scrolled into view. ASCII letters match either
    /// case. An empty needle ends the search.
    pub fn find(&self, needle: &str) -> Result<Found, Error> {
        if needle.is_empty() {
            self.end_find();
            return Ok(Found::default());
        }
        let mut guard = self.shared.lock();
        let Vt { term, finder, .. } = &mut *guard;
        let finder = match finder {
            Some(f) => f,
            None => finder.insert(Finder::new(term)?),
        };
        let found = finder.set(term, needle)?;
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(found)
    }

    /// Move to the next older match, wrapping around to the newest. Nothing
    /// when there is no search.
    pub fn find_older(&self) -> Result<Found, Error> {
        self.step_find(Finder::older)
    }

    /// Move to the next newer match, wrapping around to the oldest.
    pub fn find_newer(&self) -> Result<Found, Error> {
        self.step_find(Finder::newer)
    }

    fn step_find(
        &self,
        step: fn(&mut Finder, &mut Terminal<'static, 'static>) -> Result<Found, Error>,
    ) -> Result<Found, Error> {
        let mut guard = self.shared.lock();
        let Vt { term, finder, .. } = &mut *guard;
        let Some(finder) = finder else {
            return Ok(Found::default());
        };
        let found = step(finder, term)?;
        drop(guard);
        self.changed(Changes::SCREEN);
        Ok(found)
    }

    /// Stop looking, and highlight nothing.
    pub fn end_find(&self) {
        let had = self.shared.lock().finder.take().is_some();
        if had {
            self.changed(Changes::SCREEN);
        }
    }

    /// How the search stands now, output since included; `None` when there
    /// is no search.
    pub fn found(&self) -> Option<Found> {
        let mut guard = self.shared.lock();
        let Vt { term, finder, .. } = &mut *guard;
        finder.as_mut()?.catch_up(term).ok()
    }

    /// What to highlight in view for the search; empty when there is none.
    pub fn highlights(&self) -> Vec<Highlight> {
        let mut out = Vec::new();
        let mut guard = self.shared.lock();
        let Vt {
            term, finder, size, ..
        } = &mut *guard;
        if let Some(finder) = finder {
            let _ = finder.highlights(term, size.cols, size.rows, &mut out);
        }
        out
    }

    /// Everything on the screen and in the scrollback, as plain text.
    pub fn text(&self) -> String {
        let vt = self.shared.lock();
        let options = FormatterOptions::new()
            .with_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true);
        format(&vt.term, options)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    /// The screen as plain text, a line per row with trailing blanks
    /// trimmed: what is on it, not what the view is scrolled to.
    pub fn screen_text(&self) -> String {
        let vt = self.shared.lock();
        let (cols, rows) = (vt.size.cols, vt.size.rows);
        let corner = |x: u16, y: u16| vt.term.grid_ref(active(x, y));
        let (Ok(start), Ok(end)) = (
            corner(0, 0),
            corner(cols.saturating_sub(1), rows.saturating_sub(1)),
        ) else {
            return String::new();
        };
        let all = Selection::new(start, end, false);
        let options = FormatterOptions::new()
            .with_format(Format::Plain)
            .with_trim(true)
            .with_selection(&all);
        format(&vt.term, options)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    // ---- Commands run for someone else ------------------------------------

    /// Mark the cursor's line as the one a command is about to be typed on,
    /// so `command_output` can find what it prints. The mark moves with the
    /// line as it scrolls into the scrollback.
    pub fn mark_command(&self) -> Result<(), Error> {
        let mut vt = self.shared.lock();
        let y = vt.term.cursor_y()?;
        let mark = vt.term.track_grid_ref(active(0, y))?;
        vt.command_mark = Some(mark);
        Ok(())
    }

    /// What the marked command has printed so far: everything after its
    /// own line up to the cursor, soft wraps joined and trailing blanks
    /// trimmed. `None` with no mark, or when its line has left the
    /// scrollback.
    pub fn command_output(&self) -> Option<String> {
        let vt = self.shared.lock();
        let term = &vt.term;
        let start = vt.command_mark.as_ref()?.snapshot(term).ok()??;
        let end = term
            .grid_ref(active(term.cursor_x().ok()?, term.cursor_y().ok()?))
            .ok()?;
        let span = Selection::new(start, end, false);
        let options = FormatterOptions::new()
            .with_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true)
            .with_selection(&span);
        let text = format(term, options).ok()?;
        let text = String::from_utf8_lossy(&text);
        let mut lines: Vec<&str> = text.lines().collect();
        // The shell's next prompt, drawn by the time the command is read
        // as done, is not the command's; the first line is the prompt the
        // command was typed at.
        let next_prompt = prompt_rows_at_cursor(term).min(lines.len().saturating_sub(1));
        lines.truncate(lines.len() - next_prompt);
        Some(
            lines
                .get(1..)
                .unwrap_or_default()
                .join("\n")
                .trim_end()
                .to_owned(),
        )
    }

    /// Forget the command mark.
    pub fn end_command(&self) {
        self.shared.lock().command_mark = None;
    }

    /// The whole terminal as VT sequences that rebuild it: contents, styles,
    /// modes, cursor and colours. Feeding this to a fresh session restores it.
    pub fn snapshot(&self) -> Result<Vec<u8>, Error> {
        self.snapshot_with(|| ()).map(|(bytes, ())| bytes)
    }

    /// [`Session::snapshot`], and `also` run at the same moment, under the
    /// lock the output is parsed under: no output comes in between, so
    /// what a tap counted in `also` is exactly what the snapshot shows.
    pub fn snapshot_with<R>(&self, also: impl FnOnce() -> R) -> Result<(Vec<u8>, R), Error> {
        let vt = self.shared.lock();
        let also = also();
        let options = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_palette(true)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_tabstops(true)
            .with_pwd(true)
            .with_keyboard(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_protection(true)
            .with_kitty_keyboard(true)
            .with_charsets(true);
        Ok((format(&vt.term, options)?, also))
    }

    /// Run something against libghostty's terminal directly, for what this
    /// API doesn't cover yet. Don't replace its callbacks; the session needs
    /// them.
    pub fn with_terminal<R>(&self, f: impl FnOnce(&mut Terminal<'static, 'static>) -> R) -> R {
        let result = f(&mut self.shared.lock().term);
        self.changed(Changes::SCREEN);
        result
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.sink.close();
        if let Some(child) = &self.child
            && self.is_alive()
        {
            pty::hangup(child.pid);
        }
        if let Some(outlet) = self.outlet.as_ref().and_then(|o| o.get()) {
            outlet.closed();
        }
    }
}

/// How many rows of a prompt the cursor is in, counting up from its row to
/// the prompt's first: 0 when it is not in one.
fn prompt_rows_at_cursor(term: &Terminal<'static, 'static>) -> usize {
    let Ok(y) = term.cursor_y() else {
        return 0;
    };
    let prompt = |row: u16| {
        term.grid_ref(active(0, row))
            .and_then(|r| r.row())
            .and_then(|r| r.semantic_prompt())
            .unwrap_or(RowSemanticPrompt::None)
    };
    let mut rows = 0;
    for row in (0..=y).rev() {
        match prompt(row) {
            RowSemanticPrompt::None => return 0,
            RowSemanticPrompt::Continuation => rows += 1,
            RowSemanticPrompt::Prompt => return rows + 1,
        }
    }
    0
}

/// A cell's OSC 8 link, if it has one.
fn uri_of(cell: &GridRef<'_>) -> Option<String> {
    let mut buf = vec![0; 512];
    let len = match cell.hyperlink_uri(&mut buf) {
        Ok(len) => len,
        Err(libghostty_vt::Error::OutOfSpace { required }) => {
            buf.resize(required, 0);
            cell.hyperlink_uri(&mut buf).ok()?
        }
        Err(_) => return None,
    };
    (len > 0).then(|| String::from_utf8_lossy(&buf[..len]).into_owned())
}

fn active(col: u16, row: u16) -> Point {
    Point::Active(PointCoordinate {
        x: col,
        y: u32::from(row),
    })
}

fn viewport(col: u16, row: u16) -> Point {
    Point::Viewport(PointCoordinate {
        x: col,
        y: u32::from(row),
    })
}

/// The selection from the earlier start of two units to the later end, so a
/// drag by words or lines covers both the unit it began in and the one it's over.
fn span<'t>(
    term: &'t Terminal<'_, '_>,
    a: Option<Selection<'t>>,
    b: Option<Selection<'t>>,
) -> Option<Selection<'t>> {
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        (one, other) => return one.or(other),
    };
    let key = |g: &GridRef<'_>| {
        term.point_from_grid_ref(g, PointSpace::Screen)
            .ok()
            .flatten()
            .map(|p| (p.y, p.x))
    };
    let start = if key(&a.start()) <= key(&b.start()) {
        a.start()
    } else {
        b.start()
    };
    let end = if key(&a.end()) >= key(&b.end()) {
        a.end()
    } else {
        b.end()
    };
    Some(Selection::new(start, end, false))
}

fn format(term: &Terminal<'_, '_>, options: FormatterOptions<'_, '_>) -> Result<Vec<u8>, Error> {
    let mut formatter = Formatter::new(term, options)?;
    Ok(formatter.format_alloc(None)?.to_vec())
}

fn apply_theme(term: &mut Terminal<'static, 'static>, theme: &Theme) -> Result<(), Error> {
    term.set_default_fg_color(theme.foreground.map(Into::into))?
        .set_default_bg_color(theme.background.map(Into::into))?
        .set_default_cursor_color(theme.cursor.map(Into::into))?;
    if !theme.palette.is_empty() {
        let mut palette = term.default_color_palette()?;
        for &(index, color) in &theme.palette {
            palette.set(libghostty_vt::style::PaletteIndex(index), color.into());
        }
        term.set_default_color_palette(Some(palette))?;
    }
    Ok(())
}

fn apply_cursor(
    term: &mut Terminal<'static, 'static>,
    shape: CursorShape,
    blink: bool,
) -> Result<(), Error> {
    let style = match shape {
        CursorShape::Block => CursorStyle::Block,
        CursorShape::BlockHollow => CursorStyle::BlockHollow,
        CursorShape::Bar => CursorStyle::Bar,
        CursorShape::Underline => CursorStyle::Underline,
    };
    term.set_default_cursor_style(Some(style))?
        .set_default_cursor_blink(Some(blink))?;
    // The defaults apply on a reset only: this is that reset.
    term.vt_write(b"\x1b[0 q");
    Ok(())
}

/// Read the program's output and parse it until the program is gone.
fn read_loop(io: &Shared, fd: &OwnedFd) {
    use rustix::event::{PollFd, PollFlags, Timespec};
    use rustix::io::Errno;

    let mut buf = vec![0u8; READ_CHUNK];
    let limit = Timespec {
        tv_sec: HOLD_LIMIT.as_secs() as _,
        tv_nsec: 0,
    };
    loop {
        // Output, or the word to stop reading it (a freeze), whichever
        // comes first. Only while a program holds the screen is there a
        // timeout as well.
        let held = io.signals.is_held();
        let wake = crate::freeze::wake_fd();
        let mut fds = [
            PollFd::new(fd, PollFlags::IN),
            PollFd::new(&wake, PollFlags::IN),
        ];
        match rustix::event::poll(&mut fds, held.then_some(&limit)) {
            Ok(0) => {
                io.signals.set_held(false);
                io.signals.kick();
                continue;
            }
            Err(Errno::INTR) => continue,
            _ => {}
        }
        if !fds[1].revents().is_empty() {
            crate::freeze::stop_while_frozen();
            continue;
        }
        match rustix::io::read(fd, &mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let mut vt = io.lock();
                vt.output(&buf[..n]);
                if let Some(tap) = &mut vt.tap {
                    tap(&buf[..n]);
                }
                drop(vt);
                io.signals.mark(Changes::SCREEN);
                io.signals.kick();
            }
            Err(Errno::INTR | Errno::AGAIN) => continue,
            // EIO once the last process with the slave open is gone.
            Err(_) => return,
        }
    }
}
