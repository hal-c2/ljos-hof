//! The dropdown's socket: another `hof` asks it to show or hide, or to open
//! a tab. A thread waits in `accept`, so a hidden dropdown costs nothing
//! until it is asked.
//!
//! A request is its fields separated by NUL, the first the verb, sent on a
//! connection of its own and ended by closing it.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use ljos::prelude::*;

use crate::App;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Toggle,
    Tab {
        directory: String,
        command: Vec<String>,
    },
}

impl Request {
    fn encode(&self) -> Vec<u8> {
        let fields: Vec<&str> = match self {
            Request::Toggle => vec!["toggle"],
            Request::Tab { directory, command } => {
                let mut f = vec!["tab", directory.as_str()];
                f.extend(command.iter().map(String::as_str));
                f
            }
        };
        fields.join("\0").into_bytes()
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut fields = text.split('\0');
        match fields.next()? {
            "toggle" => Some(Request::Toggle),
            "tab" => Some(Request::Tab {
                directory: fields.next().unwrap_or_default().to_owned(),
                command: fields.map(str::to_owned).collect(),
            }),
            _ => None,
        }
    }
}

/// Where the dropdown listens: the command line's, else beside the MCP
/// socket in the user's runtime directory or private temporary directory.
pub fn socket_path() -> PathBuf {
    if let Some(p) = &crate::launch().socket {
        return p.clone();
    }
    hof_mcp::socket_path().with_file_name("hof-dropdown.sock")
}

/// Sends `request` to the dropdown at `path`. `Err` when none listens.
pub fn send(path: &Path, request: &Request) -> io::Result<()> {
    let mut stream = hof_mcp::connect_unix(path)?;
    stream.write_all(&request.encode())?;
    stream.shutdown(std::net::Shutdown::Write)
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;
    hof_mcp::prepare_socket_parent(path)?;
    let listener = match UnixListener::bind(path) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            // A socket left by a dropdown that died; one that answers is
            // alive, and keeps it.
            if hof_mcp::connect_unix(path).is_ok() {
                return Err(e);
            }
            std::fs::remove_file(path)?;
            UnixListener::bind(path)?
        }
        other => other?,
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Listens for requests to the [`App`] singleton `me`.
pub(crate) fn listen(engine: &Engine, me: ObjectId) {
    let path = socket_path();
    let listener = match bind(&path) {
        Ok(l) => l,
        Err(e) => {
            engine.warn(format!("hof: cannot listen on {}: {e}", path.display()));
            return;
        }
    };
    let post = Platform::of(engine).poster();
    let spawned = std::thread::Builder::new()
        .name("hof-dropdown".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                if hof_mcp::peer_is_same_user(&stream).is_err() {
                    continue;
                }
                let mut bytes = Vec::new();
                if stream.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                let Some(request) = Request::decode(&bytes) else {
                    continue;
                };
                post.send(move |e: &Engine| {
                    let cx = Cx::new(e, me);
                    let _ = match request {
                        Request::Toggle => App::emit_toggle_requested(&cx),
                        Request::Tab { directory, command } => {
                            App::emit_tab_requested(&cx, command, directory)
                        }
                    };
                });
            }
        });
    if let Err(e) = spawned {
        engine.warn(format!("hof: cannot listen: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_socket_is_private_and_a_dead_one_is_replaced() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dropdown.sock");
        let listener = bind(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(listener);
        assert!(path.exists(), "a dead listener leaves its socket behind");
        let _live = bind(&path).unwrap();
    }

    #[test]
    fn requests_survive_the_wire() {
        for r in [
            Request::Toggle,
            Request::Tab {
                directory: "/tmp".into(),
                command: vec!["htop".into(), "-d".into(), "5".into()],
            },
            Request::Tab {
                directory: String::new(),
                command: Vec::new(),
            },
        ] {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
    }
}
