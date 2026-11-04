//! The session on disk.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, SavedWindow, Session, VERSION, validate};

const SCROLLBACK_DIR: &str = "scrollback";
const SCROLLBACK_EXT: &str = "vt";

/// Writes `session` to `path` and its scrollback beside it, replacing what
/// was there. A crash part way leaves the old session or the new one, never
/// a torn file. Only one process should save to a path at a time.
pub fn save(path: &Path, session: &Session) -> Result<(), Error> {
    // Checked before anything is written, since two panes with one id would
    // share a scrollback file.
    validate::check_ids(session, path)?;

    let dir = parent(path);
    create_private_dir(&dir)?;
    let scrollback_dir = dir.join(SCROLLBACK_DIR);

    let mut kept = HashSet::new();
    for (window, w) in session.windows.iter().enumerate() {
        for pane in w.tabs.iter().flat_map(|t| t.panes()) {
            let Some(text) = &pane.scrollback else {
                continue;
            };
            if kept.is_empty() {
                create_private_dir(&scrollback_dir)?;
            }
            let name = scrollback_name(window, pane.id);
            write_atomically(&scrollback_dir.join(&name), text.as_bytes())?;
            kept.insert(name);
        }
    }

    // Serde skips the scrollback, so the session is written as it is, but
    // under this crate's version whatever the caller's says.
    #[derive(Serialize)]
    struct Out<'a> {
        version: u32,
        windows: &'a [SavedWindow],
    }
    let out = Out {
        version: VERSION,
        windows: &session.windows,
    };
    let mut json =
        serde_json::to_vec_pretty(&out).map_err(|e| io_error(path, io::Error::from(e)))?;
    json.push(b'\n');
    write_atomically(path, &json)?;

    // Only once the new session is in place: until then the old one may
    // still name these files.
    remove_stale_scrollback(&scrollback_dir, &kept)
}

/// The session at `path`, or `None` if nothing was saved there.
pub fn load(path: &Path) -> Result<Option<Session>, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(path, source)),
    };
    let corrupt = |source| Error::Corrupt {
        path: path.to_owned(),
        source,
    };

    // The version is read on its own first, so a newer file whose shape
    // has changed is reported as newer, not as broken.
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let Version { version } = serde_json::from_slice(&bytes).map_err(corrupt)?;
    if version > VERSION {
        return Err(Error::FutureVersion {
            path: path.to_owned(),
            found: version,
        });
    }
    let mut session: Session = serde_json::from_slice(&bytes).map_err(corrupt)?;
    validate::check_ids(&session, path)?;

    // Read before repairing, which may drop windows and so renumber them.
    let scrollback_dir = parent(path).join(SCROLLBACK_DIR);
    for (window, w) in session.windows.iter_mut().enumerate() {
        for tab in &mut w.tabs {
            let mut panes = Vec::new();
            tab.root.panes_mut(&mut panes);
            for pane in panes {
                let file = scrollback_dir.join(scrollback_name(window, pane.id));
                pane.scrollback = read_scrollback(&file)?;
            }
        }
    }

    validate::repair(&mut session);
    session.version = VERSION;
    Ok(Some(session))
}

/// Forgets the session at `path` and its scrollback. Nothing saved is fine.
pub fn clear(path: &Path) -> Result<(), Error> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(path, source)),
    }
    let dir = parent(path).join(SCROLLBACK_DIR);
    remove_stale_scrollback(&dir, &HashSet::new())?;
    match fs::remove_dir(&dir) {
        Ok(()) => Ok(()),
        // Something not ours is in it, or it was never made: either way
        // there is no scrollback left.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(source) => Err(io_error(&dir, source)),
    }
}

fn scrollback_name(window: usize, pane: u32) -> OsString {
    format!("{window}-{pane}.{SCROLLBACK_EXT}").into()
}

fn read_scrollback(file: &Path) -> Result<Option<String>, Error> {
    match fs::read(file) {
        // A snapshot is always UTF-8 when written; a damaged one is still
        // worth showing, a stray character or two notwithstanding.
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(file, source)),
    }
}

/// Removes our scrollback files that aren't in `kept`, leaving anything
/// else in the directory alone.
fn remove_stale_scrollback(dir: &Path, kept: &HashSet<OsString>) -> Result<(), Error> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error(dir, source)),
    };
    for entry in entries {
        let entry = entry.map_err(|source| io_error(dir, source))?;
        let file = entry.path();
        let ours = file.extension().is_some_and(|e| e == SCROLLBACK_EXT);
        if ours && !kept.contains(&entry.file_name()) {
            match fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error(&file, source)),
            }
        }
    }
    Ok(())
}

/// Writes `bytes` to a private temporary file beside `path`, flushes it to
/// disk and renames it over `path`, so readers see all of it or none.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let dir = parent(path);
    let mut builder = tempfile::Builder::new();
    builder.prefix(".hof-").suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut tmp = builder
        .tempfile_in(&dir)
        .map_err(|source| io_error(&dir, source))?;
    tmp.write_all(bytes)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|source| io_error(tmp.path(), source))?;
    tmp.persist(path).map_err(|e| io_error(path, e.error))?;
    // The rename is only durable once the directory is flushed too. Some
    // filesystems can't flush a directory; the file is written regardless,
    // so that is no reason to fail the save.
    if let Ok(d) = fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Makes `dir` and any missing parents, those it makes readable by the user
/// alone. A directory that already exists keeps its permissions: it may be
/// shared, and it isn't ours to change.
fn create_private_dir(dir: &Path) -> Result<(), Error> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|source| io_error(dir, source))
}

fn parent(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_owned(),
        _ => PathBuf::from("."),
    }
}

fn io_error(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.to_owned(),
        source,
    }
}
