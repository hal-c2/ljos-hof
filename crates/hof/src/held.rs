//! Which session a window keeps, and that no other Hof keeps it as well:
//! two windows of one session would save over each other, and attach to
//! the same programs.
//!
//! A window holds its session by a lock on `hof-session[-NAME].lock` in the
//! runtime directory for as long as it runs. The system lets it go when the
//! process goes, however it goes, so no lock is ever left behind.

use std::io;
use std::path::{Path, PathBuf};

use hof_session::{DEFAULT, runtime_dir};
pub use hof_session::{Held, Listed, saved_at};

/// Holds the session `name`, or `None` when another Hof does.
pub fn hold(name: &str) -> io::Result<Option<Held>> {
    hof_session::hold_in(&runtime_dir(), name)
}

/// Holds a plain `hof`'s session: `first` (`default`, or `[session]
/// name`), or when another Hof has it open the first of `default`, `2`,
/// `3` and on that none has. A window opened beside another so keeps its
/// own, rather than the other's.
pub fn hold_free(first: &str) -> io::Result<Held> {
    hof_session::hold_free_in(&runtime_dir(), first)
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
    hof_session::last_free_in(&runtime_dir(), &hof_session::path())
}

/// The sessions saved: `default` first, then the rest by name.
pub fn list() -> Vec<Listed> {
    hof_session::list_in(&runtime_dir(), &hof_session::path())
}

/// How long `hof forget` waits for the agents to hear it.
const FORGET_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// `hof forget NAME`: ends the programs the session `name` kept running,
/// on this machine (the agent at `agent`, not started for it) and on the
/// config's hosts (`config`), then forgets the session. Not one a Hof has
/// open: that one's panes are closed there. Returns how many programs it
/// ended.
pub fn forget(name: &str, config: &Path, agent: &Path) -> anyhow::Result<usize> {
    use anyhow::Context as _;
    let _held = hold(name)?.with_context(|| {
        format!("another Hof has the session {name} open: close its panes there")
    })?;
    let file = hof_session::named(name);
    let session =
        hof_session::load(&file)?.with_context(|| format!("no session {name} is saved"))?;
    if let Ok(loaded) = hof_config::load(config) {
        hof_qml::remote::set_hosts(crate::settings::transports(&loaded.config));
    }
    let ended = end_programs(&session, agent);
    hof_session::forget(name)?;
    Ok(ended)
}

/// Ends the programs `session`'s panes kept running: on this machine (the
/// agent at `agent`, not started for it) and on the hosts set. Returns how
/// many it ended.
pub(crate) fn end_programs(session: &hof_session::Session, agent: &Path) -> usize {
    let panes: Vec<(String, u32)> = session
        .windows
        .iter()
        .flat_map(|w| &w.tabs)
        .flat_map(|t| t.panes())
        .filter_map(|p| Some((p.host.clone().unwrap_or_default(), p.remote?)))
        .collect();
    if panes.is_empty() {
        return 0;
    }
    // An agent that is not running has nothing of the session's.
    hof_qml::remote::keep_here(Some(hof_qml::remote::Here {
        socket: agent.to_owned(),
        start: std::sync::Arc::new(|| Err(io::ErrorKind::NotFound.into())),
    }));
    hof_qml::remote::end_unseen(&panes, FORGET_WAIT)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_locks_are_beside_the_mcp_socket() {
        assert_eq!(
            Some(hof_session::runtime_dir().as_path()),
            hof_mcp::socket_path().parent()
        );
    }
}
