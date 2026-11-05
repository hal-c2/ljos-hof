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

/// How often copies here are looked at.
const TICK: Duration = Duration::from_millis(500);
/// How often the files there are.
const LOOK_THERE: Duration = Duration::from_secs(3);
/// How long a copy that hasn't changed is watched.
const FORGET: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after failing to reach the files it tries again.
const RETRY: Duration = Duration::from_secs(5);
/// The most copies watched at once; the oldest goes for a new one.
const MOST: usize = 64;
/// How long one request may take before the channel counts as gone.
const PATIENCE: Duration = Duration::from_secs(30);

type Connect = dyn Fn() -> io::Result<UnixStream> + Send + Sync;

/// One host's copies.
pub struct Mirror {
    dir: PathBuf,
    host: String,
    connect: Box<Connect>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<Client<UnixStream>>,
    watched: Vec<Watched>,
    watching: bool,
    /// Reaching the files failed; not tried again before this.
    retry_at: Option<Instant>,
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
            host: host.to_owned(),
            connect: Box::new(connect),
            state: Mutex::default(),
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

    /// Copies the file at `remote` here, and keeps the two the same from
    /// now on: the copy's path.
    pub fn fetch(self: &Arc<Self>, remote: &[u8]) -> io::Result<PathBuf> {
        let local = self.local_of(remote)?;
        let mut state = lock(&self.state);
        // Asked for now: whatever failed before is tried again at once.
        state.retry_at = None;
        if let Some(i) = state.watched.iter().position(|w| w.remote == remote) {
            let mut watched = state.watched.remove(i);
            // Looked at now, not when the tick comes.
            let now = Instant::now();
            watched.checked = now.checked_sub(LOOK_THERE).unwrap_or(now);
            watched.touched = Instant::now();
            let synced = self.sync(&mut state, &mut watched);
            state.watched.push(watched);
            return synced.map(|()| local);
        }
        let there = self.with_client(&mut state, |c| download(c, remote, &local))?;
        let now = Instant::now();
        if state.watched.len() >= MOST {
            state.watched.remove(0);
        }
        state.watched.push(Watched {
            remote: remote.to_vec(),
            local: local.clone(),
            here: seen(&local)?,
            there,
            saving: None,
            checked: now,
            touched: now,
        });
        if !state.watching {
            let mirror = Arc::downgrade(self);
            let started = thread::Builder::new()
                .name("hof-files-watch".into())
                .spawn(move || watch(&mirror));
            state.watching = started.is_ok();
        }
        Ok(local)
    }

    /// Whether `remote` is watched.
    pub fn is_watched(&self, remote: &[u8]) -> bool {
        lock(&self.state).watched.iter().any(|w| w.remote == remote)
    }

    fn open(&self) -> io::Result<Client<UnixStream>> {
        let socket = (self.connect)()?;
        socket.set_read_timeout(Some(PATIENCE))?;
        socket.set_write_timeout(Some(PATIENCE))?;
        let user = std::env::var("USER").unwrap_or_else(|_| "hof".into());
        Client::new(socket, &user).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "the agent on {} does not share files: it is older than this Hof; \
                         update Hof there",
                        self.host
                    ),
                )
            } else {
                e
            }
        })
    }

    /// Does `op` with the session, opened if there is none; again on a new
    /// one if the one there was turns out to have gone.
    fn with_client<T>(
        &self,
        state: &mut State,
        op: impl Fn(&mut Client<UnixStream>) -> io::Result<T>,
    ) -> io::Result<T> {
        if state.retry_at.is_some_and(|at| Instant::now() < at) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("the files on {} can't be reached just now", self.host),
            ));
        }
        let mut fresh = false;
        loop {
            let client = match &mut state.client {
                Some(client) => client,
                none => {
                    fresh = true;
                    match self.open() {
                        Ok(client) => none.insert(client),
                        Err(e) => {
                            state.retry_at = Some(Instant::now() + RETRY);
                            return Err(e);
                        }
                    }
                }
            };
            let done = op(client);
            if done.is_ok() || !client.is_broken() {
                return done;
            }
            state.client = None;
            if fresh {
                state.retry_at = Some(Instant::now() + RETRY);
                return done;
            }
        }
    }

    /// Writes a copy saved here back there, or copies again a file changed
    /// there.
    fn sync(&self, state: &mut State, w: &mut Watched) -> io::Result<()> {
        let now_here = seen(&w.local)?;
        if now_here != w.here {
            if w.saving != Some(now_here) {
                w.saving = Some(now_here);
                return Ok(());
            }
            let bytes = fs::read(&w.local)?;
            let attr = self.with_client(state, |c| c.write_file(&w.remote, &bytes))?;
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
        let attr = self.with_client(state, |c| c.stat(&w.remote))?;
        if there_of(&attr) != w.there {
            w.there = self.with_client(state, |c| download(c, &w.remote, &w.local))?;
            w.here = seen(&w.local)?;
            w.touched = Instant::now();
        }
        Ok(())
    }
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
        let mut state = lock(&m.state);
        let mut failed = None;
        let mut kept = Vec::new();
        for mut w in std::mem::take(&mut state.watched) {
            if w.touched.elapsed() > FORGET {
                continue;
            }
            match m.sync(&mut state, &mut w) {
                Ok(()) => kept.push(w),
                // The copy went from here: nothing more to keep the same.
                Err(_) if !w.local.exists() => {}
                Err(e) => {
                    failed.get_or_insert(e);
                    kept.push(w);
                }
            }
        }
        state.watched = kept;
        match failed {
            Some(e) if !failing => {
                failing = true;
                eprintln!("hof: files on {}: {e}", m.host);
            }
            Some(_) => {}
            None => failing = false,
        }
        if state.watched.is_empty() {
            state.watching = false;
            state.client = None;
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
