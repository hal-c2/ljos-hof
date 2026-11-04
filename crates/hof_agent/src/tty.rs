//! `hof copy`, `hof paste` and `hof open`: a program asking the terminal
//! it runs in, through escape sequences on `/dev/tty`.
//!
//! The sequences travel with the program's output, so the same command
//! works in a local pane, in a remote one (through `hof agent` there and
//! back), and for copying in any terminal that takes OSC 52.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use hof_vt::Clipboard;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no terminal to ask: {0}")]
    NoTerminal(#[source] io::Error),
    #[error("the terminal did not answer (it may not let programs read the clipboard)")]
    NoAnswer,
    #[error("{0}: {1}")]
    Path(String, #[source] io::Error),
}

/// Puts `text` on the terminal's `clipboard`.
pub fn copy(text: &str, clipboard: Clipboard) -> Result<(), Error> {
    say(&hof_vt::carrying(clipboard, text))
}

/// What is on the terminal's `clipboard`, as it answers within `timeout`.
/// Hof asks the user first, so the timeout leaves time to answer.
pub fn paste(clipboard: Clipboard, timeout: Duration) -> Result<String, Error> {
    use rustix::event::{PollFd, PollFlags, Timespec};
    use rustix::termios::{OptionalActions, tcgetattr, tcsetattr};

    let mut tty = open_tty()?;
    let was = tcgetattr(&tty).map_err(|e| Error::NoTerminal(e.into()))?;
    let mut raw = was.clone();
    // Raw, so the answer, which ends in no newline, is read as it comes
    // and is not echoed onto the screen.
    raw.make_raw();
    tcsetattr(&tty, OptionalActions::Now, &raw).map_err(|e| Error::NoTerminal(e.into()))?;
    let read = (|| {
        tty.write_all(&hof_vt::read_request(clipboard))
            .and_then(|()| tty.flush())
            .map_err(Error::NoTerminal)?;
        let deadline = Instant::now() + timeout;
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            if let Some(text) = hof_vt::answer_of(&got) {
                return Ok(text);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::NoAnswer);
            }
            let limit = Timespec {
                tv_sec: left.as_secs() as _,
                tv_nsec: left.subsec_nanos() as _,
            };
            let mut fds = [PollFd::new(&tty, PollFlags::IN)];
            match rustix::event::poll(&mut fds, Some(&limit)) {
                Ok(0) | Err(rustix::io::Errno::INTR) => continue,
                Ok(_) => {}
                Err(e) => return Err(Error::NoTerminal(e.into())),
            }
            match tty.read(&mut buf) {
                Ok(0) => return Err(Error::NoAnswer),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::NoTerminal(e)),
            }
        }
    })();
    // Whatever came of it, the terminal goes back as it was.
    let _ = tcsetattr(&tty, OptionalActions::Now, &was);
    read
}

/// Asks the terminal to open `target`, a URL or a file here, with what
/// the user's desktop opens it with.
pub fn open(target: &str) -> Result<(), Error> {
    say(&hof_vt::open_request(&url_of(target)?))
}

/// `target` as a URL: as it is if it has a scheme, else as a file.
fn url_of(target: &str) -> Result<String, Error> {
    if has_scheme(target) {
        return Ok(target.to_owned());
    }
    let path = std::path::absolute(Path::new(target))
        .and_then(|p| p.canonicalize())
        .map_err(|e| Error::Path(target.to_owned(), e))?;
    Ok(file_url(&path))
}

/// Whether `s` starts with a URL scheme and a colon (`https:`,
/// `mailto:`), and is not a Windows-looking `C:` drive.
fn has_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else {
        return false;
    };
    scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `file://` and `path`, with what a URL cannot hold escaped.
pub fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
            url.push(char::from(b));
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

fn open_tty() -> Result<File, Error> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(Error::NoTerminal)
}

fn say(bytes: &[u8]) -> Result<(), Error> {
    let mut tty = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(Error::NoTerminal)?;
    tty.write_all(bytes)
        .and_then(|()| tty.flush())
        .map_err(Error::NoTerminal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_stays_and_a_file_becomes_one() {
        assert_eq!(
            url_of("https://ljos.dev/a?b").unwrap(),
            "https://ljos.dev/a?b"
        );
        assert_eq!(url_of("mailto:me@here").unwrap(), "mailto:me@here");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b%.txt");
        std::fs::write(&file, "").unwrap();
        let url = url_of(file.to_str().unwrap()).unwrap();
        assert!(url.starts_with("file:///"), "{url}");
        assert!(url.ends_with("/a%20b%25.txt"), "{url}");
    }

    #[test]
    fn a_file_that_is_not_there_is_said() {
        assert!(matches!(url_of("no/such/file"), Err(Error::Path(..))));
    }

    #[test]
    fn a_drive_letter_or_a_bare_word_has_no_scheme() {
        assert!(!has_scheme("C:/x"));
        assert!(!has_scheme("notes.txt"));
        assert!(!has_scheme("1x:y"));
        assert!(has_scheme("x-man-page:ls"));
    }
}
