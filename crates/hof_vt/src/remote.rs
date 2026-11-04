//! Sessions whose program runs elsewhere: on another machine, under
//! `hof agent`. The output comes in through a [`Feed`] from whatever
//! thread the connection reads on; what the user types and the size go out
//! through an [`Outlet`].

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};

use crate::Size;
use crate::session::Shared;
use crate::signals::{Changes, Event};

/// Where a remote session sends what it has for the program: the
/// connection to the program's machine.
pub trait Outlet: Send + Sync {
    /// What the user typed or pasted, and replies when the session gives
    /// them, already encoded.
    fn input(&self, bytes: &[u8]);
    /// The grid's new size, for the program's pty.
    fn resize(&self, size: Size);
    /// The session is gone: its program can go too, or be kept for later.
    fn closed(&self);
}

/// How the connection behind a remote session is, as a [`Event::Link`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// Reaching the program's machine, for the first time or again.
    Connecting,
    /// The program runs, as this pane of its machine's agent, and its
    /// output comes in.
    Up { pane: u32 },
    /// The connection dropped and is being made again; the program runs on.
    Down(String),
    /// It is over and won't come back: the program could not start, or its
    /// machine forgot it.
    Gone(String),
}

/// The output side of a remote session, for the connection to write into.
/// It holds the session weakly: once the session is gone, what comes is
/// dropped.
#[derive(Clone)]
pub struct Feed {
    pub(crate) shared: Weak<Shared>,
}

impl Feed {
    /// The program's output, parsed as a local program's is.
    pub fn output(&self, bytes: &[u8]) {
        let Some(io) = self.shared.upgrade() else {
            return;
        };
        io.lock().output(bytes);
        io.signals.mark(Changes::SCREEN);
        io.signals.kick();
    }

    /// The screen as it is now, from a snapshot: everything before goes.
    pub fn replace(&self, snapshot: &[u8]) {
        let Some(io) = self.shared.upgrade() else {
            return;
        };
        {
            let mut vt = io.lock();
            vt.term.reset();
            vt.term.vt_write(snapshot);
        }
        // A frame the program was holding back is over with the reset.
        io.signals.set_held(false);
        io.signals
            .mark(Changes::SCREEN | Changes::TITLE | Changes::PWD | Changes::MODES);
        io.signals.kick();
    }

    /// The program ended, with its status if it exited normally.
    pub fn exited(&self, code: Option<i32>) {
        let Some(io) = self.shared.upgrade() else {
            return;
        };
        io.alive.store(false, Ordering::Release);
        io.signals.set_held(false);
        io.signals.push(Event::Exited(code));
        io.signals.mark(Changes::EXITED | Changes::SCREEN);
        io.signals.kick();
    }

    /// How the connection is now.
    pub fn link(&self, link: Link) {
        let Some(io) = self.shared.upgrade() else {
            return;
        };
        if !matches!(link, Link::Up { .. }) {
            // The rest of a held frame may never come; show what there is.
            io.signals.set_held(false);
        }
        io.signals.push(Event::Link(link));
        io.signals.kick();
    }

    /// Whether the session is still there to write into.
    pub fn is_open(&self) -> bool {
        self.shared.strong_count() > 0
    }
}

/// The outlet a remote session sends to, set once the connection has the
/// session's [`Feed`]; what comes before is dropped, as nothing can carry
/// it yet.
#[derive(Default)]
pub(crate) struct Slot(OnceLock<Arc<dyn Outlet>>);

impl Slot {
    pub fn set(&self, outlet: Arc<dyn Outlet>) {
        let _ = self.0.set(outlet);
    }

    pub fn get(&self) -> Option<&Arc<dyn Outlet>> {
        self.0.get()
    }
}
