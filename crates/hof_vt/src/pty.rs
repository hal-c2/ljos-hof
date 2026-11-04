//! A program running on a pseudo-terminal.
//!
//! Only the master side stays in this process. The child gets the slave side
//! as its stdio and its controlling terminal, in a session of its own, so job
//! control in the shell works and a hangup reaches the whole process group.

use std::ffi::OsString;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;

use crate::{Error, Size};

/// What to run, and the environment to run it in.
#[derive(Clone, Debug, Default)]
pub struct Spawn {
    /// The program; `None` means the user's login shell (`$SHELL`, falling
    /// back to `/bin/sh`).
    pub program: Option<PathBuf>,
    pub args: Vec<OsString>,
    /// The starting directory; `None` inherits ours.
    pub cwd: Option<PathBuf>,
    /// Extra variables, applied after the terminal's own.
    pub env: Vec<(OsString, OsString)>,
    /// `TERM` for the child. Defaults to `xterm-256color`, which every host
    /// has a terminfo entry for.
    pub term: Option<String>,
}

impl Spawn {
    /// The user's shell.
    pub fn shell() -> Self {
        Self::default()
    }

    /// A specific program and its arguments.
    pub fn command(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        Self {
            program: Some(program.into()),
            args: args.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    pub(crate) fn term_name(&self) -> &str {
        self.term.as_deref().unwrap_or("xterm-256color")
    }

    fn program(&self) -> PathBuf {
        self.program.clone().unwrap_or_else(|| {
            std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .map_or_else(|| PathBuf::from("/bin/sh"), PathBuf::from)
        })
    }
}

pub(crate) struct Pty {
    pub master: OwnedFd,
    pub child: Child,
}

impl Pty {
    pub fn spawn(spawn: &Spawn, size: Size) -> Result<Self, Error> {
        let master =
            rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)
                .map_err(|e| Error::Pty("openpt", e.into()))?;
        rustix::pty::grantpt(&master).map_err(|e| Error::Pty("grantpt", e.into()))?;
        rustix::pty::unlockpt(&master).map_err(|e| Error::Pty("unlockpt", e.into()))?;
        let slave_path = rustix::pty::ptsname(&master, Vec::new())
            .map_err(|e| Error::Pty("ptsname", e.into()))?;
        set_winsize(&master, size)?;

        let slave = rustix::fs::open(
            slave_path.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| Error::Pty("open slave", e.into()))?;

        let program = spawn.program();
        let mut cmd = Command::new(&program);
        cmd.args(&spawn.args)
            .stdin(Stdio::from(
                slave.try_clone().map_err(|e| Error::Pty("dup", e))?,
            ))
            .stdout(Stdio::from(
                slave.try_clone().map_err(|e| Error::Pty("dup", e))?,
            ))
            .stderr(Stdio::from(slave))
            .env("TERM", spawn.term_name())
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "ljos-hof")
            .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"))
            // Inherited from a parent terminal, these would describe the
            // wrong window to the child.
            .env_remove("COLUMNS")
            .env_remove("LINES");
        for (key, value) in &spawn.env {
            cmd.env(key, value);
        }
        if let Some(cwd) = &spawn.cwd {
            cmd.current_dir(cwd);
        }
        if spawn.program.is_none() {
            // A login shell reads the profile, as a terminal's shell should.
            let name = program
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            cmd.arg0(format!("-{name}"));
        }

        // SAFETY: the closure runs in the forked child before exec and only
        // makes async-signal-safe system calls; it allocates nothing.
        unsafe {
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                // stdin is the slave by now; make it the controlling terminal.
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }

        let child = cmd.spawn().map_err(|e| Error::Spawn(program, e))?;
        Ok(Self { master, child })
    }
}

pub(crate) fn set_winsize(master: impl AsFd, size: Size) -> Result<(), Error> {
    let winsize = Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: clamp_u16(u32::from(size.cols) * size.cell_width),
        ws_ypixel: clamp_u16(u32::from(size.rows) * size.cell_height),
    };
    rustix::termios::tcsetwinsize(master, winsize).map_err(|e| Error::Pty("tcsetwinsize", e.into()))
}

fn clamp_u16(v: u32) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

/// Hang up on the child's session, as closing a terminal window does.
pub(crate) fn hangup(pid: u32) {
    if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
        // The child is a session leader, so its pid names its process group.
        let _ = rustix::process::kill_process_group(pid, Signal::HUP);
    }
}
