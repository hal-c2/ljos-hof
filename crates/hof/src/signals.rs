//! Hof ended by a signal closes its window as Quit does, so the session is
//! saved and remote panes left as the settings say. That covers Ctrl+C
//! where it was started (SIGINT), the terminal it was started in going
//! (SIGHUP), and the desktop session ending (SIGTERM). A second signal of
//! the same kind ends it at once, as it would have without this.
//!
//! The handler only writes a byte to a pipe, which is all a handler may
//! safely do; a thread reads it and posts the close to the UI thread.

use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, PoisonError};

use ljos::prelude::*;

const SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// The pipe's end the handler writes to; -1 until [`catch`].
static WRITE: AtomicI32 = AtomicI32::new(-1);

/// The window to close, and the way to its thread; `None` until it is up.
static WINDOW: Mutex<Option<(Post, ObjectId)>> = Mutex::new(None);

extern "C" fn heard(_: libc::c_int) {
    let fd = WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: `write` is async-signal-safe; one byte from the stack.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

/// Catches SIGINT, SIGTERM and SIGHUP from now on, for the window
/// [`close_on_signal`] names. For Hof's own process, before its window.
pub fn catch() -> std::io::Result<()> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` is two descriptors' room.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for fd in fds {
        // Not for the panes' programs.
        // SAFETY: a descriptor just made, owned here.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let [read, write] = fds;
    WRITE.store(write, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("hof-signals".into())
        .spawn(move || listen(read))?;
    for signal in SIGNALS {
        // SAFETY: a zeroed `sigaction` with only the handler and flags set
        // is a valid one; `heard` does only what a handler may.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = heard as *const () as libc::sighandler_t;
            // Once: the next of its kind does what it did before.
            action.sa_flags = libc::SA_RESETHAND | libc::SA_RESTART;
            libc::sigemptyset(&raw mut action.sa_mask);
            if libc::sigaction(signal, &raw const action, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// The window a signal closes: `window`, on `engine`'s thread.
pub(crate) fn close_on_signal(engine: &Engine, window: ObjectId) {
    *WINDOW.lock().unwrap_or_else(PoisonError::into_inner) =
        Some((Platform::of(engine).poster(), window));
}

fn listen(read: RawFd) {
    loop {
        let mut byte = 0u8;
        // SAFETY: one byte into the stack, from the pipe made for this.
        let got = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
        if got < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if got <= 0 {
            return;
        }
        let window = WINDOW.lock().unwrap_or_else(PoisonError::into_inner);
        match window.as_ref() {
            Some((post, window)) => {
                let window = *window;
                post.send(move |e: &Engine| {
                    let _ = e.invoke_by_name(window, "close", &[]);
                });
            }
            // No window yet: nothing to save.
            None => std::process::exit(128 + libc::SIGTERM),
        }
    }
}
