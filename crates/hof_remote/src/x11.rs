//! X11 for remote panes, as `ssh -X` does it, over Hof's connection.
//!
//! The agent listens on a display of its own on its machine, and gives
//! the panes it opens a `DISPLAY` for it and an `XAUTHORITY` file with a
//! cookie it made up. A client that connects opens with its setup, which
//! carries the cookie. The agent checks that and takes it out, and passes
//! the connection on to Hof. Hof puts in the cookie of the X server here,
//! which never leaves this machine, and connects to that server.
//!
//! This is as much of the protocol as that needs: the setup a client
//! opens with ([`Setup`]), the answer that refuses one ([`refusal`]), the
//! files that keep cookies ([`Auth`]), and what `DISPLAY` names
//! ([`Target`]).

use std::io;
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::client::Service;
use crate::relay::Socket;

/// The one kind of authorization Hof knows: a secret both ends share.
pub const COOKIE: &[u8] = b"MIT-MAGIC-COOKIE-1";

/// The service's name, on the wire.
pub const SERVICE: &str = "x11";

/// The setup an X11 client opens its connection with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// Whether the client's numbers are big-endian (`B`), not little (`l`).
    pub big_endian: bool,
    pub major: u16,
    pub minor: u16,
    /// The kind of authorization, such as [`COOKIE`]; empty for none.
    pub auth_name: Vec<u8>,
    pub auth_data: Vec<u8>,
}

/// What a connection opened with that was not an X11 setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not an X11 connection setup")]
pub struct NotSetup;

/// What pads `n` bytes out to four.
fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

impl Setup {
    /// The setup at the start of `bytes`, and how many bytes it takes;
    /// `None` while more is to come.
    pub fn parse(bytes: &[u8]) -> Result<Option<(Setup, usize)>, NotSetup> {
        let big_endian = match bytes.first() {
            None => return Ok(None),
            Some(b'B') => true,
            Some(b'l') => false,
            Some(_) => return Err(NotSetup),
        };
        if bytes.len() < 12 {
            return Ok(None);
        }
        let u16_at = |at: usize| {
            let b = [bytes[at], bytes[at + 1]];
            if big_endian {
                u16::from_be_bytes(b)
            } else {
                u16::from_le_bytes(b)
            }
        };
        let (name_len, data_len) = (usize::from(u16_at(6)), usize::from(u16_at(8)));
        let name_at = 12;
        let data_at = name_at + name_len + pad(name_len);
        let end = data_at + data_len + pad(data_len);
        if bytes.len() < end {
            return Ok(None);
        }
        let setup = Setup {
            big_endian,
            major: u16_at(2),
            minor: u16_at(4),
            auth_name: bytes[name_at..name_at + name_len].to_vec(),
            auth_data: bytes[data_at..data_at + data_len].to_vec(),
        };
        Ok(Some((setup, end)))
    }

    pub fn encode(&self) -> Vec<u8> {
        let u16_of = |v: u16| {
            if self.big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut out = vec![if self.big_endian { b'B' } else { b'l' }, 0];
        out.extend(u16_of(self.major));
        out.extend(u16_of(self.minor));
        out.extend(u16_of(self.auth_name.len() as u16));
        out.extend(u16_of(self.auth_data.len() as u16));
        out.extend([0, 0]);
        for part in [&self.auth_name, &self.auth_data] {
            out.extend(part.iter());
            out.extend(std::iter::repeat_n(0, pad(part.len())));
        }
        out
    }

    /// The same setup with `cookie` for its authorization, or none.
    pub fn with_cookie(&self, cookie: Option<&[u8]>) -> Setup {
        let (name, data) = match cookie {
            Some(cookie) => (COOKIE.to_vec(), cookie.to_vec()),
            None => (Vec::new(), Vec::new()),
        };
        Setup {
            auth_name: name,
            auth_data: data,
            ..self.clone()
        }
    }
}

/// What a server answers a setup it refuses with, `reason` and all, in
/// the client's byte order.
pub fn refusal(big_endian: bool, reason: &str) -> Vec<u8> {
    let reason = &reason.as_bytes()[..reason.len().min(255)];
    let padded = reason.len() + pad(reason.len());
    let u16_of = |v: u16| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    // Failed, the reason's length, the protocol version, and the length
    // of what follows in fours.
    let mut out = vec![0, reason.len() as u8];
    out.extend(u16_of(11));
    out.extend(u16_of(0));
    out.extend(u16_of((padded / 4) as u16));
    out.extend(reason);
    out.extend(std::iter::repeat_n(0, padded - reason.len()));
    out
}

/// Any address, in an [`Auth`].
pub const FAMILY_WILD: u16 = 0xffff;
/// This machine, by its host name, in an [`Auth`].
pub const FAMILY_LOCAL: u16 = 256;

/// One entry of an authorization file (`~/.Xauthority`): a cookie for a
/// display on a machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Auth {
    pub family: u16,
    pub address: Vec<u8>,
    /// The display's number, as text; empty for any.
    pub number: Vec<u8>,
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

impl Auth {
    /// The entries in a file's `bytes`, up to the first that is cut off.
    pub fn parse_all(mut bytes: &[u8]) -> Vec<Auth> {
        fn u16_of(bytes: &mut &[u8]) -> Option<u16> {
            let (n, rest) = bytes.split_first_chunk::<2>()?;
            *bytes = rest;
            Some(u16::from_be_bytes(*n))
        }
        fn counted(bytes: &mut &[u8]) -> Option<Vec<u8>> {
            let n = usize::from(u16_of(bytes)?);
            let (it, rest) = bytes.split_at_checked(n)?;
            *bytes = rest;
            Some(it.to_vec())
        }
        let mut all = Vec::new();
        while !bytes.is_empty() {
            let entry = (|| {
                Some(Auth {
                    family: u16_of(&mut bytes)?,
                    address: counted(&mut bytes)?,
                    number: counted(&mut bytes)?,
                    name: counted(&mut bytes)?,
                    data: counted(&mut bytes)?,
                })
            })();
            match entry {
                Some(entry) => all.push(entry),
                None => break,
            }
        }
        all
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend(self.family.to_be_bytes());
        for part in [&self.address, &self.number, &self.name, &self.data] {
            out.extend((part.len() as u16).to_be_bytes());
            out.extend(part.iter());
        }
    }

    /// The cookie in `auths` for display `number` on this machine, which is
    /// called `host`.
    pub fn cookie_for(auths: &[Auth], number: &str, host: &str) -> Option<Vec<u8>> {
        auths
            .iter()
            .find(|a| {
                a.name == COOKIE
                    && (a.number.is_empty() || a.number == number.as_bytes())
                    && (a.family == FAMILY_WILD
                        || (a.family == FAMILY_LOCAL && a.address == host.as_bytes()))
            })
            .map(|a| a.data.clone())
    }
}

/// Where an X server listens, as `DISPLAY` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `:N`: the socket for display N on this machine.
    Local(u32),
    /// `/path:N`, as macOS's launchd gives: the socket at the path.
    Path(PathBuf),
    /// `host:N`: TCP, port 6000 + N.
    Tcp(String, u16),
}

impl Target {
    /// What `display` names, and its number as text; `None` for what is
    /// not a display.
    pub fn of(display: &str) -> Option<(Target, String)> {
        let (host, rest) = display.rsplit_once(':')?;
        let number = rest.split('.').next().unwrap_or_default();
        let n: u32 = number.parse().ok()?;
        let target = match host {
            "" | "unix" => Target::Local(n),
            path if path.starts_with('/') => Target::Path(path.into()),
            host => Target::Tcp(host.into(), u16::try_from(6000 + n).ok()?),
        };
        Some((target, number.into()))
    }

    pub fn connect(&self) -> io::Result<Box<dyn Socket>> {
        match self {
            Target::Local(n) => {
                #[cfg(target_os = "linux")]
                {
                    use std::os::linux::net::SocketAddrExt;
                    let name = format!("/tmp/.X11-unix/X{n}");
                    let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                    if let Ok(s) = UnixStream::connect_addr(&addr) {
                        return Ok(Box::new(s));
                    }
                }
                Ok(Box::new(UnixStream::connect(format!(
                    "/tmp/.X11-unix/X{n}"
                ))?))
            }
            Target::Path(path) => Ok(Box::new(UnixStream::connect(path)?)),
            Target::Tcp(host, port) => Ok(Box::new(TcpStream::connect((host.as_str(), *port))?)),
        }
    }
}

/// This machine's name, as authorization files have it.
pub fn host_name() -> String {
    rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned()
}

/// The X server here, for the connections the agent passes on.
#[derive(Debug)]
pub struct Forward {
    target: Target,
    number: String,
    /// The file with its cookie, read for each connection: an X server
    /// started again (Xwayland comes and goes) has a new one.
    xauthority: Option<PathBuf>,
}

impl Forward {
    /// The server `display` names, with its cookie from `xauthority`.
    pub fn new(display: &str, xauthority: Option<PathBuf>) -> Option<Self> {
        let (target, number) = Target::of(display)?;
        Some(Self {
            target,
            number,
            xauthority,
        })
    }

    /// The server `$DISPLAY` names, with its cookie from `$XAUTHORITY`, or
    /// `~/.Xauthority`. `None` with no display, as on a desktop without
    /// X11 or Xwayland.
    pub fn from_env() -> Option<Self> {
        let display = std::env::var("DISPLAY").ok()?;
        let xauthority = std::env::var_os("XAUTHORITY")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".Xauthority")));
        Self::new(&display, xauthority)
    }

    fn cookie(&self) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.xauthority.as_ref()?).ok()?;
        Auth::cookie_for(&Auth::parse_all(&bytes), &self.number, &host_name())
    }
}

impl Service for Forward {
    fn name(&self) -> &str {
        SERVICE
    }

    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
        let not = || io::Error::new(io::ErrorKind::InvalidData, NotSetup);
        let (setup, len) = Setup::parse(opening).map_err(|_| not())?.ok_or_else(not)?;
        let mut first = setup.with_cookie(self.cookie().as_deref()).encode();
        first.extend_from_slice(&opening[len..]);
        Ok((self.target.connect()?, first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(big_endian: bool) -> Setup {
        Setup {
            big_endian,
            major: 11,
            minor: 0,
            auth_name: COOKIE.to_vec(),
            auth_data: (0..16).collect(),
        }
    }

    #[test]
    fn a_setup_comes_back_as_it_went_in_either_byte_order() {
        for big_endian in [true, false] {
            let bytes = setup(big_endian).encode();
            assert_eq!(bytes.len() % 4, 0);
            let mut more = bytes.clone();
            more.extend(b"next");
            assert_eq!(
                Setup::parse(&more),
                Ok(Some((setup(big_endian), bytes.len())))
            );
            for cut in 0..bytes.len() {
                assert_eq!(Setup::parse(&bytes[..cut]), Ok(None), "cut at {cut}");
            }
        }
        assert_eq!(Setup::parse(b"GET / HTTP/1.1"), Err(NotSetup));
    }

    #[test]
    fn a_setup_without_its_cookie_has_no_authorization() {
        let bare = setup(false).with_cookie(None);
        assert_eq!(bare.encode().len(), 12);
        assert_eq!(bare.with_cookie(Some(b"k")).auth_name, COOKIE);
    }

    #[test]
    fn a_refusal_says_why_padded_to_four() {
        let no = refusal(false, "no Hof");
        assert_eq!(no[..2], [0, 6]);
        assert_eq!(u16::from_le_bytes([no[2], no[3]]), 11);
        assert_eq!(u16::from_le_bytes([no[6], no[7]]), 2);
        assert_eq!(&no[8..14], b"no Hof");
        assert_eq!(no.len(), 16);
        assert_eq!(refusal(true, "")[6..8], [0, 0]);
    }

    #[test]
    fn the_cookie_for_a_display_is_found_by_its_number_and_host() {
        let entry = |family, address: &str, number: &str, data: &[u8]| Auth {
            family,
            address: address.into(),
            number: number.into(),
            name: COOKIE.to_vec(),
            data: data.to_vec(),
        };
        let auths = [
            entry(FAMILY_LOCAL, "elsewhere", "0", b"theirs"),
            entry(FAMILY_LOCAL, "here", "1", b"one"),
            entry(FAMILY_LOCAL, "here", "0", b"zero"),
            entry(FAMILY_WILD, "", "", b"any"),
        ];
        let mut file = Vec::new();
        for a in &auths {
            a.encode(&mut file);
        }
        let read = Auth::parse_all(&file);
        assert_eq!(read, auths);
        assert_eq!(Auth::cookie_for(&read, "0", "here"), Some(b"zero".to_vec()));
        assert_eq!(Auth::cookie_for(&read, "7", "here"), Some(b"any".to_vec()));
        assert_eq!(Auth::cookie_for(&read[..2], "0", "here"), None);
        assert_eq!(Auth::parse_all(&file[..file.len() - 1]).len(), 3, "cut off");
    }

    #[test]
    fn a_display_says_where_its_server_is() {
        assert_eq!(Target::of(":0"), Some((Target::Local(0), "0".into())));
        assert_eq!(
            Target::of("unix:10.0"),
            Some((Target::Local(10), "10".into()))
        );
        assert_eq!(
            Target::of("localhost:10.0"),
            Some((Target::Tcp("localhost".into(), 6010), "10".into()))
        );
        assert_eq!(
            Target::of("/private/tmp/launchd/org.xquartz:0"),
            Some((
                Target::Path("/private/tmp/launchd/org.xquartz".into()),
                "0".into()
            ))
        );
        assert_eq!(Target::of("wayland-1"), None);
        assert_eq!(Target::of(":x"), None);
    }
}
