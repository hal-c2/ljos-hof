//! Sound for remote panes: the PulseAudio native protocol, which
//! PipeWire's `pipewire-pulse` speaks too.
//!
//! The agent listens on a socket of its own and gives the panes it opens
//! a `PULSE_SERVER` for it. A client opens with its first packet, the
//! authorization. Hof puts in the cookie of the server here, if there is
//! one, and connects to that server.
//!
//! The authorization also offers to share memory with the server, which
//! a client and server on two machines can't. Hof takes the offer out,
//! and the sound comes in the packets instead.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::client::Service;
use crate::relay::Socket;

/// The service's name, on the wire.
pub const SERVICE: &str = "sound";

/// A packet's header: its length, channel, offset (two words) and flags.
pub const DESCRIPTOR: usize = 20;

/// The most a client's first packet may hold. Its authorization is far
/// less than this.
pub const MOST_OPENING: usize = 64 * 1024;

/// What a connection opened with that was not a packet one could take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a PulseAudio packet a client opens with")]
pub struct NotOpening;

/// How long the packet at the start of `bytes` is, header and all; `None`
/// while more is to come.
pub fn packet_len(bytes: &[u8]) -> Result<Option<usize>, NotOpening> {
    let Some(length) = bytes.first_chunk::<4>() else {
        return Ok(None);
    };
    let len = DESCRIPTOR + u32::from_be_bytes(*length) as usize;
    if len > MOST_OPENING {
        return Err(NotOpening);
    }
    Ok((bytes.len() >= len).then_some(len))
}

/// The command that authorizes a client.
const AUTH: u32 = 8;
const TAG_U32: u8 = b'L';
const TAG_ARBITRARY: u8 = b'x';
/// Bits of the version in the authorization: the client can share memory,
/// and memfd memory.
const SHARES_MEMORY: u32 = 0x8000_0000 | 0x4000_0000;
/// How long a cookie is.
pub const COOKIE_LEN: usize = 256;

/// Takes the offer to share memory out of a client's authorization
/// `packet`, and puts `cookie` in it, if there is one. A packet that is not
/// one is left as it is.
pub fn rewrite_auth(packet: &mut [u8], cookie: Option<&[u8]>) {
    let Some(payload) = packet.get_mut(DESCRIPTOR..) else {
        return;
    };
    let u32_at = |p: &[u8], at: usize| {
        (p.get(at) == Some(&TAG_U32))
            .then(|| p.get(at + 1..at + 5))
            .flatten()
            .map(|b| u32::from_be_bytes(b.try_into().expect("four bytes")))
    };
    // The command, its tag, the client's version, the cookie.
    if u32_at(payload, 0) != Some(AUTH) || u32_at(payload, 5).is_none() {
        return;
    }
    let Some(version) = u32_at(payload, 10) else {
        return;
    };
    payload[11..15].copy_from_slice(&(version & !SHARES_MEMORY).to_be_bytes());
    let cookie_at = 15;
    if payload.get(cookie_at) != Some(&TAG_ARBITRARY)
        || payload.get(cookie_at + 1..cookie_at + 5) != Some(&(COOKIE_LEN as u32).to_be_bytes()[..])
    {
        return;
    }
    if let (Some(cookie), Some(slot)) = (
        cookie.filter(|c| c.len() == COOKIE_LEN),
        payload.get_mut(cookie_at + 5..cookie_at + 5 + COOKIE_LEN),
    ) {
        slot.copy_from_slice(cookie);
    }
}

/// The sound server here, for the connections the agent passes on.
#[derive(Debug)]
pub struct Forward {
    server: PathBuf,
    /// The files its cookie may be in, read for each connection.
    cookies: Vec<PathBuf>,
}

impl Forward {
    pub fn new(server: PathBuf, cookies: Vec<PathBuf>) -> Self {
        Self { server, cookies }
    }

    /// The server `$PULSE_SERVER` names, if it is a socket here, else
    /// `$XDG_RUNTIME_DIR/pulse/native`, else, where there is no
    /// `$XDG_RUNTIME_DIR`, as on a Mac, one PulseAudio keeps under
    /// `~/.config/pulse`; its cookie from `$PULSE_COOKIE`, or where
    /// PulseAudio keeps it.
    pub fn from_env() -> Option<Self> {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from);
        let config = var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let named = var("PULSE_SERVER").and_then(|servers| {
            servers.to_str()?.split_whitespace().find_map(|s| {
                let path = s.strip_prefix("unix:").unwrap_or(s);
                path.starts_with('/').then(|| PathBuf::from(path))
            })
        });
        let server = named
            .or_else(|| {
                var("XDG_RUNTIME_DIR").map(|dir| Path::new(&dir).join("pulse").join("native"))
            })
            .or_else(|| {
                config
                    .as_deref()
                    .and_then(|c| kept_runtime(&c.join("pulse")))
            })?;
        let cookies = [
            var("PULSE_COOKIE").map(PathBuf::from),
            config.map(|c| c.join("pulse").join("cookie")),
            home.map(|h| h.join(".pulse-cookie")),
        ];
        Some(Self::new(server, cookies.into_iter().flatten().collect()))
    }

    /// Whether the server's socket is here; one that is not, no program
    /// should be told of.
    pub fn is_there(&self) -> bool {
        self.server.exists()
    }

    fn cookie(&self) -> Option<Vec<u8>> {
        self.cookies
            .iter()
            .find_map(|p| std::fs::read(p).ok())
            .filter(|c| c.len() == COOKIE_LEN)
    }
}

/// The socket of a server running here that PulseAudio keeps in
/// `pulse`, the user's configuration, as `<machine>-runtime/native`, where
/// it has no `$XDG_RUNTIME_DIR` to keep it in.
fn kept_runtime(pulse: &Path) -> Option<PathBuf> {
    std::fs::read_dir(pulse)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with("-runtime"))
        .map(|e| e.path().join("native"))
        .find(|socket| UnixStream::connect(socket).is_ok())
}

impl Service for Forward {
    fn name(&self) -> &str {
        SERVICE
    }

    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
        let not = || io::Error::new(io::ErrorKind::InvalidData, NotOpening);
        let len = packet_len(opening).map_err(|_| not())?.ok_or_else(not)?;
        let mut first = opening.to_vec();
        rewrite_auth(&mut first[..len], self.cookie().as_deref());
        Ok((Box::new(UnixStream::connect(&self.server)?), first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client's authorization, as libpulse sends it.
    fn auth_packet(version: u32, cookie: &[u8; COOKIE_LEN]) -> Vec<u8> {
        let mut payload = vec![TAG_U32];
        payload.extend(AUTH.to_be_bytes());
        payload.push(TAG_U32);
        payload.extend(1u32.to_be_bytes());
        payload.push(TAG_U32);
        payload.extend(version.to_be_bytes());
        payload.push(TAG_ARBITRARY);
        payload.extend((COOKIE_LEN as u32).to_be_bytes());
        payload.extend(cookie);
        let mut packet = (payload.len() as u32).to_be_bytes().to_vec();
        packet.extend(u32::MAX.to_be_bytes());
        packet.extend([0; 12]);
        packet.extend(payload);
        packet
    }

    #[test]
    fn a_packet_is_whole_once_its_header_and_payload_are_in() {
        let packet = auth_packet(35, &[1; COOKIE_LEN]);
        assert_eq!(packet_len(&packet), Ok(Some(packet.len())));
        assert_eq!(packet_len(&packet[..packet.len() - 1]), Ok(None));
        assert_eq!(packet_len(&packet[..3]), Ok(None));
        assert_eq!(packet_len(&[0xff; 20]), Err(NotOpening));
    }

    #[test]
    fn the_authorization_gets_the_cookie_here_and_shares_no_memory() {
        let mut packet = auth_packet(35 | SHARES_MEMORY, &[1; COOKIE_LEN]);
        rewrite_auth(&mut packet, Some(&[7; COOKIE_LEN]));
        assert_eq!(packet, auth_packet(35, &[7; COOKIE_LEN]));

        let mut kept = auth_packet(35 | SHARES_MEMORY, &[1; COOKIE_LEN]);
        rewrite_auth(&mut kept, None);
        assert_eq!(kept, auth_packet(35, &[1; COOKIE_LEN]), "no cookie here");
    }

    #[test]
    fn a_server_kept_in_the_users_configuration_is_found_while_it_listens() {
        let dir = tempfile::tempdir().unwrap();
        let pulse = dir.path().join("pulse");
        std::fs::create_dir_all(pulse.join("abc-runtime")).unwrap();
        std::fs::create_dir_all(pulse.join("gone-runtime")).unwrap();
        assert_eq!(kept_runtime(&pulse), None);
        let socket = pulse.join("abc-runtime").join("native");
        let _listening = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert_eq!(kept_runtime(&pulse), Some(socket));
    }

    #[test]
    fn a_packet_that_is_not_an_authorization_is_left_alone() {
        let mut packet = auth_packet(35 | SHARES_MEMORY, &[1; COOKIE_LEN]);
        packet[DESCRIPTOR + 4] = 9;
        let before = packet.clone();
        rewrite_auth(&mut packet, Some(&[7; COOKIE_LEN]));
        assert_eq!(packet, before);
        rewrite_auth(&mut [0; 4], None);
    }
}
