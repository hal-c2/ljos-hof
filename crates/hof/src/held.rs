//! Which session a window keeps, and that no other Hof keeps it as well:
//! two windows of one session would save over each other, and attach to
//! the same programs.
//!
//! A window holds its session by a lock on `hof-session[-NAME].lock` in the
//! runtime directory for as long as it runs. The system lets it go when the
//! process goes, however it goes, so no lock is ever left behind.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hof_session::DEFAULT;
use rustix::fs::{FlockOperation, flock};

/// The most sessions a plain `hof` tries before it gives up: `default`,
/// then `2`, `3` and on, one for each window open at once.
const MOST: u32 = 99;

/// A session this window keeps, held until it is dropped.
#[derive(Debug)]
pub struct Held {
    pub name: String,
    _lock: File,
}

/// Holds the session `name`, or `None` when another Hof does.
pub fn hold(name: &str) -> io::Result<Option<Held>> {
    hold_in(&runtime_dir(), name)
}

/// Holds the first session of a plain `hof`'s no other Hof has:
/// `default`, then `2`, `3` and on. A window opened beside another so
/// keeps its own, rather than the other's.
pub fn hold_first_free() -> io::Result<Held> {
    hold_first_free_in(&runtime_dir())
}

/// Whether a Hof has the session `name` open.
pub fn is_open(name: &str) -> bool {
    matches!(hold(name), Ok(None))
}

/// Where the window keeping the session `name` takes `hof ctl`, as does
/// the next one to: the programs that ran on reach whichever it is.
pub fn control_socket(name: &str) -> PathBuf {
    runtime_dir().join(if name == DEFAULT {
        "hof-session.sock".to_owned()
    } else {
        format!("hof-session-{name}.sock")
    })
}

/// The session saved last that no Hof has open: what `hof attach` opens
/// when it is given no name.
pub fn last_free() -> Option<String> {
    hof_session::saved()
        .into_iter()
        .filter(|(name, _)| !is_open(name))
        .max_by_key(|(_, file)| saved_at(file))
        .map(|(name, _)| name)
}

/// A saved session, as `hof sessions` lists it.
#[derive(Debug)]
pub struct Listed {
    pub name: String,
    /// A Hof has it open.
    pub open: bool,
    /// Its tabs and panes; `None` when it could not be read.
    pub held: Option<(usize, usize)>,
    pub saved: SystemTime,
}

/// The sessions saved: `default` first, then the rest by name.
pub fn list() -> Vec<Listed> {
    hof_session::saved()
        .into_iter()
        .map(|(name, file)| Listed {
            open: is_open(&name),
            held: hof_session::load(&file).ok().flatten().map(|s| {
                let tabs = s.windows.iter().map(|w| w.tabs.len()).sum();
                let panes = s
                    .windows
                    .iter()
                    .flat_map(|w| &w.tabs)
                    .map(|t| t.panes().len())
                    .sum();
                (tabs, panes)
            }),
            saved: saved_at(&file),
            name,
        })
        .collect()
}

/// When `file` was saved, or the epoch if that can't be told.
pub fn saved_at(file: &Path) -> SystemTime {
    file.metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn hold_in(dir: &Path, name: &str) -> io::Result<Option<Held>> {
    if !hof_session::is_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{name:?} is no session name: letters, digits, '.', '_' and '-', \
                 not first a '.' or '-'"
            ),
        ));
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // Closed on exec as Rust opens every file: a program a pane starts
    // never holds the session on after Hof.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(lock_name(name)))?;
    match flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(Held {
            name: name.to_owned(),
            _lock: lock,
        })),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn hold_first_free_in(dir: &Path) -> io::Result<Held> {
    let names = std::iter::once(DEFAULT.to_owned()).chain((2..=MOST).map(|n| n.to_string()));
    for name in names {
        if let Some(held) = hold_in(dir, &name)? {
            return Ok(held);
        }
    }
    Err(io::Error::other(format!(
        "{MOST} Hofs keep a session already"
    )))
}

fn lock_name(name: &str) -> String {
    if name == DEFAULT {
        "hof-session.lock".to_owned()
    } else {
        format!("hof-session-{name}.lock")
    }
}

/// Where the locks and the sessions' control sockets are: the runtime
/// directory, beside the MCP socket.
fn runtime_dir() -> PathBuf {
    let mcp = hof_mcp::socket_path();
    mcp.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_held_by_one_hof_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let work = hold_in(dir.path(), "work").unwrap().expect("free");
        assert_eq!(work.name, "work");
        assert!(hold_in(dir.path(), "work").unwrap().is_none(), "held");
        assert!(hold_in(dir.path(), "other").unwrap().is_some());
        drop(work);
        assert!(hold_in(dir.path(), "work").unwrap().is_some(), "let go");
    }

    #[test]
    fn a_plain_hof_beside_another_keeps_the_next_session() {
        let dir = tempfile::tempdir().unwrap();
        let first = hold_first_free_in(dir.path()).unwrap();
        let second = hold_first_free_in(dir.path()).unwrap();
        let third = hold_first_free_in(dir.path()).unwrap();
        assert_eq!(
            [&first.name, &second.name, &third.name],
            ["default", "2", "3"]
        );
        drop(second);
        assert_eq!(hold_first_free_in(dir.path()).unwrap().name, "2");
    }

    #[test]
    fn a_name_no_session_can_have_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let e = hold_in(dir.path(), "../up").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(!dir.path().join("hof-session-../up.lock").exists());
    }
}
