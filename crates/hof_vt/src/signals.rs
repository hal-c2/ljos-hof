//! How a session tells the UI something happened, without the UI polling and
//! without flooding it.
//!
//! Whatever happens is a bit in a pending set. The waker fires only on the
//! first bit after the UI last drained the set, so a program writing a
//! thousand frames between two UI frames costs the UI one wake, and the UI
//! reads the screen as it is then. Things the UI must not lose, such as
//! notifications, go into a queue alongside.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

bitflags::bitflags! {
    /// What changed since the UI last asked.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Changes: u32 {
        /// The screen needs drawing again.
        const SCREEN = 1 << 0;
        const TITLE = 1 << 1;
        /// The working directory the shell reported (OSC 7).
        const PWD = 1 << 2;
        const BELL = 1 << 3;
        /// Terminal modes the UI acts on: mouse tracking, cursor blink and the like.
        const MODES = 1 << 4;
        /// There are [`Event`]s to take.
        const EVENTS = 1 << 5;
        /// The program has exited; an [`Event::Exited`] says how.
        const EXITED = 1 << 6;
    }
}

/// Things a session reports that the UI must not miss or merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A desktop notification (OSC 9 / OSC 777).
    Notification { title: String, body: String },
    /// Progress the program reports (OSC 9;4), for a tab or taskbar.
    Progress(Progress),
    /// A shell-integration mark (OSC 133): where prompts, input and output begin.
    Prompt(PromptMark),
    /// The program exited, with its status if it exited normally.
    Exited(Option<i32>),
    /// A remote session's connection changed.
    Link(crate::remote::Link),
    /// The program said how its agent is, in Hof's own sequence.
    Agent(crate::report::Report),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Cleared,
    Set(u8),
    Error(Option<u8>),
    Indeterminate,
    Paused(Option<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMark {
    PromptStart,
    InputStart,
    /// The command was submitted and its output begins.
    OutputStart {
        command: Option<String>,
    },
    CommandEnd {
        exit_code: Option<i32>,
    },
}

pub(crate) type Waker = Box<dyn Fn() + Send + Sync>;

pub(crate) struct Signals {
    pending: AtomicU32,
    /// Whether a wake is out that the UI hasn't answered yet.
    woken: AtomicBool,
    /// Synchronized output (mode 2026): the program asked us not to show the
    /// screen until it finishes a frame.
    held: AtomicBool,
    events: Mutex<Vec<Event>>,
    waker: Waker,
}

impl Signals {
    pub fn new(waker: Waker) -> Self {
        Self {
            pending: AtomicU32::new(0),
            woken: AtomicBool::new(false),
            held: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
            waker,
        }
    }

    /// Record a change without waking anyone. Callbacks from inside
    /// libghostty use this; the caller kicks once it has released the lock.
    pub fn mark(&self, changes: Changes) {
        self.pending.fetch_or(changes.bits(), Ordering::AcqRel);
    }

    pub fn push(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
        self.mark(Changes::EVENTS);
    }

    pub fn set_held(&self, held: bool) {
        self.held.store(held, Ordering::Release);
    }

    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    /// Wake the UI if something is pending, nothing holds the screen, and no
    /// wake is already out.
    pub fn kick(&self) {
        if self.is_held() || self.pending.load(Ordering::Acquire) == 0 {
            return;
        }
        if !self.woken.swap(true, Ordering::AcqRel) {
            (self.waker)();
        }
    }

    pub fn take(&self) -> Changes {
        // Re-arm before draining: a change landing between the two then
        // wakes again rather than being stranded until the next one.
        self.woken.store(false, Ordering::Release);
        Changes::from_bits_retain(self.pending.swap(0, Ordering::AcqRel))
    }

    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    fn counting() -> (Signals, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let signals = Signals::new(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        (signals, wakes)
    }

    #[test]
    fn many_changes_before_the_ui_drains_cost_one_wake() {
        let (signals, wakes) = counting();
        for _ in 0..1000 {
            signals.mark(Changes::SCREEN);
            signals.kick();
        }
        signals.mark(Changes::TITLE);
        signals.kick();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(signals.take(), Changes::SCREEN | Changes::TITLE);

        signals.mark(Changes::SCREEN);
        signals.kick();
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_held_screen_wakes_only_when_released() {
        let (signals, wakes) = counting();
        signals.set_held(true);
        signals.mark(Changes::SCREEN);
        signals.kick();
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        signals.set_held(false);
        signals.kick();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn nothing_pending_wakes_nobody() {
        let (signals, wakes) = counting();
        signals.kick();
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }
}
