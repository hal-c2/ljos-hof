//! Which Hof has a session open, so that no other one does as well: two
//! windows of one session would save over each other, and attach to the
//! same programs.
//!
//! A session is held by a lock on `hof-session[-NAME].lock` in the runtime
//! directory, for as long as whoever holds it runs: a window here, or the
//! agent for a Hof on another machine that has it open. The system lets it
//! go when the process goes, however it goes, so no lock is left behind.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};

use crate::DEFAULT;

/// The most sessions a plain `hof` tries before it gives up: `default`,
/// then `2`, `3` and on, one for each window open at once.
const MOST: u32 = 99;

/// A session held open until this is dropped.
#[derive(Debug)]
pub struct Held {
    pub name: String,
    _lock: File,
}

/// Where the locks are: `$XDG_RUNTIME_DIR`, else `hof-UID` in the
/// temporary directory. The same as Hof's MCP socket's, so that Hof and the
/// agent beside it find the same locks.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        // The spec says to ignore a relative one.
        Some(dir) if dir.is_absolute() => dir,
        _ => std::env::temp_dir().join(format!("hof-{}", rustix::process::getuid().as_raw())),
    }
}

/// Holds the session `name`, its lock in `dir`, or `None` when another
/// Hof has it open.
pub fn hold_in(dir: &Path, name: &str) -> io::Result<Option<Held>> {
    if !crate::is_name(name) {
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
    // Closed on exec as Rust opens every file: a program started from here
    // never holds the session on after it.
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

/// Holds the first session of a plain `hof`'s no other Hof has, its lock
/// in `dir`: `default`, then `2`, `3` and on. A window opened beside
/// another so keeps its own, rather than the other's.
pub fn hold_first_free_in(dir: &Path) -> io::Result<Held> {
    hold_free_in(dir, DEFAULT)
}

/// Holds the session `first`, its lock in `dir`, or when another Hof has
/// it open the first of `default`, `2`, `3` and on that none has: a plain
/// `hof` told which session is its own (`[session] name`).
pub fn hold_free_in(dir: &Path, first: &str) -> io::Result<Held> {
    let names = std::iter::once(first.to_owned())
        .chain(std::iter::once(DEFAULT.to_owned()))
        .chain((2..=MOST).map(|n| n.to_string()));
    for name in names {
        if let Some(held) = hold_in(dir, &name)? {
            return Ok(held);
        }
    }
    Err(io::Error::other(format!(
        "{MOST} Hofs keep a session already"
    )))
}

/// Whether a Hof has the session `name` open, its lock in `dir`.
pub fn is_open_in(dir: &Path, name: &str) -> bool {
    matches!(hold_in(dir, name), Ok(None))
}

/// The session saved last that no Hof has open: what `hof attach` opens
/// when it is given no name. `dir` has the locks; `default` is where the
/// default session is saved.
pub fn last_free_in(dir: &Path, default: &Path) -> Option<String> {
    crate::saved_beside(default)
        .into_iter()
        .filter(|(name, _)| !is_open_in(dir, name))
        .max_by_key(|(_, file)| saved_at(file))
        .map(|(name, _)| name)
}

/// A saved session, as `hof sessions` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub name: String,
    /// A Hof has it open.
    pub open: bool,
    /// Its tabs and panes; `None` when it could not be read.
    pub held: Option<(usize, usize)>,
    pub saved: SystemTime,
}

/// The sessions saved beside `default`, their locks in `dir`: `default`
/// first, then the rest by name.
pub fn list_in(dir: &Path, default: &Path) -> Vec<Listed> {
    crate::saved_beside(default)
        .into_iter()
        .map(|(name, file)| Listed {
            open: is_open_in(dir, &name),
            held: crate::load(&file).ok().flatten().map(|s| {
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

fn lock_name(name: &str) -> String {
    if name == DEFAULT {
        "hof-session.lock".to_owned()
    } else {
        format!("hof-session-{name}.lock")
    }
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
        assert!(is_open_in(dir.path(), "work"));
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
    fn a_plain_hof_told_its_session_keeps_it_or_the_next_free() {
        let dir = tempfile::tempdir().unwrap();
        let work = hold_free_in(dir.path(), "work").unwrap();
        let beside = hold_free_in(dir.path(), "work").unwrap();
        assert_eq!([&work.name, &beside.name], ["work", "default"]);
        assert!(hold_free_in(dir.path(), "../up").is_err(), "no such name");
    }

    #[test]
    fn a_name_no_session_can_have_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let e = hold_in(dir.path(), "../up").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(!dir.path().join("hof-session-../up.lock").exists());
    }
}
