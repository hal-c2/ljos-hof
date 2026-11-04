//! Hof's Model Context Protocol server: AI agents list panes, read their
//! screens and scrollback, type into them, open tabs and splits, and run a
//! command and wait for it to finish.
//!
//! The crate knows nothing of the UI. The app implements [`Host`], whose
//! methods are plain blocking calls the app answers from its UI thread,
//! and starts [`serve_unix`] with it. Agents start `hof mcp`, which runs
//! [`bridge_stdio`] to join their stdio to that socket, because stdio is
//! the transport every MCP client speaks.
//!
//! [`handle`] is the whole protocol as one function from a request line to
//! a reply line, so it can be driven without a socket.

#![forbid(unsafe_code)]

mod bridge;
mod client;
mod host;
mod protocol;
mod server;
mod tools;
mod unix;

use std::ffi::OsString;
use std::path::PathBuf;

pub use bridge::{bridge, bridge_stdio};
pub use client::{Answer, call};
pub use host::{
    AgentState, AgentWait, CommandResult, Direction, Host, HostError, HostResult, PaneId, PaneInfo,
};
pub use protocol::{PROTOCOL_VERSION, handle};
pub use server::{Server, serve_unix};
pub use tools::Policy;
pub use unix::same_user as peer_is_same_user;
pub use unix::{connect as connect_unix, prepare_parent as prepare_socket_parent};

/// Where the app listens unless configured otherwise:
/// `$XDG_RUNTIME_DIR/hof-mcp.sock`, else `mcp.sock` in a private directory
/// for this user in the temporary directory.
pub fn socket_path() -> PathBuf {
    socket_path_in(
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::temp_dir(),
        rustix::process::getuid().as_raw(),
    )
}

fn socket_path_in(runtime_dir: Option<OsString>, temp_dir: PathBuf, uid: u32) -> PathBuf {
    match runtime_dir.map(PathBuf::from) {
        // The spec says to ignore a relative one.
        Some(dir) if dir.is_absolute() => dir.join("hof-mcp.sock"),
        _ => unix::fallback_dir(&temp_dir, uid).join("mcp.sock"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_lives_in_the_runtime_directory_when_there_is_one() {
        let path = socket_path_in(Some("/run/user/1000".into()), "/tmp".into(), 1000);
        assert_eq!(path, PathBuf::from("/run/user/1000/hof-mcp.sock"));
    }

    #[test]
    fn without_a_runtime_directory_the_socket_is_in_a_private_user_directory() {
        assert_eq!(
            socket_path_in(None, "/tmp".into(), 1000),
            PathBuf::from("/tmp/hof-1000/mcp.sock")
        );
        assert_eq!(
            socket_path_in(Some("relative".into()), "/tmp".into(), 7),
            PathBuf::from("/tmp/hof-7/mcp.sock")
        );
    }
}
