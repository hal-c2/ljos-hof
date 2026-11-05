//! The files of the agent's machine, reached from Hof's: a channel of its
//! own (`files`), that Hof opens, speaking 9P2000.L ([`ninep`]).
//!
//! The agent serves its user's files on it ([`serve`]). Hof asks for them
//! ([`Client`]): when a program there runs `hof open FILE`, or its `file:`
//! link is clicked, Hof copies the file here ([`Mirror`]), opens the copy
//! with what opens such files here, and puts it back each time it is saved.

mod client;
mod mirror;
pub mod ninep;
mod server;

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

pub use client::Client;
pub use mirror::Mirror;
pub use server::serve;

use crate::relay::Socket;

/// What the agent knows the channel by.
pub const SERVICE: &str = "files";

/// The agent's end of a new files channel: a socket whose other end a
/// thread serves.
pub fn open_server() -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
    let (ours, theirs) = UnixStream::pair()?;
    std::thread::Builder::new()
        .name("hof-files".into())
        .spawn(move || {
            let _ = serve(theirs);
        })?;
    Ok((Box::new(ours), Vec::new()))
}

/// The path a program on the other machine meant by `url`: a `file:` URL
/// (`file:///p`, `file://host/p`, `file:/p`) or an absolute path, with its
/// escapes undone. `None` for anything else, a relative path among them:
/// where it is relative to is there, not here.
pub fn path_of_url(url: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    let rest = match url.split_once(':') {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => {
            match rest.strip_prefix("//") {
                // The host it names is the program's own machine.
                Some(after) => &after[after.find('/')?..],
                None => rest,
            }
        }
        _ => url,
    };
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
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

/// Where Hof keeps what it copies here: `$XDG_CACHE_HOME/hof`, else
/// `~/Library/Caches/hof` on a Mac and `~/.cache/hof` elsewhere.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        && dir.is_absolute()
    {
        return Some(dir.join("hof"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Caches/hof"))
    } else {
        Some(home.join(".cache/hof"))
    }
}

/// The directory a host's copies go in, under `cache`: `hosts/<name>`, the
/// name kept to one plain path component.
pub fn host_dir(cache: &Path, host: &str) -> PathBuf {
    let mut name: String = host
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    if name.is_empty() || name.starts_with('.') {
        name.insert(0, '_');
    }
    cache.join("hosts").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_url_or_absolute_path_is_a_path_there() {
        for url in [
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "file://box/etc/passwd",
            "File:/etc/passwd",
            "/etc/passwd",
        ] {
            assert_eq!(
                path_of_url(url).as_deref(),
                Some(&b"/etc/passwd"[..]),
                "{url}"
            );
        }
        assert_eq!(
            path_of_url("file:///a%20b/%E2%9C%93").as_deref(),
            Some("/a b/✓".as_bytes())
        );
        for not in [
            "relative.desktop",
            "https://a.example",
            "file://box",
            "file:a",
        ] {
            assert_eq!(path_of_url(not), None, "{not}");
        }
    }

    #[test]
    fn a_file_url_reads_back_as_its_path() {
        let path = Path::new("/tmp/a b/c%d");
        assert_eq!(file_url(path), "file:///tmp/a%20b/c%25d");
        assert_eq!(
            path_of_url(&file_url(path)).as_deref(),
            Some(&b"/tmp/a b/c%d"[..])
        );
    }

    #[test]
    fn a_host_is_one_directory() {
        let cache = Path::new("/c");
        assert_eq!(host_dir(cache, "dev"), Path::new("/c/hosts/dev"));
        assert_eq!(host_dir(cache, "../x/y"), Path::new("/c/hosts/_.._x_y"));
        assert_eq!(host_dir(cache, ""), Path::new("/c/hosts/_"));
    }
}
