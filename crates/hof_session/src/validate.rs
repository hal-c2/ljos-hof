//! Making a read session one the app can use as it is.

use std::collections::HashSet;
use std::path::Path;

use serde::Serializer;

use crate::{Error, MIN_RATIO, SavedNode, SavedTab, Session};

/// A ratio either half of a split can live with. JSON has no NaN, so one
/// that slipped into the layout becomes an even split rather than a file
/// that can't be read back.
pub fn clamp_ratio(ratio: f64) -> f64 {
    if ratio.is_nan() {
        0.5
    } else {
        ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO)
    }
}

pub fn serialize_ratio<S: Serializer>(ratio: &f64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(clamp_ratio(*ratio))
}

/// Refuses a session whose panes can't be told apart. Ids need only be
/// unique within a window, as each window has its own layout.
pub fn check_ids(session: &Session, path: &Path) -> Result<(), Error> {
    for (window, w) in session.windows.iter().enumerate() {
        let mut seen = HashSet::new();
        for pane in w.tabs.iter().flat_map(SavedTab::panes) {
            if !seen.insert(pane.id) {
                return Err(Error::DuplicatePane {
                    path: path.to_owned(),
                    window,
                    id: pane.id,
                });
            }
        }
    }
    Ok(())
}

/// Clamps what is out of range and drops what is empty.
pub fn repair(session: &mut Session) {
    session.windows.retain(|w| !w.tabs.is_empty());
    for window in &mut session.windows {
        window.current_tab = window.current_tab.min(window.tabs.len() - 1);
        for tab in &mut window.tabs {
            clamp_ratios(&mut tab.root);
            let panes = tab.panes();
            if !panes.iter().any(|p| p.id == tab.focused) {
                tab.focused = panes[0].id;
            }
        }
    }
}

fn clamp_ratios(node: &mut SavedNode) {
    if let SavedNode::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        *ratio = clamp_ratio(*ratio);
        clamp_ratios(first);
        clamp_ratios(second);
    }
}
