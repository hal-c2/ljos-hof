//! What an agent hands to the newer program it becomes, so that its panes'
//! programs run on through an update: a file it leaves open across `exec`,
//! read by the new program's `hof agent --resume FD`.
//!
//! The panes' ptys, the socket clients reach it at, and the X11 and sound
//! listeners stay open across `exec` too; the file says which descriptor
//! is which. The process stays the same one, so the programs stay its
//! children.
//!
//! The format is the agent's own, and only another `hof agent` of the same
//! [`RESUME_FORMAT`] reads it: the old one asks the new one first.

use std::io::{self, Read, Seek, Write};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::path::Path;

use hof_remote::wire::PaneId;

/// The version of what is handed over. A program that reads another
/// version is not exec'd into: the agent stays as it is.
pub const RESUME_FORMAT: u32 = 1;

const MAGIC: &[u8] = b"HOFRESUME";

/// The most a handover file may hold, against reading a wrong one.
const MOST: u64 = 1 << 30;

#[derive(Debug, PartialEq)]
pub(crate) struct State {
    pub instance: u64,
    pub next_pane: u32,
    /// Where clients connect.
    pub listener: RawFd,
    pub panes: Vec<PaneState>,
    pub x11: Option<X11State>,
    pub sound: Option<RawFd>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct PaneState {
    pub id: PaneId,
    pub command: Vec<String>,
    /// Its pty's master side and its program, while it may still run.
    pub running: Option<(RawFd, u32)>,
    /// How it ended, when it has.
    pub exited: Option<Option<i32>>,
    pub cols: u16,
    pub rows: u16,
    /// The terminal as it shows it, to start the new one from.
    pub screen: Vec<u8>,
    /// How much output it has written, all told.
    pub output: u64,
    /// How much of the client's input reached it.
    pub input: u64,
}

#[derive(Debug, PartialEq)]
pub(crate) struct X11State {
    pub listener: RawFd,
    pub number: u32,
    pub cookie: [u8; 16],
}

impl State {
    /// Every descriptor the new program is to have.
    pub fn descriptors(&self) -> Vec<RawFd> {
        let mut fds = vec![self.listener];
        fds.extend(self.panes.iter().filter_map(|p| p.running.map(|r| r.0)));
        fds.extend(self.x11.as_ref().map(|x| x.listener));
        fds.extend(self.sound);
        fds
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(MAGIC);
        w.u32(RESUME_FORMAT);
        w.u64(self.instance);
        w.u32(self.next_pane);
        w.fd(self.listener);
        w.u32(self.panes.len() as u32);
        for p in &self.panes {
            w.u32(p.id);
            w.u32(p.command.len() as u32);
            for arg in &p.command {
                w.bytes(arg.as_bytes());
            }
            match p.running {
                None => w.u8(0),
                Some((fd, pid)) => {
                    w.u8(1);
                    w.fd(fd);
                    w.u32(pid);
                }
            }
            match p.exited {
                None => w.u8(0),
                Some(None) => w.u8(1),
                Some(Some(code)) => {
                    w.u8(2);
                    w.u32(code as u32);
                }
            }
            w.u16(p.cols);
            w.u16(p.rows);
            w.bytes(&p.screen);
            w.u64(p.output);
            w.u64(p.input);
        }
        match &self.x11 {
            None => w.u8(0),
            Some(x) => {
                w.u8(1);
                w.fd(x.listener);
                w.u32(x.number);
                w.0.extend_from_slice(&x.cookie);
            }
        }
        match self.sound {
            None => w.u8(0),
            Some(fd) => {
                w.u8(1);
                w.fd(fd);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut r = Reader(bytes);
        if r.take(MAGIC.len())? != MAGIC {
            return Err(invalid("not a hof agent handover"));
        }
        let format = r.u32()?;
        if format != RESUME_FORMAT {
            return Err(invalid(&format!(
                "handed over in format {format}, where this reads {RESUME_FORMAT}"
            )));
        }
        let instance = r.u64()?;
        let next_pane = r.u32()?;
        let listener = r.fd()?;
        let count = r.u32()?;
        let mut panes = Vec::new();
        for _ in 0..count {
            let id = r.u32()?;
            let args = r.u32()?;
            let mut command = Vec::new();
            for _ in 0..args {
                command.push(String::from_utf8(r.bytes()?.to_vec()).map_err(io::Error::other)?);
            }
            let running = match r.u8()? {
                0 => None,
                _ => Some((r.fd()?, r.u32()?)),
            };
            let exited = match r.u8()? {
                0 => None,
                1 => Some(None),
                _ => Some(Some(r.u32()? as i32)),
            };
            panes.push(PaneState {
                id,
                command,
                running,
                exited,
                cols: r.u16()?,
                rows: r.u16()?,
                screen: r.bytes()?.to_vec(),
                output: r.u64()?,
                input: r.u64()?,
            });
        }
        let x11 = match r.u8()? {
            0 => None,
            _ => Some(X11State {
                listener: r.fd()?,
                number: r.u32()?,
                cookie: r.take(16)?.try_into().map_err(io::Error::other)?,
            }),
        };
        let sound = match r.u8()? {
            0 => None,
            _ => Some(r.fd()?),
        };
        if !r.0.is_empty() {
            return Err(invalid("more than a handover in the handover"));
        }
        Ok(Self {
            instance,
            next_pane,
            listener,
            panes,
            x11,
            sound,
        })
    }
}

/// A file holding `bytes`, read from its start, in `dir` but named by no
/// path: what the new program reads by its descriptor.
pub(crate) fn unnamed_file(dir: &Path, bytes: &[u8]) -> io::Result<OwnedFd> {
    use std::os::unix::fs::OpenOptionsExt;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    let path = dir.join(format!(
        ".hof-agent-handover-{}-{nanos}",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let _ = std::fs::remove_file(&path);
    file.write_all(bytes)?;
    file.rewind()?;
    Ok(file.into())
}

/// What the file at descriptor `fd`, handed over, says. Closes it.
pub(crate) fn read(fd: RawFd) -> io::Result<State> {
    if fd < 0 {
        return Err(invalid("no such descriptor"));
    }
    // SAFETY: the descriptor was left open for this, and nothing else in
    // this process has it.
    let file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    let mut bytes = Vec::new();
    file.take(MOST).read_to_end(&mut bytes)?;
    State::decode(&bytes)
}

/// Whether `exec` closes `fd`: so that the programs the agent starts get
/// none of these, and the new program gets each.
pub(crate) fn close_on_exec(fd: RawFd, close: bool) -> io::Result<()> {
    // SAFETY: only its flags are changed, and the descriptor stays open.
    let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
    let flags = if close {
        rustix::io::FdFlags::CLOEXEC
    } else {
        rustix::io::FdFlags::empty()
    };
    rustix::io::fcntl_setfd(fd, flags).map_err(io::Error::from)
}

/// Takes `fd`, handed over, as this program's own.
///
/// # Safety
///
/// `fd` must be one the handover named, taken once.
pub(crate) unsafe fn own(fd: RawFd) -> io::Result<OwnedFd> {
    close_on_exec(fd, true)?;
    // SAFETY: as the caller promises.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Whether `program` takes on what this one hands over: it says it reads
/// the same [`RESUME_FORMAT`].
pub(crate) fn takes_over(program: &Path) -> io::Result<()> {
    let output = std::process::Command::new(program)
        .args(["agent", "--resume-format"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", program.display())))?;
    let said = String::from_utf8_lossy(&output.stdout);
    match said.trim().parse::<u32>() {
        Ok(RESUME_FORMAT) if output.status.success() => Ok(()),
        Ok(other) => Err(io::Error::other(format!(
            "{} takes over format {other}, where this hands over {RESUME_FORMAT}",
            program.display()
        ))),
        Err(_) => Err(io::Error::other(format!(
            "{} cannot take over from a running agent",
            program.display()
        ))),
    }
}

/// Becomes `program`, which reads the handover at `file`, and listens at
/// `socket`, with `fds` left open for it. Returns only when it could
/// not, with `fds` as they were.
pub(crate) fn exec(program: &Path, file: RawFd, socket: &Path, fds: &[RawFd]) -> io::Error {
    use std::os::unix::process::CommandExt;
    let restore = |upto: usize| {
        for &fd in &fds[..upto] {
            let _ = close_on_exec(fd, true);
        }
    };
    for (n, &fd) in fds.iter().enumerate() {
        if let Err(e) = close_on_exec(fd, false) {
            restore(n);
            return e;
        }
    }
    eprintln!("hof agent: becoming {}", program.display());
    let error = std::process::Command::new(program)
        .arg("agent")
        .arg("--resume")
        .arg(file.to_string())
        .arg("--socket")
        .arg(socket)
        .exec();
    restore(fds.len());
    error
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn fd(&mut self, fd: RawFd) {
        self.u32(fd as u32);
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u64(v.len() as u64);
        self.0.extend_from_slice(v);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(invalid("the handover ends early"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn fd(&mut self) -> io::Result<RawFd> {
        let fd = self.u32()? as i32;
        if fd < 0 {
            return Err(invalid("a descriptor out of range"));
        }
        Ok(fd)
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = usize::try_from(self.u64()?).map_err(io::Error::other)?;
        self.take(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State {
            instance: 0x1234_5678_9abc,
            next_pane: 1007,
            listener: 3,
            panes: vec![
                PaneState {
                    id: 1005,
                    command: vec!["/bin/sh".into(), "-c".into(), "echo é".into()],
                    running: Some((7, 4242)),
                    exited: None,
                    cols: 80,
                    rows: 24,
                    screen: b"\x1b[2Jready".to_vec(),
                    output: 12,
                    input: 3,
                },
                PaneState {
                    id: 1006,
                    command: Vec::new(),
                    running: None,
                    exited: Some(Some(-1)),
                    cols: 1,
                    rows: 2,
                    screen: Vec::new(),
                    output: 0,
                    input: 0,
                },
            ],
            x11: Some(X11State {
                listener: 9,
                number: 10,
                cookie: [7; 16],
            }),
            sound: None,
        }
    }

    #[test]
    fn a_handover_reads_back_as_it_was_written() {
        let s = state();
        assert_eq!(State::decode(&s.encode()).unwrap(), s);
        assert_eq!(s.descriptors(), vec![3, 7, 9]);
    }

    #[test]
    fn a_handover_cut_short_or_of_another_format_is_refused() {
        let bytes = state().encode();
        assert!(State::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut other = bytes.clone();
        other[MAGIC.len() + 3] += 1;
        assert!(State::decode(&other).is_err());
        assert!(State::decode(b"something else").is_err());
    }

    #[test]
    fn the_file_handed_over_holds_what_was_written_and_has_no_name() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = state().encode();
        let fd = unnamed_file(dir.path(), &bytes).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let raw = std::os::fd::IntoRawFd::into_raw_fd(fd);
        assert_eq!(read(raw).unwrap(), state());
    }
}
