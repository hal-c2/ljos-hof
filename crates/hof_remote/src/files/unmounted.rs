//! Where Hof can't mount another machine's files (off Linux): a
//! [`Mount`] is never made, and a folder opened there is copied instead.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// A host's files, mounted here: not on this system.
pub struct Mount {
    at: PathBuf,
}

impl Mount {
    /// Says this system can't.
    pub fn new(
        _at: PathBuf,
        _host: &str,
        _connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mounting another machine's files is only on Linux",
        ))
    }

    /// Says this system can't.
    pub fn of(
        at: PathBuf,
        host: &str,
        _root: &[u8],
        connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::new(at, host, connect)
    }

    pub fn path(&self) -> &Path {
        &self.at
    }

    pub fn local_of(&self, _remote: &[u8]) -> io::Result<PathBuf> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
