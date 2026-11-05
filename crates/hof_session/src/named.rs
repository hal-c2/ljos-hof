//! Sessions by name (`hof attach work`): each its own file and scrollback,
//! so that two never share either.
//!
//! ```text
//! ~/.local/state/hof/
//! ├── session.json               "default", where it always was
//! ├── scrollback/
//! └── sessions/
//!     └── work/
//!         ├── session.json       "work"
//!         └── scrollback/
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;

/// The session a plain `hof` keeps when no other Hof has it.
pub const DEFAULT: &str = "default";

const SESSIONS_DIR: &str = "sessions";
const FILE: &str = "session.json";

/// Whether `name` can name a session: letters, digits, `.`, `_` and `-`,
/// not first a `.` or a `-`, and at most 64 of them. It is a directory's
/// name, and one a shell needs no quotes for.
pub fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Where the session `name` is kept: [`crate::path`] for [`DEFAULT`], else
/// `sessions/<name>/session.json` beside it.
pub fn named(name: &str) -> PathBuf {
    named_beside(&crate::path(), name)
}

/// [`named`], given where the default session is.
pub fn named_beside(default: &Path, name: &str) -> PathBuf {
    if name == DEFAULT {
        return default.to_owned();
    }
    state_dir(default).join(SESSIONS_DIR).join(name).join(FILE)
}

/// The sessions saved, by name, and where: [`DEFAULT`] first, then the
/// rest by name.
pub fn saved() -> Vec<(String, PathBuf)> {
    saved_beside(&crate::path())
}

/// [`saved`], given where the default session is.
pub fn saved_beside(default: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    if default.is_file() {
        found.push((DEFAULT.to_owned(), default.to_owned()));
    }
    let mut named = Vec::new();
    if let Ok(entries) = fs::read_dir(state_dir(default).join(SESSIONS_DIR)) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let file = entry.path().join(FILE);
            if is_name(&name) && name != DEFAULT && file.is_file() {
                named.push((name, file));
            }
        }
    }
    named.sort();
    found.extend(named);
    found
}

/// Forgets the session `name`, as [`crate::clear`] does, and the directory
/// a named one had of its own once nothing is left in it.
pub fn forget(name: &str) -> Result<(), Error> {
    forget_beside(&crate::path(), name)
}

/// [`forget`], given where the default session is.
pub fn forget_beside(default: &Path, name: &str) -> Result<(), Error> {
    let file = named_beside(default, name);
    crate::clear(&file)?;
    if name != DEFAULT
        && let Some(dir) = file.parent()
    {
        // Only an empty one, never more: whatever else is there stays.
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

fn state_dir(default: &Path) -> PathBuf {
    match default.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
        _ => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SavedNode, SavedPane, SavedTab, SavedWindow, Session};

    fn one_pane() -> Session {
        Session::new(vec![SavedWindow {
            tabs: vec![SavedTab {
                root: SavedNode::Pane(SavedPane {
                    id: 1,
                    scrollback: Some("seen\r\n".into()),
                    ..SavedPane::default()
                }),
                focused: 1,
                zoomed: false,
            }],
            current_tab: 0,
            width: 0,
            height: 0,
            dropdown: false,
        }])
    }

    #[test]
    fn a_name_is_what_a_directory_and_a_shell_take_plainly() {
        for good in ["default", "work", "2", "my-box.local", "a_b"] {
            assert!(is_name(good), "{good}");
        }
        let long = "x".repeat(65);
        for bad in ["", ".", "..", ".hidden", "-x", "a/b", "a b", "ö", &long] {
            assert!(!is_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_default_is_where_the_session_always_was_and_the_rest_apart() {
        let default = Path::new("/state/hof/session.json");
        assert_eq!(named_beside(default, DEFAULT), default);
        assert_eq!(
            named_beside(default, "work"),
            Path::new("/state/hof/sessions/work/session.json")
        );
    }

    #[test]
    fn sessions_saved_by_name_keep_their_own_scrollback() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("session.json");
        assert!(saved_beside(&default).is_empty());

        for name in ["work", DEFAULT, "2"] {
            crate::save(&named_beside(&default, name), &one_pane()).unwrap();
        }
        let names: Vec<_> = saved_beside(&default).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["default", "2", "work"]);

        // Saving one leaves another's scrollback be.
        crate::save(&named_beside(&default, "work"), &Session::new(vec![])).unwrap();
        let back = crate::load(&default).unwrap().unwrap();
        assert_eq!(
            back.windows[0].tabs[0].panes()[0].scrollback.as_deref(),
            Some("seen\r\n")
        );

        forget_beside(&default, "2").unwrap();
        assert!(
            !dir.path().join("sessions/2").exists(),
            "its directory goes"
        );
        forget_beside(&default, DEFAULT).unwrap();
        let names: Vec<_> = saved_beside(&default).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["work"]);
        assert!(crate::load(&named_beside(&default, "2")).unwrap().is_none());
    }
}
