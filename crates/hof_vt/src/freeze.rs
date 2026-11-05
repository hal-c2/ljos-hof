//! Holding every program's output where it is, unread, for as long as it
//! takes to hand the terminals to another process: what `hof agent` does
//! to become a newer one of itself without its programs noticing.
//!
//! Each reader waits on its program's pty and on one pipe the whole
//! process shares. [`freeze`] writes to the pipe: each reader finishes
//! what it read, and then stops before reading more, so whatever the
//! programs write after stays in the kernel for whoever reads the pty
//! next. Dropping the [`Frozen`] lets them read on.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

struct Switch {
    /// Readable while frozen: what wakes a reader waiting for output.
    wake: io::PipeReader,
    set: io::PipeWriter,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    frozen: bool,
    /// Readers that may still read a program's output.
    reading: usize,
    /// Of those, the ones stopped for the freeze.
    stopped: usize,
}

fn switch() -> &'static Switch {
    static SWITCH: OnceLock<Switch> = OnceLock::new();
    SWITCH.get_or_init(|| {
        let (wake, set) = io::pipe().expect("a pipe for freezing the readers");
        // Emptied without waiting, once thawed.
        let _ = rustix::fs::fcntl_setfl(&wake, rustix::fs::OFlags::NONBLOCK);
        Switch {
            wake,
            set,
            state: Mutex::default(),
            changed: Condvar::new(),
        }
    })
}

impl Switch {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What a reader waits on beside its pty: readable when it is to stop.
pub(crate) fn wake_fd() -> BorrowedFd<'static> {
    switch().wake.as_fd()
}

/// A reader starts: [`freeze`] waits for it too from now on.
pub(crate) fn reader_started() {
    switch().state().reading += 1;
}

/// A reader is done with its pty for good, its program's end told.
pub(crate) fn reader_ended() {
    let s = switch();
    s.state().reading -= 1;
    s.changed.notify_all();
}

/// Called by a reader that saw [`wake_fd`] readable: stops until thawed,
/// if it is still frozen.
pub(crate) fn stop_while_frozen() {
    let s = switch();
    let mut state = s.state();
    if !state.frozen {
        return;
    }
    state.stopped += 1;
    s.changed.notify_all();
    while state.frozen {
        state = s.changed.wait(state).unwrap_or_else(|e| e.into_inner());
    }
    state.stopped -= 1;
}

/// The readers stopped; dropped, they read on.
#[must_use]
pub struct Frozen(());

/// Stops every program's output being read, in every session of this
/// process, once each reader is done with what it has: within `within`,
/// or not at all.
pub fn freeze(within: Duration) -> io::Result<Frozen> {
    let s = switch();
    let deadline = Instant::now() + within;
    let mut state = s.state();
    if state.frozen {
        return Err(io::Error::other("the readers are frozen already"));
    }
    state.frozen = true;
    (&s.set).write_all(&[1])?;
    let frozen = Frozen(());
    while state.stopped < state.reading {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            drop(state);
            drop(frozen);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "a program's output was still being read",
            ));
        }
        state = s
            .changed
            .wait_timeout(state, left)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    Ok(frozen)
}

impl Drop for Frozen {
    fn drop(&mut self) {
        let s = switch();
        let mut state = s.state();
        let mut drained = [0; 16];
        while matches!((&s.wake).read(&mut drained), Ok(n) if n > 0) {}
        state.frozen = false;
        s.changed.notify_all();
    }
}
