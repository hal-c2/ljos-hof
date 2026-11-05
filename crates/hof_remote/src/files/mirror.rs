//! Copies here of files on the other machine, kept the same as them.
//!
//! A file asked for is copied into the host's directory of the cache, at
//! its path there (`hosts/dev/home/me/notes.md`), readable by this user
//! only and never executable. From then on, while it is watched, a copy
//! saved here is written back there once it has settled, and a file
//! changed there is copied again, if the copy here was not changed too:
//! what was saved here last is what the user meant.
//!
//! A copy is watched until it goes from here, or a day after it last
//! changed, or Hof quits.
//!
//! A folder asked for is copied as it is, at its place in the same tree,
//! and not kept the same: its files are read-only here, so nothing saved
//! in them is lost unseen, and one opened by itself later becomes a copy
//! kept the same like any other. A large folder is copied a level at a
//! time until [`FOLDER_FILES`] or [`FOLDER_BYTES`] is reached.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::client::Client;
use super::ninep::{Attr, Time};
use super::reach::Reach;

/// How often copies here are looked at.
const TICK: Duration = Duration::from_millis(500);
/// How often the files there are.
const LOOK_THERE: Duration = Duration::from_secs(3);
/// How long a copy that hasn't changed is watched.
const FORGET: Duration = Duration::from_secs(24 * 60 * 60);
/// The most copies watched at once; the oldest goes for a new one.
const MOST: usize = 64;
/// The most files copied of a folder.
pub const FOLDER_FILES: usize = 2000;
/// The most bytes copied of a folder.
pub const FOLDER_BYTES: u64 = 256 * 1024 * 1024;

/// `d_type`s in a folder's listing.
const DT_DIR: u8 = 4;
const DT_REG: u8 = 8;
/// `S_IFMT` and `S_IFDIR` of a mode.
const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;

/// What [`Mirror::open`] made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// A copy of a file, kept the same as it.
    File(PathBuf),
    /// A copy of a folder as it is now.
    Folder(PathBuf, FolderCopy),
}

impl Opened {
    /// Where the copy is.
    pub fn path(&self) -> &Path {
        match self {
            Opened::File(path) | Opened::Folder(path, _) => path,
        }
    }
}

/// How much of a folder was copied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderCopy {
    /// Files copied, or already here as copies kept the same.
    pub files: usize,
    /// Files and folders left out: past the limits, or neither a file nor
    /// a folder (links, devices).
    pub left_out: usize,
}

/// One host's copies.
///
/// Nothing waits on the network with the list of copies locked: a copy is
/// locked while it is looked at, and the session while it is asked of. So
/// opening a file waits for the one request in flight, not for a sweep of
/// every copy, and is never held up by a lock the UI needs.
pub struct Mirror {
    dir: PathBuf,
    /// The 9P session: one request at a time.
    reach: Reach,
    watched: Mutex<Watching>,
}

#[derive(Default)]
struct Watching {
    /// By their paths there, which never change: found without waiting
    /// for a copy that is being looked at.
    copies: Vec<(Vec<u8>, Arc<Mutex<Watched>>)>,
    /// The thread that looks at them is running.
    running: bool,
}

/// What a copy here was, as far as telling it changed goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    modified: Option<SystemTime>,
    len: u64,
}

/// What the file there was.
type There = (Time, u64);

struct Watched {
    remote: Vec<u8>,
    local: PathBuf,
    here: Seen,
    there: There,
    /// Changed here, and so a tick ago: written back when it is the same
    /// a tick later, not halfway through being saved.
    saving: Option<Seen>,
    checked: Instant,
    touched: Instant,
}

fn seen(path: &Path) -> io::Result<Seen> {
    let meta = fs::metadata(path)?;
    Ok(Seen {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

fn there_of(attr: &Attr) -> There {
    (attr.mtime, attr.size)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A directory only this user may enter, and those above it.
fn private_dirs(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

impl Mirror {
    /// Copies of `host`'s files, in `dir`, over the channels `connect`
    /// opens to its agent's files service.
    pub fn new(
        dir: PathBuf,
        host: &str,
        connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            dir,
            reach: Reach::new(host, b"/", connect),
            watched: Mutex::default(),
        })
    }

    /// Where the copy of `remote`, a path there, goes here: under the
    /// host's directory, at that path, with no way out of it.
    pub fn local_of(&self, remote: &[u8]) -> io::Result<PathBuf> {
        let path = Path::new(std::ffi::OsStr::from_bytes(remote));
        let mut local = self.dir.clone();
        let mut named = false;
        for part in path.components() {
            match part {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(name) => {
                    local.push(name);
                    named = true;
                }
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a path with .. in it",
                    ));
                }
            }
        }
        if !path.is_absolute() || !named {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a file's whole path",
            ));
        }
        Ok(local)
    }

    fn find(&self, remote: &[u8]) -> Option<Arc<Mutex<Watched>>> {
        lock(&self.watched)
            .copies
            .iter()
            .find(|(path, _)| path == remote)
            .map(|(_, copy)| copy.clone())
    }

    /// Copies the file at `remote` here, and keeps the two the same from
    /// now on: the copy's path.
    pub fn fetch(self: &Arc<Self>, remote: &[u8]) -> io::Result<PathBuf> {
        let local = self.local_of(remote)?;
        // Asked for now: whatever failed before is tried again at once.
        self.reach.retry_now();
        if let Some(copy) = self.find(remote) {
            let mut watched = lock(&copy);
            // Looked at now, not when the tick comes.
            let now = Instant::now();
            watched.checked = now.checked_sub(LOOK_THERE).unwrap_or(now);
            watched.touched = now;
            return self.sync(&mut watched).map(|()| local);
        }
        let there = self.with_client(|c| download(c, remote, &local))?;
        let now = Instant::now();
        let copy = Arc::new(Mutex::new(Watched {
            remote: remote.to_vec(),
            local: local.clone(),
            here: seen(&local)?,
            there,
            saving: None,
            checked: now,
            touched: now,
        }));
        let mut watching = lock(&self.watched);
        // Asked for twice at once: one copy, watched once.
        if !watching.copies.iter().any(|(path, _)| path == remote) {
            if watching.copies.len() >= MOST {
                watching.copies.remove(0);
            }
            watching.copies.push((remote.to_vec(), copy));
        }
        if !watching.running {
            let mirror = Arc::downgrade(self);
            let started = thread::Builder::new()
                .name("hof-files-watch".into())
                .spawn(move || watch(&mirror));
            watching.running = started.is_ok();
        }
        Ok(local)
    }

    /// Copies `remote` here: a file with [`fetch`](Self::fetch), a folder
    /// with [`fetch_folder`](Self::fetch_folder).
    pub fn open(self: &Arc<Self>, remote: &[u8]) -> io::Result<Opened> {
        if self.is_folder(remote)? {
            let (local, copied) = self.fetch_folder(remote)?;
            return Ok(Opened::Folder(local, copied));
        }
        self.fetch(remote).map(Opened::File)
    }

    /// Whether `remote` is a folder there. One already kept the same is
    /// a file, and not asked about again.
    pub fn is_folder(&self, remote: &[u8]) -> io::Result<bool> {
        if self.find(remote).is_some() {
            return Ok(false);
        }
        self.local_of(remote)?;
        let attr = self.with_client(|c| c.stat(remote))?;
        Ok(attr.mode & S_IFMT == S_IFDIR)
    }

    /// Copies the folder at `remote` here as it is now, a level at a time
    /// up to the limits: the copy's path, and how much of it there is.
    /// Copies kept the same are left as they are; what is no longer there
    /// goes from here, unless it is one of them.
    pub fn fetch_folder(&self, remote: &[u8]) -> io::Result<(PathBuf, FolderCopy)> {
        let top = self.local_of(remote)?;
        let mut copied = FolderCopy::default();
        let mut bytes = 0u64;
        let mut level = vec![remote.to_vec()];
        while !level.is_empty() {
            let mut next = Vec::new();
            for folder in level {
                let local = self.local_of(&folder)?;
                private_dirs(&local)?;
                // The top must be listed; one below that can't be is left out.
                let entries = match self.with_client(|c| c.list(&folder)) {
                    Ok(entries) => entries,
                    Err(e) if folder == remote => return Err(e),
                    Err(_) => {
                        copied.left_out += 1;
                        continue;
                    }
                };
                let mut names = Vec::new();
                for entry in entries {
                    if entry.name == b"." || entry.name == b".." {
                        continue;
                    }
                    names.push(entry.name.clone());
                    let mut path = folder.clone();
                    if !path.ends_with(b"/") {
                        path.push(b'/');
                    }
                    path.extend_from_slice(&entry.name);
                    match entry.kind {
                        DT_DIR => next.push(path),
                        DT_REG if copied.files >= FOLDER_FILES => copied.left_out += 1,
                        DT_REG if self.find(&path).is_some() => copied.files += 1,
                        DT_REG => {
                            let there = self.with_client(|c| c.stat(&path));
                            match there {
                                Ok(attr) if bytes + attr.size <= FOLDER_BYTES => {
                                    let to = self.local_of(&path)?;
                                    if self.with_client(|c| download(c, &path, &to)).is_ok() {
                                        read_only(&to);
                                        bytes += attr.size;
                                        copied.files += 1;
                                    } else {
                                        copied.left_out += 1;
                                    }
                                }
                                _ => copied.left_out += 1,
                            }
                        }
                        _ => copied.left_out += 1,
                    }
                }
                self.tidy(&local, &names);
            }
            if copied.files >= FOLDER_FILES || bytes >= FOLDER_BYTES {
                copied.left_out += next.len();
                break;
            }
            level = next;
        }
        Ok((top, copied))
    }

    /// Takes from the copy of a folder at `local` what is no longer in it
    /// there (`names`), but not a copy kept the same.
    fn tidy(&self, local: &Path, names: &[Vec<u8>]) {
        let Ok(here) = fs::read_dir(local) else {
            return;
        };
        let kept: Vec<PathBuf> = lock(&self.watched)
            .copies
            .iter()
            .map(|(_, copy)| lock(copy).local.clone())
            .collect();
        for entry in here.flatten() {
            let name = entry.file_name();
            let path = entry.path();
            if names.iter().any(|n| n.as_slice() == name.as_bytes())
                || name.as_bytes().ends_with(b".hof-part")
                || kept.iter().any(|k| k.starts_with(&path))
            {
                continue;
            }
            let _ = match entry.file_type() {
                Ok(t) if t.is_dir() => fs::remove_dir_all(&path),
                _ => fs::remove_file(&path),
            };
        }
    }

    /// Whether `remote` is watched.
    pub fn is_watched(&self, remote: &[u8]) -> bool {
        self.find(remote).is_some()
    }

    fn with_client<T>(
        &self,
        op: impl Fn(&mut Client<UnixStream>) -> io::Result<T>,
    ) -> io::Result<T> {
        self.reach.with_client(op)
    }

    /// Writes a copy saved here back there, or copies again a file changed
    /// there.
    fn sync(&self, w: &mut Watched) -> io::Result<()> {
        let now_here = seen(&w.local)?;
        if now_here != w.here {
            if w.saving != Some(now_here) {
                w.saving = Some(now_here);
                return Ok(());
            }
            let bytes = fs::read(&w.local)?;
            let attr = self.with_client(|c| c.write_file(&w.remote, &bytes))?;
            w.there = there_of(&attr);
            w.here = now_here;
            w.saving = None;
            w.touched = Instant::now();
            return Ok(());
        }
        w.saving = None;
        if w.checked.elapsed() < LOOK_THERE {
            return Ok(());
        }
        w.checked = Instant::now();
        let attr = self.with_client(|c| c.stat(&w.remote))?;
        if there_of(&attr) != w.there {
            w.there = self.with_client(|c| download(c, &w.remote, &w.local))?;
            w.here = seen(&w.local)?;
            w.touched = Instant::now();
        }
        Ok(())
    }
}

/// Makes a copy of a folder's file read-only, so that nothing saved in it
/// is lost unseen when the folder is copied again.
fn read_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o400));
}

/// Copies the file at `remote` to `local`, whole or not at all: what it
/// was there.
fn download(client: &mut Client<UnixStream>, remote: &[u8], local: &Path) -> io::Result<There> {
    let dir = local.parent().ok_or(io::ErrorKind::InvalidInput)?;
    private_dirs(dir)?;
    let mut part_name = std::ffi::OsString::from(".");
    part_name.push(local.file_name().ok_or(io::ErrorKind::InvalidInput)?);
    part_name.push(".hof-part");
    let part = dir.join(part_name);
    let copied = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&part)?;
        let mut out = BufWriter::new(file);
        let attr = client.read_file(remote, &mut out)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        fs::rename(&part, local)?;
        Ok(there_of(&attr))
    })();
    if copied.is_err() {
        let _ = fs::remove_file(&part);
    }
    copied
}

fn watch(mirror: &Weak<Mirror>) {
    let mut failing = false;
    loop {
        thread::sleep(TICK);
        let Some(m) = mirror.upgrade() else { return };
        // Looked at one at a time, the list let go meanwhile.
        let copies: Vec<_> = lock(&m.watched)
            .copies
            .iter()
            .map(|(_, copy)| copy.clone())
            .collect();
        let mut failed = None;
        let mut gone = Vec::new();
        for copy in &copies {
            let mut w = lock(copy);
            if w.touched.elapsed() > FORGET {
                gone.push(copy.clone());
                continue;
            }
            match m.sync(&mut w) {
                Ok(()) => {}
                // The copy went from here: nothing more to keep the same.
                Err(_) if !w.local.exists() => gone.push(copy.clone()),
                Err(e) => {
                    failed.get_or_insert(e);
                }
            }
        }
        match failed {
            Some(e) if !failing => {
                failing = true;
                eprintln!("hof: files on {}: {e}", m.reach.host());
            }
            Some(_) => {}
            None => failing = false,
        }
        let mut watching = lock(&m.watched);
        watching
            .copies
            .retain(|(_, c)| !gone.iter().any(|g| Arc::ptr_eq(c, g)));
        if watching.copies.is_empty() {
            watching.running = false;
            drop(watching);
            m.reach.hang_up();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_stays_in_its_hosts_directory() {
        let m = Mirror::new("/c/hosts/dev".into(), "dev", || {
            Err(io::ErrorKind::NotConnected.into())
        });
        assert_eq!(
            m.local_of(b"/home/me/a.txt").unwrap(),
            Path::new("/c/hosts/dev/home/me/a.txt")
        );
        assert_eq!(
            m.local_of(b"//home/./me//a.txt").unwrap(),
            Path::new("/c/hosts/dev/home/me/a.txt")
        );
        for bad in [&b"/home/../../etc/passwd"[..], b"relative", b"/", b""] {
            assert!(
                m.local_of(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
