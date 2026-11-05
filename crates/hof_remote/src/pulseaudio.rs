//! A sound server for remote panes where this machine has none running,
//! as on a Mac: PulseAudio, from Homebrew, started the first time a
//! program there plays or records, and playing through CoreAudio.
//!
//! Hof's PulseAudio is its own: it listens only on a socket in a
//! directory only this user can enter, keeps what it remembers there, and
//! ends a while after its last client goes, to be started again for the
//! next.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::client::Service;
use crate::program;
use crate::pulse::{NotOpening, SERVICE, packet_len, rewrite_auth};
use crate::relay::Socket;

/// How long PulseAudio may take to listen.
const STARTING: Duration = Duration::from_secs(20);

/// How long PulseAudio stays once its last client has gone, in seconds.
const IDLE: u32 = 30;

/// The module that finds this machine's speakers and microphones.
const DEVICES: &str = if cfg!(target_os = "macos") {
    "module-coreaudio-detect"
} else {
    "module-detect"
};

/// PulseAudio, to be started when a program first needs it.
#[derive(Debug)]
pub struct Pulseaudio {
    program: PathBuf,
    running: Mutex<Option<Running>>,
}

#[derive(Debug)]
struct Running {
    child: Child,
    /// Holds the socket and what PulseAudio keeps, removed with it.
    dir: PathBuf,
}

impl Running {
    fn socket(&self) -> PathBuf {
        self.dir.join("native")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Pulseaudio {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            running: Mutex::new(None),
        }
    }

    /// PulseAudio, if it is here: `$HOF_PULSEAUDIO`, else, on a Mac,
    /// `pulseaudio` on the `PATH` or where Homebrew puts it.
    pub fn find() -> Option<Self> {
        let brew = [
            PathBuf::from("/opt/homebrew/bin/pulseaudio"),
            PathBuf::from("/usr/local/bin/pulseaudio"),
        ];
        program::find("HOF_PULSEAUDIO", "pulseaudio", &brew).map(Self::new)
    }

    /// The socket PulseAudio listens on, starting it if it is not running.
    fn socket(&self) -> io::Result<PathBuf> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = running.as_mut()
            && matches!(r.child.try_wait(), Ok(None))
        {
            return Ok(r.socket());
        }
        *running = None;
        let r = self.start()?;
        let socket = r.socket();
        *running = Some(r);
        Ok(socket)
    }

    fn start(&self) -> io::Result<Running> {
        let dir = program::private_dir("pulseaudio")?;
        let socket = dir.join("native");
        let child = Command::new(&self.program)
            // None of the user's configuration: only what is here.
            .arg("-n")
            .args(["--daemonize=no", "--system=no", "--use-pid-file=no"])
            .arg("--disable-shm=yes")
            .arg(format!("--exit-idle-time={IDLE}"))
            // Anyone who can reach the socket may play, and only this user
            // can reach it.
            .arg("-L")
            .arg(format!(
                "module-native-protocol-unix auth-anonymous=1 socket={}",
                socket.display()
            ))
            .args(["-L", DEVICES])
            // Somewhere to play to while there are no speakers.
            .args(["-L", "module-always-sink"])
            .env("PULSE_RUNTIME_PATH", &dir)
            .env("PULSE_STATE_PATH", &dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&dir);
                io::Error::new(e.kind(), format!("{}: {e}", self.program.display()))
            })?;
        let mut running = Running { child, dir };
        program::wait_to_listen(&mut running.child, &self.program, &socket, STARTING)?;
        Ok(running)
    }
}

impl Service for Pulseaudio {
    fn name(&self) -> &str {
        SERVICE
    }

    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
        let not = || io::Error::new(io::ErrorKind::InvalidData, NotOpening);
        let len = packet_len(opening).map_err(|_| not())?.ok_or_else(not)?;
        let mut first = opening.to_vec();
        // It asks for no cookie.
        rewrite_auth(&mut first[..len], None);
        let socket = self.socket()?;
        Ok((Box::new(UnixStream::connect(socket)?), first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulse::{COOKIE_LEN, DESCRIPTOR};
    use std::io::{Read as _, Write as _};
    use std::path::Path;

    /// The start of a client's authorization: the command, a tag, and its
    /// version, offering to share memory.
    fn opening() -> Vec<u8> {
        let mut payload = vec![b'L'];
        payload.extend(8u32.to_be_bytes());
        payload.push(b'L');
        payload.extend(1u32.to_be_bytes());
        payload.push(b'L');
        payload.extend((35u32 | 0xc000_0000).to_be_bytes());
        payload.push(b'x');
        payload.extend((COOKIE_LEN as u32).to_be_bytes());
        payload.extend([1; COOKIE_LEN]);
        let mut packet = (payload.len() as u32).to_be_bytes().to_vec();
        packet.extend(u32::MAX.to_be_bytes());
        packet.extend([0; 12]);
        packet.extend(payload);
        packet
    }

    /// A stand-in for PulseAudio: a script that listens on the socket its
    /// arguments name and writes them, and what the first client to send
    /// anything sent, into `out`.
    fn fake(dir: &Path, out: &Path) -> PathBuf {
        let script = dir.join("pulseaudio");
        std::fs::write(
            &script,
            format!(
                r#"#!/usr/bin/env python3
import socket, sys
args = sys.argv[1:]
open("{out}/args", "w").write("\n".join(args))
native = next(a for a in args if a.startswith("module-native-protocol-unix"))
path = next(w for w in native.split() if w.startswith("socket=")).split("=", 1)[1]
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
data = b""
while not data:
    # Hof knocks to see that it listens before it sends a client.
    c, _ = s.accept()
    while len(data) < 20:
        more = c.recv(4096)
        if not more: break
        data += more
open("{out}/sent", "wb").write(data)
c.sendall(b"ok")
c.close()
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
    fn pulseaudio_is_started_on_a_socket_of_its_own_and_gets_the_client() {
        let dir = tempfile::tempdir().unwrap();
        let server = Pulseaudio::new(fake(dir.path(), dir.path()));
        let (mut socket, first) = server.open(&opening()).unwrap();
        socket.write_all(&first).unwrap();
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, b"ok");

        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert!(args.lines().any(|a| a == "-n"), "{args}");
        assert!(args.contains("auth-anonymous=1"), "{args}");
        assert!(args.lines().any(|a| a == DEVICES), "{args}");
        let sent = std::fs::read(dir.path().join("sent")).unwrap();
        let version =
            u32::from_be_bytes(sent[DESCRIPTOR + 11..DESCRIPTOR + 15].try_into().unwrap());
        assert_eq!(version, 35, "no memory shared");
    }

    #[test]
    fn a_pulseaudio_that_will_not_start_is_an_error_naming_it() {
        let server = Pulseaudio::new("/nonexistent/pulseaudio");
        let e = server.open(&opening()).err().expect("an error");
        assert!(e.to_string().contains("/nonexistent/pulseaudio"), "{e}");
    }
}
