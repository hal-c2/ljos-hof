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

use hof_remote::peer::same_user;

use crate::Agent;

/// How long the bridge waits for an agent it started to listen.
const START_WAIT: Duration = Duration::from_secs(5);

/// Where the agent listens: `$HOF_AGENT_SOCKET`, else
/// `$XDG_RUNTIME_DIR/hof-agent.sock`, else `agent.sock` in a private
/// directory for this user in the temporary directory.
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
        _ => temp_dir.join(format!("hof-agent-{uid}")).join("agent.sock"),
    }
}

fn prepare_parent(path: &Path) -> io::Result<()> {
    let uid = rustix::process::getuid().as_raw();
    let dir = std::env::temp_dir().join(format!("hof-agent-{uid}"));
    if path.parent() == Some(dir.as_path()) {
        ensure_private_dir(&dir, uid)?;
    }
    Ok(())
}

fn ensure_private_dir(path: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match std::fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must be a directory owned by this user and private to it",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    same_user(&stream)?;
    Ok(stream)
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
                if same_user(&stream).is_err() {
                    continue;
                }
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
    prepare_parent(path)?;
    match connect(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("an agent already listens at {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
        Err(_) => {}
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
///
/// One that leads a session already (started so by systemd, say) has none
/// to leave.
pub fn detach() -> io::Result<()> {
    match rustix::process::setsid() {
        Ok(_) => Ok(()),
        Err(_) if rustix::process::getsid(None).ok() == Some(rustix::process::getpid()) => Ok(()),
        Err(e) => Err(e.into()),
    }
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

/// [`start`], for the agent that runs this machine's own panes, which is
/// to outlive the Hof that starts it: where systemd runs the user's
/// session, in a scope of its own (`systemd-run --user --scope`), so that
/// what ends the app's (closing it, as a desktop may do) leaves the agent
/// be. It keeps Hof's environment either way. Waits until it listens, or
/// until it is clear it won't.
pub fn start_apart(program: &Path, path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(started) = start_scoped(program, path) {
        return started;
    }
    start(program, path)
}

/// `None` when there is no systemd to ask, or it would not: then the agent
/// is started as any other.
#[cfg(target_os = "linux")]
fn start_scoped(program: &Path, path: &Path) -> Option<io::Result<()>> {
    let unit = format!("hof-agent-{}-{}", std::process::id(), instance_hint());
    let mut child = Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
        .arg(&unit)
        .arg("--")
        .arg(program)
        .args(["agent", "--detach", "--socket"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        if connect(path).is_ok() {
            // `systemd-run --scope` became the agent: reaped when it goes.
            let _ = thread::Builder::new()
                .name("hof-agent-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                });
            return Some(Ok(()));
        }
        match child.try_wait() {
            // Ended at once: another agent listens already, or systemd
            // would not, and then the agent is started without it.
            Ok(Some(_)) => return connect(path).is_ok().then_some(Ok(())),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = thread::Builder::new()
                    .name("hof-agent-reap".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return Some(Ok(()));
            }
        }
    }
}

/// Tells two starts by one process apart, for the scope's name.
#[cfg(target_os = "linux")]
fn instance_hint() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    let socket = match connect(path) {
        Ok(socket) => socket,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            start()?;
            wait_for(path)?
        }
        Err(error) => return Err(error),
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
        match connect(path) {
            Ok(socket) => return Ok(socket),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
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
            Path::new("/tmp/hof-agent-7/agent.sock")
        );
    }

    #[test]
    fn a_fallback_directory_is_private_and_cannot_be_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("agent");
        let uid = rustix::process::getuid().as_raw();
        ensure_private_dir(&dir, uid).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        ensure_private_dir(&dir, uid).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(temp.path(), &dir).unwrap();
        assert!(ensure_private_dir(&dir, uid).is_err());
    }
}
