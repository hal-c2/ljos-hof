//! An X server for remote panes where this machine has none, as on a Mac
//! without XQuartz: [yserver](https://github.com/joske/yserver), started
//! rootless the first time a program there opens a window, so that each X
//! window is a window of its own here.
//!
//! Hof makes up a cookie and gives it to yserver in an authorization file
//! of its own, so only Hof's connections are served. yserver ends when
//! its last window closes, and is started again for the next.

use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::client::Service;
use crate::relay::Socket;
use crate::x11::{self, Auth, COOKIE, FAMILY_WILD, NotSetup, SERVICE, Setup};

/// The first display number tried, above those a desktop and `ssh -X`
/// take.
const FIRST_DISPLAY: u32 = 20;

/// How long yserver may take to listen.
const STARTING: Duration = Duration::from_secs(20);

/// yserver, to be started when a program first needs it.
#[derive(Debug)]
pub struct Yserver {
    program: PathBuf,
    running: Mutex<Option<Running>>,
}

#[derive(Debug)]
struct Running {
    child: Child,
    number: u32,
    cookie: [u8; 16],
    /// Holds the authorization file, removed with it.
    dir: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Yserver {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            running: Mutex::new(None),
        }
    }

    /// yserver, if it is here: `$HOF_YSERVER`, else, on a Mac, `yserver`
    /// on the `PATH` or the app in `/Applications` or `~/Applications`.
    pub fn find() -> Option<Self> {
        if let Some(program) = std::env::var_os("HOF_YSERVER").filter(|p| !p.is_empty()) {
            return Some(Self::new(program));
        }
        if !cfg!(target_os = "macos") {
            return None;
        }
        let on_path = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|dir| dir.join("yserver"));
        let app = "Applications/yserver.app/Contents/MacOS/yserver";
        let apps = [
            Some(Path::new("/").join(app)),
            std::env::var_os("HOME").map(|h| Path::new(&h).join(app)),
        ];
        on_path
            .chain(apps.into_iter().flatten())
            .find(|p| is_program(p))
            .map(Self::new)
    }

    /// The display yserver serves and its cookie, starting it if it is not
    /// running.
    fn display(&self) -> io::Result<(u32, [u8; 16])> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = running.as_mut()
            && matches!(r.child.try_wait(), Ok(None))
        {
            return Ok((r.number, r.cookie));
        }
        *running = None;
        let r = self.start()?;
        let display = (r.number, r.cookie);
        *running = Some(r);
        Ok(display)
    }

    fn start(&self) -> io::Result<Running> {
        let number = free_display()?;
        // One of this Hof's own for each start: two starting at once take
        // the same free number, and a directory left by a Hof that died is
        // not reused.
        static STARTS: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hof-yserver-{}-{}",
            std::process::id(),
            STARTS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
        let cookie = random_cookie()?;
        let mut file = Vec::new();
        Auth {
            family: FAMILY_WILD,
            address: Vec::new(),
            number: number.to_string().into_bytes(),
            name: COOKIE.to_vec(),
            data: cookie.to_vec(),
        }
        .encode(&mut file);
        let auth = dir.join("Xauthority");
        {
            use std::io::Write as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&auth)?
                .write_all(&file)?;
        }
        let child = Command::new(&self.program)
            .arg("--rootless")
            .arg(format!(":{number}"))
            .arg("-auth")
            .arg(&auth)
            .args(["-nolisten", "tcp"])
            // Ends when its last window does, and so with Hof, whose going
            // closes them; started again for the next.
            .arg("-terminate")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", self.program.display())))?;
        let mut running = Running {
            child,
            number,
            cookie,
            dir,
        };
        let began = Instant::now();
        loop {
            if UnixStream::connect(socket_path(number)).is_ok() {
                return Ok(running);
            }
            if let Ok(Some(status)) = running.child.try_wait() {
                return Err(io::Error::other(format!(
                    "{} stopped before it listened ({status})",
                    self.program.display()
                )));
            }
            if began.elapsed() > STARTING {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} did not listen on :{number}", self.program.display()),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Service for Yserver {
    fn name(&self) -> &str {
        SERVICE
    }

    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
        let not = || io::Error::new(io::ErrorKind::InvalidData, NotSetup);
        let (setup, len) = Setup::parse(opening).map_err(|_| not())?.ok_or_else(not)?;
        let connected = self
            .display()
            .and_then(|(number, cookie)| Ok((UnixStream::connect(socket_path(number))?, cookie)));
        match connected {
            Ok((socket, cookie)) => {
                let mut first = setup.with_cookie(Some(&cookie)).encode();
                first.extend_from_slice(&opening[len..]);
                Ok((Box::new(socket), first))
            }
            Err(e) => Ok(x11::refused(
                &setup,
                &format!("Hof: could not start yserver on {} ({e})", x11::host_name()),
            )),
        }
    }
}

fn socket_path(number: u32) -> String {
    format!("/tmp/.X11-unix/X{number}")
}

/// A display number no server here has.
fn free_display() -> io::Result<u32> {
    (FIRST_DISPLAY..FIRST_DISPLAY + 1000)
        .find(|n| {
            !Path::new(&format!("/tmp/.X{n}-lock")).exists()
                && !Path::new(&socket_path(*n)).exists()
        })
        .ok_or_else(|| io::Error::other("no display number free"))
}

fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn random_cookie() -> io::Result<[u8; 16]> {
    use std::io::Read as _;
    let mut cookie = [0; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut cookie)?;
    Ok(cookie)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn setup() -> Setup {
        Setup {
            big_endian: false,
            major: 11,
            minor: 0,
            auth_name: COOKIE.to_vec(),
            auth_data: b"the agent's".to_vec(),
        }
    }

    /// A stand-in for yserver: a script that listens on the display it is
    /// given and writes the authorization file it was given, and what the
    /// first client to send anything sent, into `out`.
    fn fake(dir: &Path, out: &Path) -> PathBuf {
        let script = dir.join("yserver");
        std::fs::write(
            &script,
            format!(
                r#"#!/usr/bin/env python3
import os, socket, sys, shutil
args = sys.argv[1:]
assert args[0] == "--rootless", args
n = args[1].lstrip(":")
auth = args[args.index("-auth") + 1]
path = "/tmp/.X11-unix/X" + n
os.makedirs("/tmp/.X11-unix", exist_ok=True)
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
try:
    shutil.copy(auth, "{out}/auth")
    data = b""
    while not data:
        # Hof knocks to see that it listens before it sends a client.
        c, _ = s.accept()
        while len(data) < 48:
            more = c.recv(64)
            if not more: break
            data += more
    open("{out}/setup", "wb").write(data)
    c.sendall(b"ok")
    c.close()
finally:
    os.unlink(path)
"#,
                out = out.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn yserver_is_started_with_a_cookie_of_ours_and_gets_it_in_the_setup() {
        let dir = tempfile::tempdir().unwrap();
        let server = Yserver::new(fake(dir.path(), dir.path()));
        let mut opening = setup().encode();
        opening.extend_from_slice(b"more");
        let (mut socket, first) = server.open(&opening).unwrap();
        socket.write_all(&first).unwrap();
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, b"ok");

        let auths = Auth::parse_all(&std::fs::read(dir.path().join("auth")).unwrap());
        assert_eq!(auths.len(), 1);
        let sent = std::fs::read(dir.path().join("setup")).unwrap();
        let (got, len) = Setup::parse(&sent).unwrap().unwrap();
        assert_eq!(got.auth_name, COOKIE);
        assert_eq!(
            got.auth_data, auths[0].data,
            "yserver's cookie, not the agent's"
        );
        assert_eq!(&sent[len..], b"more");
    }

    #[test]
    fn a_yserver_that_will_not_start_is_a_refusal_saying_why() {
        let server = Yserver::new("/nonexistent/yserver");
        let (mut socket, first) = server.open(&setup().encode()).unwrap();
        assert!(first.is_empty());
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).unwrap();
        assert_eq!(answer[0], 0, "failed");
        let reason = String::from_utf8_lossy(&answer[8..8 + usize::from(answer[1])]).into_owned();
        assert!(reason.contains("could not start yserver"), "{reason}");
        assert!(reason.contains("/nonexistent/yserver"), "{reason}");
    }
}
