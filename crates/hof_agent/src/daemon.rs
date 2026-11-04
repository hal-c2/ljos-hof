//! The agent as a process: on a unix socket only its user can reach, for as
//! long as it has panes or connections, and the bridge that ssh runs to
//! reach it.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::Agent;

/// How long the bridge waits for an agent it started to listen.
const START_WAIT: Duration = Duration::from_secs(5);

/// Where the agent listens: `$HOF_AGENT_SOCKET`, else
/// `$XDG_RUNTIME_DIR/hof-agent.sock`, else `hof-agent-<uid>.sock` in the
/// temporary directory.
pub fn socket_path() -> PathBuf {
    socket_path_in(
        std::env::var_os("HOF_AGENT_SOCKET"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::temp_dir(),
        rustix::process::getuid().as_raw(),
    )
}

fn socket_path_in(
    set: Option<OsString>,
    runtime_dir: Option<OsString>,
    temp_dir: PathBuf,
    uid: u32,
) -> PathBuf {
    if let Some(path) = set.filter(|p| !p.is_empty()) {
        return path.into();
    }
    match runtime_dir.map(PathBuf::from) {
        // The spec says to ignore a relative one.
        Some(dir) if dir.is_absolute() => dir.join("hof-agent.sock"),
        _ => temp_dir.join(format!("hof-agent-{uid}.sock")),
    }
}

/// How long an agent with nothing to do waits before it goes: long enough
/// for whoever started it to connect, and for a probe to come and go.
pub const LINGER: Duration = Duration::from_secs(10);

/// Listens at `path` and serves everyone who connects, until it has had no
/// connection and no pane for `linger`. Then it removes the socket and
/// returns.
///
/// Fails if another agent already listens there.
pub fn serve(agent: &Agent, path: &Path, linger: Duration) -> io::Result<()> {
    let listener = bind(path)?;
    let serving = agent.clone();
    // Not joined: it waits in `accept` until the process ends.
    thread::Builder::new()
        .name("hof-agent-accept".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let agent = serving.clone();
                let _ = thread::Builder::new()
                    .name("hof-agent-read".into())
                    .spawn(move || {
                        if let Ok(reader) = stream.try_clone() {
                            let _ = agent.serve(reader, stream);
                        }
                    });
            }
        })?;
    // Looked at now and then: how soon after `linger` it goes hardly
    // matters, and a connection that comes and goes between two looks
    // was never idleness.
    let every = (linger / 4).clamp(Duration::from_millis(10), Duration::from_secs(1));
    let mut idle_since = None;
    loop {
        thread::sleep(every);
        if !agent.is_idle() {
            idle_since = None;
            continue;
        }
        if idle_since.get_or_insert_with(Instant::now).elapsed() >= linger {
            // First, so a bridge that comes now starts a new agent rather
            // than reaching this one as it goes.
            let _ = std::fs::remove_file(path);
            agent.remove_files();
            return Ok(());
        }
    }
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    if UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("an agent already listens at {}", path.display()),
        ));
    }
    // Left by an agent that died.
    let _ = std::fs::remove_file(path);
    // Created for its user only: from the start, not after a chmod.
    let umask = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o177));
    let listener = UnixListener::bind(path);
    rustix::process::umask(umask);
    let listener = listener?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Leaves the session and terminal it was started from, so the agent lives
/// on when the ssh session that started it hangs up.
pub fn detach() -> io::Result<()> {
    rustix::process::setsid()?;
    Ok(())
}

/// Starts `program agent --detach --socket <path>` with no stdio of the
/// caller's, so ssh need not wait for it, and does not wait for it either.
///
/// It is the bridge's `start` when `program` is Hof itself.
pub fn start(program: &Path, path: &Path) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(["agent", "--detach", "--socket"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reaped if it goes first, so it is no zombie of a bridge that stays.
    thread::Builder::new()
        .name("hof-agent-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// `hof agent --stdio`: joins stdin and stdout to the agent at `path`,
/// starting one with `start` when none answers.
pub fn bridge_stdio(path: &Path, start: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    bridge(path, io::stdin(), io::stdout(), start)
}

/// [`bridge_stdio`] with any input and output. Returns when the agent
/// hangs up or `output` fails; the input ending is passed on as the end of
/// the connection.
pub fn bridge<R, W>(
    path: &Path,
    input: R,
    mut output: W,
    start: impl FnOnce() -> io::Result<()>,
) -> io::Result<()>
where
    R: Read + Send + 'static,
    W: Write,
{
    let socket = match UnixStream::connect(path) {
        Ok(socket) => socket,
        Err(_) => {
            start()?;
            wait_for(path)?
        }
    };
    let mut to_agent = socket.try_clone()?;
    // Not joined: when the agent goes first this thread is still blocked
    // reading the input, and the process ending is what ends it.
    thread::Builder::new()
        .name("hof-agent-stdin".into())
        .spawn(move || {
            let mut input = input;
            let _ = io::copy(&mut input, &mut to_agent);
            // The client is gone: so is this connection.
            let _ = to_agent.shutdown(Shutdown::Both);
        })?;
    let mut from_agent = &socket;
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = match from_agent.read(&mut buf) {
            Ok(0) => return output.flush(),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        output.write_all(&buf[..n])?;
        // Someone is typing at the other end of this.
        output.flush()?;
    }
}

fn wait_for(path: &Path) -> io::Result<UnixStream> {
    let deadline = Instant::now() + START_WAIT;
    loop {
        match UnixStream::connect(path) {
            Ok(socket) => return Ok(socket),
            Err(e) if Instant::now() >= deadline => {
                return Err(io::Error::new(
                    e.kind(),
                    format!(
                        "the agent started did not listen at {}: {e}",
                        path.display()
                    ),
                ));
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_the_one_asked_for_else_in_the_runtime_directory() {
        let path = |set: Option<&str>, runtime: Option<&str>| {
            socket_path_in(
                set.map(Into::into),
                runtime.map(Into::into),
                "/tmp".into(),
                7,
            )
        };
        assert_eq!(
            path(Some("/x.sock"), Some("/run/user/7")),
            Path::new("/x.sock")
        );
        assert_eq!(
            path(None, Some("/run/user/7")),
            Path::new("/run/user/7/hof-agent.sock")
        );
        assert_eq!(
            path(Some(""), Some("run")),
            Path::new("/tmp/hof-agent-7.sock")
        );
    }
}
