//! Who is on the other end of a Unix socket: Hof's sockets serve only
//! the user who runs it, though the directory they are in should already
//! keep everyone else out.

use std::io;
use std::os::unix::net::UnixStream;

/// The user on the other end of a socket on this machine.
#[cfg(target_os = "linux")]
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    Ok(rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw())
}

/// The user on the other end of a socket on this machine. The BSDs and
/// macOS have no `SO_PEERCRED`, but all have `getpeereid`.
#[cfg(not(target_os = "linux"))]
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: the descriptor is the stream's, open while it is borrowed,
    // and both out-pointers are to locals.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0 {
        Ok(uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Fails unless the other end of `stream` is `uid`.
pub fn same_user_as(stream: &UnixStream, uid: u32) -> io::Result<()> {
    if peer_uid(stream)? == uid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the socket peer belongs to another user",
        ))
    }
}

/// Fails unless the other end of `stream` is the user running this.
pub fn same_user(stream: &UnixStream) -> io::Result<()> {
    same_user_as(stream, rustix::process::getuid().as_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

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
