//! The dropdown's socket: another `hof` asks it to show or hide, or to open
//! a tab. A thread waits in `accept`, so a hidden dropdown costs nothing
//! until it is asked.
//!
//! A request is its fields separated by NUL, the first the verb, sent on a
//! connection of its own and ended by closing it.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
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

/// Where the dropdown listens: the command line's, else in the user's
/// runtime directory.
pub fn socket_path() -> PathBuf {
    if let Some(p) = &crate::launch().socket {
        return p.clone();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("hof-dropdown.sock")
}

/// Sends `request` to the dropdown at `path`. `Err` when none listens.
pub fn send(path: &Path, request: &Request) -> io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(&request.encode())?;
    stream.shutdown(std::net::Shutdown::Write)
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            // A socket left by a dropdown that died; one that answers is
            // alive, and keeps it.
            if UnixStream::connect(path).is_ok() {
                return Err(e);
            }
            std::fs::remove_file(path)?;
            UnixListener::bind(path)
        }
        other => other,
    }
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
