//! The session, `[session] restore`: the window's tabs and splits, and each
//! pane's directory, program and scrollback, saved when the window closes
//! and opened again at the next start.
//!
//! The layout comes from the `Workspace`; what only the panes know (where
//! their shell is, what they show) comes from the QML as JSON, since it
//! reads it off each `Terminal`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hof_qml::layout::{Axis, Layout, Node, PaneId};
use hof_session::{SavedAxis, SavedNode, SavedPane, SavedTab, SavedWindow, Session};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("what the panes said was not JSON: {0}")]
    Panes(#[from] serde_json::Error),
    #[error(transparent)]
    Store(#[from] hof_session::Error),
}

/// Where this run keeps its session: the launch's, else its name's, else
/// the usual place.
pub(crate) fn path() -> PathBuf {
    let launch = crate::launch();
    match (&launch.session, &launch.session_name) {
        (Some(at), _) => at.clone(),
        (None, Some(name)) => hof_session::named(name),
        (None, None) => hof_session::path(),
    }
}

/// What the QML says of a pane, by its id: what the layout does not know.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PaneState {
    directory: String,
    command: Vec<String>,
    title: String,
    /// The terminal's `snapshot()`; empty to keep none.
    scrollback: String,
    /// The `[hosts]` name it runs on; empty for this machine.
    host: String,
    /// Its pane number under `hof agent` there.
    remote: Option<u32>,
}

/// What the QML needs to start a restored pane, as `Pane.start` takes it.
#[derive(Debug, Default, PartialEq, Serialize)]
pub(crate) struct PaneStart {
    pub directory: String,
    pub command: Vec<String>,
    /// Drawn before the program starts: the scrollback it had.
    pub replay: String,
    /// The `[hosts]` name to run it on; empty for this machine.
    pub host: String,
    /// The pane under `hof agent` there to pick up again, rather than
    /// starting the program anew.
    pub remote: Option<u32>,
}

/// A saved window, opened: the size to take and how to start each pane.
#[derive(Debug, Serialize)]
pub(crate) struct Restored {
    pub width: u32,
    pub height: u32,
    /// By pane id, as text: the keys of a JSON object.
    pub panes: BTreeMap<String, PaneStart>,
}

/// Saves `layout` with what `panes` (JSON, pane id to [`PaneState`]) says
/// of each pane, and the window's size.
pub(crate) fn save(
    at: &Path,
    layout: &Layout,
    panes: &str,
    width: f64,
    height: f64,
) -> Result<(), Error> {
    let panes: BTreeMap<String, PaneState> = serde_json::from_str(panes)?;
    let window = saved_window(layout, panes, width, height);
    hof_session::save(at, &Session::new(vec![window]))?;
    Ok(())
}

/// The saved session at `at`, opened as a layout, if there is one with a
/// window to open.
pub(crate) fn load(at: &Path) -> Result<Option<(Layout, Restored)>, Error> {
    let Some(session) = hof_session::load(at)? else {
        return Ok(None);
    };
    // One window is saved; a dropdown's would be its own run's.
    Ok(session
        .windows
        .into_iter()
        .find(|w| !w.dropdown && !w.tabs.is_empty())
        .map(opened))
}

/// Forgets the saved session: the user closed everything, so the next
/// start is a fresh one.
pub(crate) fn clear(at: &Path) -> Result<(), Error> {
    let launch = crate::launch();
    match &launch.session_name {
        // Its own directory goes too, and with it the name from `hof
        // sessions`.
        Some(name) if launch.session.is_none() => hof_session::forget(name)?,
        _ => hof_session::clear(at)?,
    }
    Ok(())
}

fn saved_window(
    layout: &Layout,
    mut panes: BTreeMap<String, PaneState>,
    width: f64,
    height: f64,
) -> SavedWindow {
    let size = |v: f64| {
        if v.is_finite() && v > 0.0 {
            v.round() as u32
        } else {
            0
        }
    };
    SavedWindow {
        tabs: layout
            .tabs()
            .iter()
            .map(|tab| SavedTab {
                root: saved_node(&tab.root, &mut panes),
                focused: tab.focused.0,
                zoomed: tab.zoomed,
            })
            .collect(),
        current_tab: layout.current(),
        width: size(width),
        height: size(height),
        dropdown: false,
    }
}

fn saved_node(node: &Node, panes: &mut BTreeMap<String, PaneState>) -> SavedNode {
    let some = |s: String| (!s.is_empty()).then_some(s);
    match node {
        Node::Pane(id) => {
            let state = panes.remove(&id.0.to_string()).unwrap_or_default();
            SavedNode::Pane(SavedPane {
                id: id.0,
                working_directory: some(state.directory).map(PathBuf::from),
                command: state.command,
                title: some(state.title),
                scrollback: some(state.scrollback),
                host: some(state.host),
                remote: state.remote,
            })
        }
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => SavedNode::Split {
            axis: match axis {
                Axis::Horizontal => SavedAxis::Horizontal,
                Axis::Vertical => SavedAxis::Vertical,
            },
            ratio: *ratio,
            first: Box::new(saved_node(first, panes)),
            second: Box::new(saved_node(second, panes)),
        },
    }
}

fn opened(window: SavedWindow) -> (Layout, Restored) {
    let mut panes = BTreeMap::new();
    let tabs = window
        .tabs
        .iter()
        .map(|tab| (node(&tab.root, &mut panes), PaneId(tab.focused)))
        .collect();
    let mut layout = Layout::restore(tabs, window.current_tab);
    for (index, tab) in window.tabs.iter().enumerate() {
        layout.set_zoom(index, tab.zoomed);
    }
    let restored = Restored {
        width: window.width,
        height: window.height,
        panes,
    };
    (layout, restored)
}

fn node(saved: &SavedNode, panes: &mut BTreeMap<String, PaneStart>) -> Node {
    match saved {
        SavedNode::Pane(p) => {
            panes.insert(
                p.id.to_string(),
                PaneStart {
                    directory: p
                        .working_directory
                        .as_ref()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default(),
                    command: p.command.clone(),
                    replay: p.scrollback.clone().unwrap_or_default(),
                    host: p.host.clone().unwrap_or_default(),
                    remote: p.remote,
                },
            );
            Node::Pane(PaneId(p.id))
        }
        SavedNode::Split {
            axis,
            ratio,
            first,
            second,
        } => Node::Split {
            axis: match axis {
                SavedAxis::Horizontal => Axis::Horizontal,
                SavedAxis::Vertical => Axis::Vertical,
            },
            ratio: *ratio,
            first: Box::new(node(first, panes)),
            second: Box::new(node(second, panes)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hof_qml::layout::Axis;

    fn two_tabs() -> Layout {
        let mut layout = Layout::default();
        layout.new_tab();
        layout.split(Axis::Horizontal);
        layout.new_tab();
        layout.split(Axis::Vertical);
        layout.toggle_zoom();
        layout
    }

    #[test]
    fn a_saved_layout_opens_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("session.json");
        let layout = two_tabs();
        let panes = r#"{"1": {"directory": "/tmp", "command": ["htop"], "title": "top",
                         "scrollback": "hello\r\n"},
                        "3": {"host": "dev", "remote": 7}}"#;
        save(&at, &layout, panes, 800.0, 400.0).unwrap();

        let (opened, restored) = load(&at).unwrap().expect("a session");
        let shape = |l: &Layout| -> Vec<_> {
            l.tabs()
                .iter()
                .map(|t| (t.root.clone(), t.focused, t.zoomed))
                .collect()
        };
        assert_eq!(shape(&opened), shape(&layout));
        assert_eq!(opened.current(), layout.current());
        assert_eq!((restored.width, restored.height), (800, 400));
        assert_eq!(restored.panes.len(), 4);
        assert_eq!(
            restored.panes["1"],
            PaneStart {
                directory: "/tmp".into(),
                command: vec!["htop".into()],
                replay: "hello\r\n".into(),
                ..PaneStart::default()
            }
        );
        assert_eq!(restored.panes["2"], PaneStart::default());
        assert_eq!(
            restored.panes["3"],
            PaneStart {
                host: "dev".into(),
                remote: Some(7),
                ..PaneStart::default()
            }
        );
    }

    #[test]
    fn nothing_saved_is_nothing_to_open() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("session.json")).unwrap().is_none());
    }

    #[test]
    fn a_cleared_session_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("session.json");
        save(&at, &two_tabs(), "{}", 0.0, 0.0).unwrap();
        clear(&at).unwrap();
        assert!(load(&at).unwrap().is_none());
    }

    #[test]
    fn what_the_panes_say_must_be_json() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("session.json");
        assert!(matches!(
            save(&at, &two_tabs(), "not json", 0.0, 0.0),
            Err(Error::Panes(_))
        ));
    }
}
