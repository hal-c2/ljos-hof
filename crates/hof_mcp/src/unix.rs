//! Unix-socket paths and the check that the peer is this user.

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

pub fn fallback_dir(temp_dir: &Path, uid: u32) -> PathBuf {
    temp_dir.join(format!("hof-{uid}"))
}

/// Makes the shared-temporary fallback directory, when `path` uses it. An
/// explicitly configured socket keeps whatever directory its user chose.
pub fn prepare_parent(path: &Path) -> io::Result<()> {
    let uid = rustix::process::getuid().as_raw();
    let fallback = fallback_dir(&std::env::temp_dir(), uid);
    if path.parent() == Some(fallback.as_path()) {
        ensure_private_dir(&fallback, uid)?;
    }
    Ok(())
}

fn ensure_private_dir(path: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
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

pub fn connect(path: &Path) -> io::Result<UnixStream> {
    connect_as(path, rustix::process::getuid().as_raw())
}

fn connect_as(path: &Path, expected_uid: u32) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    same_user_as(&stream, expected_uid)?;
    Ok(stream)
}

pub fn same_user(stream: &UnixStream) -> io::Result<()> {
    same_user_as(stream, rustix::process::getuid().as_raw())
}

#[cfg(target_os = "linux")]
fn same_user_as(stream: &UnixStream, expected_uid: u32) -> io::Result<()> {
    let credentials = rustix::net::sockopt::socket_peercred(stream)?;
    if credentials.uid.as_raw() == expected_uid {
        Ok(())
    } else {
        Err(foreign_peer())
    }
}

#[cfg(not(target_os = "linux"))]
fn same_user_as(_stream: &UnixStream, _expected_uid: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn foreign_peer() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "the socket peer belongs to another user",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fallback_directory_is_private_and_cannot_be_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("hof-1000");
        ensure_private_dir(&dir, 1000).unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        ensure_private_dir(&dir, 1000).unwrap();
        fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(temp.path(), &dir).unwrap();
        assert!(ensure_private_dir(&dir, 1000).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_peer_with_another_users_identity_is_refused() {
        let (stream, _other) = UnixStream::pair().unwrap();
        let uid = rustix::process::getuid().as_raw();
        same_user_as(&stream, uid).unwrap();
        assert_eq!(
            same_user_as(&stream, uid + 1).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
