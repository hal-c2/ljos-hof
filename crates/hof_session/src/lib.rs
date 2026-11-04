//! Hof's session: the windows, their tabs and splits, and what ran in each
//! pane, saved when Hof quits and restored when it starts.
//!
//! These types mirror the app's layout without depending on it, so the
//! session can be read and written without the UI toolkit. The app converts
//! between them and its own layout.
//!
//! # The file format
//!
//! A session is one JSON file, by default `$XDG_STATE_HOME/hof/session.json`
//! (see [`path`]), with a `scrollback/` directory beside it:
//!
//! ```text
//! ~/.local/state/hof/            0700
//! ├── session.json               0600
//! └── scrollback/                0700
//!     ├── 0-1.vt                 0600  window 0, pane 1
//!     └── 1-4.vt                 0600  window 1, pane 4
//! ```
//!
//! The JSON holds everything but the scrollback, pretty-printed so it can
//! be read and fixed by hand:
//!
//! ```json
//! {
//!   "version": 1,
//!   "windows": [
//!     {
//!       "tabs": [
//!         {
//!           "root": {
//!             "split": {
//!               "axis": "horizontal",
//!               "ratio": 0.5,
//!               "first": { "pane": { "id": 1, "working_directory": "/home/me", "command": [], "title": null } },
//!               "second": { "pane": { "id": 2, "working_directory": "/tmp", "command": ["htop"], "title": "htop" } }
//!             }
//!           },
//!           "focused": 2,
//!           "zoomed": false
//!         }
//!       ],
//!       "current_tab": 0,
//!       "width": 1200,
//!       "height": 800,
//!       "dropdown": false
//!     }
//!   ]
//! }
//! ```
//!
//! - A tab's `root` is a tree. A leaf is `{"pane": {...}}`; a split is
//!   `{"split": {...}}` with an `axis` of `horizontal` (side by side, first
//!   on the left) or `vertical` (stacked, first on top) and the first
//!   half's `ratio`. Because every leaf is a pane, a tab always has one.
//! - `working_directory` is a string, or `{"bytes": [...]}` for a path that
//!   isn't UTF-8. `command` is the program and its arguments; empty means
//!   the user's shell.
//! - A pane's scrollback, if it was saved, is `scrollback/<window>-<pane>.vt`:
//!   the terminal's snapshot as written, VT escapes and all. It is kept out
//!   of the JSON because it can be large and is not meant to be read. It is
//!   found by name, so the JSON says nothing about it.
//! - Missing fields take their defaults and unknown fields are ignored, so
//!   older and newer files of the same `version` both read. A larger
//!   `version` is refused rather than half-read.
//!
//! The files are private to the user because scrollback can hold anything
//! that was on screen, passwords and tokens included.
//!
//! # Reading
//!
//! [`load`] repairs what it can and refuses the rest: ratios are clamped to
//! `0.05..=0.95`, `current_tab` and `focused` to a tab and pane that exist,
//! and windows without tabs are dropped. A pane id that appears twice in a
//! window is an error, since the app could not tell the panes apart.

mod path_repr;
mod store;
mod validate;

use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use store::{clear, load, save};

/// The newest format this crate writes and reads.
pub const VERSION: u32 = 1;

/// The smallest share a half of a split keeps; the largest is its
/// complement.
pub const MIN_RATIO: f64 = 0.05;

/// Everything Hof restores.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Session {
    /// The format, for refusing files from a newer Hof. [`save`] writes
    /// [`VERSION`] whatever this says.
    pub version: u32,
    #[serde(default)]
    pub windows: Vec<SavedWindow>,
}

impl Session {
    pub fn new(windows: Vec<SavedWindow>) -> Self {
        Self {
            version: VERSION,
            windows,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SavedWindow {
    pub tabs: Vec<SavedTab>,
    /// An index into `tabs`.
    #[serde(default)]
    pub current_tab: usize,
    /// The window's size in logical pixels. A dropdown's size comes from the
    /// configuration, so it may ignore these.
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// The window drops down from the screen's edge rather than being an
    /// ordinary window.
    #[serde(default)]
    pub dropdown: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SavedTab {
    pub root: SavedNode,
    /// The id of the focused pane, one of the tree's.
    pub focused: u32,
    /// The focused pane fills the tab.
    #[serde(default)]
    pub zoomed: bool,
}

impl SavedTab {
    /// The tab's panes, in the tree's order: left to right, top to bottom.
    pub fn panes(&self) -> Vec<&SavedPane> {
        let mut out = Vec::new();
        self.root.panes(&mut out);
        out
    }
}

/// A tab's tree of splits, its leaves the panes.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedNode {
    Pane(SavedPane),
    Split {
        axis: SavedAxis,
        /// The first half's share of the split.
        #[serde(serialize_with = "validate::serialize_ratio")]
        ratio: f64,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

impl SavedNode {
    fn panes<'a>(&'a self, out: &mut Vec<&'a SavedPane>) {
        match self {
            SavedNode::Pane(p) => out.push(p),
            SavedNode::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn panes_mut<'a>(&'a mut self, out: &mut Vec<&'a mut SavedPane>) {
        match self {
            SavedNode::Pane(p) => out.push(p),
            SavedNode::Split { first, second, .. } => {
                first.panes_mut(out);
                second.panes_mut(out);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedAxis {
    /// Side by side: the first half on the left.
    Horizontal,
    /// Stacked: the first half on top.
    Vertical,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SavedPane {
    /// The layout's pane id, unique within its window.
    pub id: u32,
    /// Where the pane's program was, if known; `None` starts it in the
    /// default directory.
    #[serde(default, with = "path_repr")]
    pub working_directory: Option<PathBuf>,
    /// The program and its arguments; empty starts the user's shell.
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// The terminal's snapshot, VT escapes and all. It is stored in its own
    /// file, not the JSON, so serde skips it: only [`save`] and [`load`]
    /// carry it.
    #[serde(skip)]
    pub scrollback: Option<String>,
    /// The machine the pane runs on, a name in `[hosts]`; `None` is this
    /// one.
    #[serde(default)]
    pub host: Option<String>,
    /// The pane's number under `hof agent` on its host. Its program
    /// outlives Hof there, so restoring picks it up again rather than
    /// starting a new one.
    #[serde(default)]
    pub remote: Option<u32>,
}

/// Where the session is kept: `$XDG_STATE_HOME/hof/session.json`, else
/// `~/.local/state/hof/session.json`.
pub fn path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
        .filter(|h| !h.as_os_str().is_empty());
    path_from(std::env::var_os("XDG_STATE_HOME").as_deref(), home)
}

/// [`path`], given the environment.
fn path_from(xdg_state_home: Option<&OsStr>, home: Option<PathBuf>) -> PathBuf {
    // The base directory spec says a relative XDG path is to be ignored, as
    // an empty one is: it would land the session wherever Hof was started.
    let state = xdg_state_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/state")))
        // With no home at all there is nowhere right, so the session goes
        // beside wherever Hof runs rather than nowhere.
        .unwrap_or_else(|| PathBuf::from(".local/state"));
    state.join("hof").join("session.json")
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: not a Hof session: {source}", path.display())]
    Corrupt {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(
        "{}: written by a newer Hof (session version {found}; this one reads up to {VERSION})",
        path.display()
    )]
    FutureVersion { path: PathBuf, found: u32 },
    #[error("{}: pane {id} appears twice in window {window}", path.display())]
    DuplicatePane {
        path: PathBuf,
        window: usize,
        id: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Option<PathBuf> {
        Some(PathBuf::from("/home/me"))
    }

    #[test]
    fn xdg_state_home_is_where_the_session_goes() {
        assert_eq!(
            path_from(Some(OsStr::new("/state")), home()),
            PathBuf::from("/state/hof/session.json")
        );
    }

    #[test]
    fn without_xdg_state_home_the_session_goes_under_home() {
        assert_eq!(
            path_from(None, home()),
            PathBuf::from("/home/me/.local/state/hof/session.json")
        );
    }

    #[test]
    fn an_empty_xdg_state_home_counts_as_unset() {
        assert_eq!(
            path_from(Some(OsStr::new("")), home()),
            PathBuf::from("/home/me/.local/state/hof/session.json")
        );
    }

    #[test]
    fn a_relative_xdg_state_home_is_ignored() {
        assert_eq!(
            path_from(Some(OsStr::new("state")), home()),
            PathBuf::from("/home/me/.local/state/hof/session.json")
        );
    }

    #[test]
    fn with_no_home_at_all_the_session_still_has_a_path() {
        assert_eq!(
            path_from(None, None),
            PathBuf::from(".local/state/hof/session.json")
        );
    }
}
