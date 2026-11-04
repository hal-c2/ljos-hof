//! The Unix socket the app listens on.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::host::Host;
use crate::protocol::{self, handle};
use crate::tools::Policy;

/// A message longer than this is not one a client meant to send, and
/// buffering it would let one connection eat the app's memory.
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// A running server. Dropping it stops it, like [`Server::shutdown`].
pub struct Server {
    path: PathBuf,
    /// The socket file's device and inode, so that stopping never removes
    /// a socket another Hof has since put at the same path.
    identity: (u64, u64),
    listener: UnixListener,
    stopping: Arc<AtomicBool>,
    connections: Arc<Mutex<Connections>>,
    accept_thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Connections {
    next: u64,
    open: HashMap<u64, UnixStream>,
}

/// Serve MCP on a Unix socket at `path`, readable and writable only by the
/// user, with one thread per connection.
///
/// A socket left at `path` by a Hof that did not exit cleanly is replaced;
/// one that still answers is not, and is an `AddrInUse` error. Anything at
/// `path` that is not a socket is an `AlreadyExists` error.
pub fn serve_unix(
    path: impl AsRef<Path>,
    host: Arc<dyn Host>,
    policy: Policy,
) -> io::Result<Server> {
    let path = path.as_ref().to_path_buf();
    crate::unix::prepare_parent(&path)?;
    clear_stale_socket(&path)?;
    let listener = bind_private(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    let identity = (metadata.dev(), metadata.ino());

    let stopping = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(Mutex::new(Connections::default()));
    let accept_thread = {
        let listener = listener.try_clone()?;
        let stopping = stopping.clone();
        let connections = connections.clone();
        thread::Builder::new()
            .name("hof-mcp-accept".into())
            .spawn(move || accept_loop(&listener, &host, policy, &stopping, &connections))?
    };
    Ok(Server {
        path,
        identity,
        listener,
        stopping,
        connections,
        accept_thread: Some(accept_thread),
    })
}

impl Server {
    /// Where the socket is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stop accepting, hang up on every client and remove the socket file.
    ///
    /// A connection waiting on the [`Host`] keeps its thread until the
    /// host answers, then finds its client gone.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(accept_thread) = self.accept_thread.take() else {
            return;
        };
        self.stopping.store(true, Ordering::SeqCst);
        // On Linux this wakes the blocked accept. Elsewhere it may not, so
        // a connection of our own does, as long as the path still leads to
        // our socket.
        let _ = rustix::net::shutdown(&self.listener, rustix::net::Shutdown::Both);
        if self.still_ours() {
            let _ = UnixStream::connect(&self.path);
        }
        {
            let mut connections = self
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for (_, stream) in connections.open.drain() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        let _ = accept_thread.join();
        if self.still_ours() {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn still_ours(&self) -> bool {
        fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.identity)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn clear_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a socket", path.display()),
        ));
    }
    match crate::unix::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("something is already serving on {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => fs::remove_file(path),
        Err(error) => Err(error),
    }
}

/// Bind under a temporary name and move the socket into place only once it
/// is private. A connection queued in between still has its peer checked.
fn bind_private(path: &Path) -> io::Result<UnixListener> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().unwrap_or(Path::new("."));
    // Kept short: socket paths have a small length limit.
    let temporary = directory.join(format!(
        ".hof-mcp.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_file(&temporary);
    let listener = UnixListener::bind(&temporary)?;
    let placed = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = placed {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(listener)
}

fn accept_loop(
    listener: &UnixListener,
    host: &Arc<dyn Host>,
    policy: Policy,
    stopping: &AtomicBool,
    connections: &Arc<Mutex<Connections>>,
) {
    loop {
        let accepted = listener.accept();
        if stopping.load(Ordering::SeqCst) {
            return;
        }
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(error) => {
                match error.kind() {
                    io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted => {}
                    // Out of descriptors, most likely: back off rather
                    // than spin until a connection closes.
                    _ => thread::sleep(Duration::from_millis(100)),
                }
                continue;
            }
        };
        if crate::unix::same_user(&stream).is_err() {
            continue;
        }
        let Some(id) = register(connections, &stream, stopping) else {
            return;
        };
        let host = host.clone();
        let connections_for_thread = connections.clone();
        let spawned = thread::Builder::new()
            .name("hof-mcp-connection".into())
            .spawn(move || {
                let _ = serve_connection(&stream, &*host, policy);
                connections_for_thread
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .open
                    .remove(&id);
            });
        if spawned.is_err() {
            connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .open
                .remove(&id);
        }
    }
}

/// Keep a handle on the connection so shutting down can hang up on it.
/// Checked under the lock, so a connection accepted as the server stops is
/// either hung up on or never served.
fn register(
    connections: &Mutex<Connections>,
    stream: &UnixStream,
    stopping: &AtomicBool,
) -> Option<u64> {
    let mut connections = connections.lock().unwrap_or_else(PoisonError::into_inner);
    if stopping.load(Ordering::SeqCst) {
        return None;
    }
    let handle = stream.try_clone().ok()?;
    let id = connections.next;
    connections.next += 1;
    connections.open.insert(id, handle);
    Some(id)
}

/// Requests on one connection are answered in order, one at a time, so
/// input an agent sends in two calls reaches the pane in that order.
fn serve_connection(stream: &UnixStream, host: &dyn Host, policy: Policy) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut writer = stream;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_MESSAGE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > MAX_MESSAGE {
            let mut reply = protocol::parse_error("message too long");
            reply.push('\n');
            writer.write_all(reply.as_bytes())?;
            return Ok(());
        }
        let reply = match std::str::from_utf8(&line) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => handle(host, policy, text.trim()),
            Err(_) => Some(protocol::parse_error("not UTF-8")),
        };
        if let Some(mut reply) = reply {
            reply.push('\n');
            writer.write_all(reply.as_bytes())?;
        }
    }
}
