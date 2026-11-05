//! The other machine's files, mounted here: a FUSE file system whose every
//! request is asked of the agent over the files channel, as it comes.
//!
//! Nothing is copied: a folder opened here is the folder there, as it is
//! now, and a file saved in it is saved there. Each file is known by its
//! path there, which is what 9P walks; a file moved through the mount is
//! known by its new path from then on.
//!
//! Every file shows as this user's: what may be done with it is what the
//! user the agent runs as may do there, and the agent says so when it
//! can't. The kernel keeps what it is told of a file for a second, so a
//! change made there shows here within one.
//!
//! When the channel goes, the next request opens another; files open
//! through the mount are opened again on it. While the machine can't be
//! reached, requests fail at once with `EIO` rather than wait.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
    TimeOrNow, WriteFlags,
};

use super::client::Client;
use super::ninep::{self, Attr, SetAttr, open, set};
use super::reach::Reach;

/// How long the kernel keeps what it is told of a file.
const TTL: Duration = Duration::from_secs(1);

/// A host's files, mounted at a directory here while this lives.
pub struct Mount {
    at: PathBuf,
    /// The directory there that `at` shows.
    root: PathBuf,
    /// `None` for a mount another Hof made, and unmounts.
    session: Option<BackgroundSession>,
}

impl Mount {
    /// Mounts `host`'s files at `at`, over the channels `connect` opens
    /// to its agent's files service. A mount left there by a Hof that
    /// ended goes first; one a running Hof has there already is used as
    /// it is.
    pub fn new(
        at: PathBuf,
        host: &str,
        connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::of(at, host, b"/", connect)
    }

    /// Mounts the directory `root` on `host`, and only what is in it, at
    /// `at`: `..` there goes no higher than `root`.
    pub fn of(
        at: PathBuf,
        host: &str,
        root: &[u8],
        connect: impl Fn() -> io::Result<UnixStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let root = PathBuf::from(OsStr::from_bytes(root));
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the folder to mount is not a whole path",
            ));
        }
        match fs::metadata(&at) {
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::NOTCONN.raw_os_error()) => {
                unmount(&at)?;
            }
            Ok(meta) if is_mounted(&at, &meta) => {
                return Ok(Self {
                    at,
                    root,
                    session: None,
                });
            }
            _ => {}
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&at)?;
        let files = Files {
            reach: Reach::new(host, root.as_os_str().as_bytes(), connect),
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
            nodes: Mutex::new(Nodes::new()),
            handles: Mutex::default(),
        };
        let mut config = Config::default();
        config.mount_options = vec![
            MountOption::FSName(format!("hof:{host}")),
            MountOption::Subtype("hof".into()),
            MountOption::NoDev,
            MountOption::NoSuid,
        ];
        let session = fuser::spawn_mount(files, &at, &config)?;
        Ok(Self {
            at,
            root,
            session: Some(session),
        })
    }

    /// Where it is mounted.
    pub fn path(&self) -> &Path {
        &self.at
    }

    /// Where `remote`, a whole path there, is here.
    pub fn local_of(&self, remote: &[u8]) -> io::Result<PathBuf> {
        let path = Path::new(OsStr::from_bytes(remote));
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a file's whole path",
            ));
        }
        let path = path
            .strip_prefix(&self.root)
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "not in the folder mounted"))?;
        let mut local = self.at.clone();
        for part in path.components() {
            match part {
                Component::Normal(name) => local.push(name),
                Component::RootDir | Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a path with .. in it",
                    ));
                }
            }
        }
        Ok(local)
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            // Taken away now, though a program may still be in it: it
            // keeps what it has open until it lets go.
            let _ = unmount(&self.at);
            drop(session);
        }
    }
}

/// Whether `at` is where a file system other than its parent's is.
fn is_mounted(at: &Path, meta: &fs::Metadata) -> bool {
    at.parent()
        .and_then(|up| fs::metadata(up).ok())
        .is_some_and(|up| up.dev() != meta.dev())
}

/// Takes the mount at `at` away, though it is in use or its server gone.
fn unmount(at: &Path) -> io::Result<()> {
    let run = |tool: &str| {
        Command::new(tool)
            .args(["-u", "-z", "-q"])
            .arg(at)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    let status = match run("fusermount3") {
        Err(e) if e.kind() == io::ErrorKind::NotFound => run("fusermount")?,
        other => other?,
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "could not unmount {}: {status}",
            at.display()
        )))
    }
}

/// The file system: each file by its path there.
struct Files {
    reach: Reach,
    /// This user, whose every file shows as.
    uid: u32,
    gid: u32,
    nodes: Mutex<Nodes>,
    handles: Mutex<Handles>,
}

/// The files the kernel knows, by the numbers it knows them by.
struct Nodes {
    by_ino: HashMap<u64, Node>,
    by_path: HashMap<Vec<u8>, u64>,
    next: u64,
}

struct Node {
    path: Vec<u8>,
    /// How many times the kernel was told of it and hasn't forgotten.
    lookups: u64,
}

impl Nodes {
    fn new() -> Self {
        let root = INodeNo::ROOT.0;
        Self {
            by_ino: HashMap::from([(
                root,
                Node {
                    path: b"/".to_vec(),
                    lookups: 1,
                },
            )]),
            by_path: HashMap::from([(b"/".to_vec(), root)]),
            next: root + 1,
        }
    }

    fn path(&self, ino: u64) -> Option<Vec<u8>> {
        self.by_ino.get(&ino).map(|n| n.path.clone())
    }

    /// The kernel is told of the file at `path` once more: its number.
    fn looked_up(&mut self, path: Vec<u8>) -> u64 {
        if let Some(&ino) = self.by_path.get(&path)
            && let Some(node) = self.by_ino.get_mut(&ino)
        {
            node.lookups += 1;
            return ino;
        }
        let ino = self.next;
        self.next += 1;
        self.by_path.insert(path.clone(), ino);
        self.by_ino.insert(ino, Node { path, lookups: 1 });
        ino
    }

    fn forget(&mut self, ino: u64, times: u64) {
        if ino == INodeNo::ROOT.0 {
            return;
        }
        let Some(node) = self.by_ino.get_mut(&ino) else {
            return;
        };
        node.lookups = node.lookups.saturating_sub(times);
        if node.lookups == 0 {
            let node = self.by_ino.remove(&ino).expect("just found");
            if self.by_path.get(&node.path) == Some(&ino) {
                self.by_path.remove(&node.path);
            }
        }
    }

    /// The file at `path` went: a new one there is another file.
    fn gone(&mut self, path: &[u8]) {
        self.by_path.remove(path);
    }

    /// What was at `from`, and in it, is at `to` now.
    fn moved(&mut self, from: &[u8], to: &[u8]) {
        self.gone(to);
        let within = |path: &[u8]| {
            path == from || (path.starts_with(from) && path.get(from.len()) == Some(&b'/'))
        };
        let moving: Vec<(Vec<u8>, u64)> = self
            .by_path
            .iter()
            .filter(|(path, _)| within(path))
            .map(|(path, &ino)| (path.clone(), ino))
            .collect();
        for (path, ino) in moving {
            self.by_path.remove(&path);
            let new = [to, &path[from.len()..]].concat();
            if let Some(node) = self.by_ino.get_mut(&ino) {
                node.path.clone_from(&new);
            }
            self.by_path.insert(new, ino);
        }
    }
}

#[derive(Default)]
struct Handles {
    open: HashMap<u64, Opened>,
    next: u64,
}

enum Opened {
    File(OpenFile),
    /// Its entries, as they were when it was opened.
    Dir(Vec<Listed>),
}

#[derive(Clone, Copy)]
struct OpenFile {
    ino: u64,
    /// As it is opened again on a new session.
    flags: u32,
    fid: u32,
    unit: u32,
    /// The session the fid is of.
    session: u64,
}

struct Listed {
    ino: u64,
    kind: FileType,
    name: Vec<u8>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `name` in the directory `dir`, both paths there.
fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    if dir.ends_with(b"/") {
        [dir, name].concat()
    } else {
        [dir, b"/", name].concat()
    }
}

/// What the kernel is told of `e`: what the other machine said, or that
/// its files could not be reached.
fn errno(e: &io::Error) -> Errno {
    if let Some(n) = ninep::errno_there(e) {
        return Errno::from_i32(n as i32);
    }
    match e.kind() {
        io::ErrorKind::NotFound => Errno::ENOENT,
        io::ErrorKind::IsADirectory => Errno::EISDIR,
        io::ErrorKind::NotADirectory => Errno::ENOTDIR,
        _ => Errno::EIO,
    }
}

fn kind_of(mode: u32) -> FileType {
    match mode & 0o170_000 {
        0o040_000 => FileType::Directory,
        0o120_000 => FileType::Symlink,
        0o010_000 => FileType::NamedPipe,
        0o020_000 => FileType::CharDevice,
        0o060_000 => FileType::BlockDevice,
        0o140_000 => FileType::Socket,
        _ => FileType::RegularFile,
    }
}

/// The kind a directory's entry says, as Linux's `d_type` numbers it.
fn kind_of_entry(kind: u8) -> FileType {
    match kind {
        1 => FileType::NamedPipe,
        2 => FileType::CharDevice,
        4 => FileType::Directory,
        6 => FileType::BlockDevice,
        10 => FileType::Symlink,
        12 => FileType::Socket,
        _ => FileType::RegularFile,
    }
}

fn time_of(t: ninep::Time) -> SystemTime {
    UNIX_EPOCH + Duration::new(t.0, t.1.min(999_999_999) as u32)
}

/// What `when` is, as `Tsetattr` takes it: the time, and whether it is
/// given rather than now.
fn set_time(when: TimeOrNow) -> (ninep::Time, bool) {
    match when {
        TimeOrNow::Now => ((0, 0), false),
        TimeOrNow::SpecificTime(t) => {
            let since = t.duration_since(UNIX_EPOCH).unwrap_or_default();
            ((since.as_secs(), u64::from(since.subsec_nanos())), true)
        }
    }
}

/// The open flags asked to be passed on: how it is opened, and whether
/// it is cut short or added to.
fn flags_of(flags: i32) -> u32 {
    (flags as u32) & (open::ACCMODE | open::APPEND | open::TRUNC)
}

impl Files {
    fn path_of(&self, ino: INodeNo) -> Result<Vec<u8>, Errno> {
        lock(&self.nodes).path(ino.0).ok_or(Errno::ENOENT)
    }

    fn attr(&self, ino: u64, a: &Attr) -> FileAttr {
        FileAttr {
            ino: INodeNo(ino),
            size: a.size,
            blocks: a.blocks,
            atime: time_of(a.atime),
            mtime: time_of(a.mtime),
            ctime: time_of(a.ctime),
            crtime: time_of(a.ctime),
            kind: kind_of(a.mode),
            perm: (a.mode & 0o7777) as u16,
            nlink: a.nlink.try_into().unwrap_or(u32::MAX),
            uid: self.uid,
            gid: self.gid,
            rdev: a.rdev as u32,
            blksize: if a.blksize == 0 {
                4096
            } else {
                a.blksize.try_into().unwrap_or(4096)
            },
            flags: 0,
        }
    }

    /// The kernel is told of the file at `path`, as `attr` says it is.
    fn entry(&self, path: Vec<u8>, attr: &Attr, reply: ReplyEntry) {
        let ino = lock(&self.nodes).looked_up(path);
        reply.entry(&TTL, &self.attr(ino, attr), Generation(0));
    }

    fn stat(&self, path: &[u8]) -> Result<Attr, Errno> {
        self.reach
            .with_client(|c| c.stat(path))
            .map_err(|e| errno(&e))
    }

    fn handle(&self, opened: Opened) -> u64 {
        let mut handles = lock(&self.handles);
        handles.next += 1;
        let fh = handles.next;
        handles.open.insert(fh, opened);
        fh
    }

    fn open_file(&self, fh: FileHandle) -> Result<OpenFile, Errno> {
        match lock(&self.handles).open.get(&fh.0) {
            Some(Opened::File(file)) => Ok(*file),
            _ => Err(Errno::EBADF),
        }
    }

    /// Does `op` with the open file `fh`'s fid, opening it again if it was
    /// of a session that has gone; again on a new session, as
    /// [`Reach::with_session`] says, only if `again`.
    fn with_file<T>(
        &self,
        fh: FileHandle,
        again: bool,
        op: impl Fn(&mut Client<UnixStream>, u32, u32) -> io::Result<T>,
    ) -> Result<T, Errno> {
        let file = self.open_file(fh)?;
        let path = lock(&self.nodes).path(file.ino).ok_or(Errno::ENOENT)?;
        let reopened = std::cell::Cell::new(None::<OpenFile>);
        let done = self.reach.with_session(again, |c, session| {
            let now = match reopened.get() {
                Some(f) if f.session == session => f,
                _ if file.session == session => file,
                _ => {
                    // What was asked to be cut short was, the first time.
                    let flags = file.flags & !open::TRUNC;
                    let (fid, unit) = c.open(&path, flags)?;
                    let f = OpenFile {
                        fid,
                        unit,
                        session,
                        flags,
                        ..file
                    };
                    reopened.set(Some(f));
                    f
                }
            };
            op(c, now.fid, now.unit)
        });
        if let Some(f) = reopened.get()
            && let Some(Opened::File(held)) = lock(&self.handles).open.get_mut(&fh.0)
        {
            *held = f;
        }
        done.map_err(|e| errno(&e))
    }
}

impl Filesystem for Files {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let found = self.path_of(parent).and_then(|dir| {
            let path = join(&dir, name.as_bytes());
            self.stat(&path).map(|attr| (path, attr))
        });
        match found {
            Ok((path, attr)) => self.entry(path, &attr, reply),
            Err(e) => reply.error(e),
        }
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        lock(&self.nodes).forget(ino.0, nlookup);
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let open = fh.filter(|&fh| self.open_file(fh).is_ok());
        let attr = match open {
            // Still there to ask of if it went from its directory.
            Some(fh) => self.with_file(fh, true, |c, fid, _| c.getattr(fid)),
            None => self.path_of(ino).and_then(|path| self.stat(&path)),
        };
        match attr {
            Ok(attr) => reply.attr(&TTL, &self.attr(ino.0, &attr)),
            Err(e) => reply.error(e),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        // Whose a file is here says nothing of whose it is there.
        if uid.is_some_and(|u| u != self.uid) || gid.is_some_and(|g| g != self.gid) {
            reply.error(Errno::EPERM);
            return;
        }
        let mut attr = SetAttr::default();
        if let Some(mode) = mode {
            attr.valid |= set::MODE;
            attr.mode = mode;
        }
        if let Some(size) = size {
            attr.valid |= set::SIZE;
            attr.size = size;
        }
        if let Some(when) = atime {
            let (t, given) = set_time(when);
            attr.valid |= set::ATIME | if given { set::ATIME_SET } else { 0 };
            attr.atime = t;
        }
        if let Some(when) = mtime {
            let (t, given) = set_time(when);
            attr.valid |= set::MTIME | if given { set::MTIME_SET } else { 0 };
            attr.mtime = t;
        }
        let done = self.path_of(ino).and_then(|path| {
            if attr.valid != 0 {
                self.reach
                    .with_client(|c| c.setattr(&path, attr))
                    .map_err(|e| errno(&e))?;
            }
            self.stat(&path)
        });
        match done {
            Ok(attr) => reply.attr(&TTL, &self.attr(ino.0, &attr)),
            Err(e) => reply.error(e),
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        let target = self.path_of(ino).and_then(|path| {
            self.reach
                .with_client(|c| c.readlink(&path))
                .map_err(|e| errno(&e))
        });
        match target {
            Ok(target) => reply.data(&target),
            Err(e) => reply.error(e),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let made = self.path_of(parent).and_then(|dir| {
            let name = name.as_bytes();
            self.reach
                .with_session(false, |c, _| c.mkdir(&dir, name, mode & !umask & 0o7777))
                .map_err(|e| errno(&e))?;
            let path = join(&dir, name);
            self.stat(&path).map(|attr| (path, attr))
        });
        match made {
            Ok((path, attr)) => self.entry(path, &attr, reply),
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.remove(parent, name, false, reply);
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.remove(parent, name, true, reply);
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        if !flags.is_empty() {
            // Neither swapping nor refusing to replace can be asked of 9P.
            reply.error(Errno::EINVAL);
            return;
        }
        let moved = self.path_of(parent).and_then(|from| {
            let to = self.path_of(newparent)?;
            let (name, newname) = (name.as_bytes(), newname.as_bytes());
            self.reach
                .with_session(false, |c, _| c.rename(&from, name, &to, newname))
                .map_err(|e| errno(&e))?;
            Ok((join(&from, name), join(&to, newname)))
        });
        match moved {
            Ok((from, to)) => {
                lock(&self.nodes).moved(&from, &to);
                reply.ok();
            }
            Err(e) => reply.error(e),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let flags = flags_of(flags.0);
        let opened = self.path_of(ino).and_then(|path| {
            self.reach
                .with_session(true, |c, session| {
                    c.open(&path, flags).map(|(fid, unit)| (fid, unit, session))
                })
                .map_err(|e| errno(&e))
        });
        match opened {
            Ok((fid, unit, session)) => {
                let fh = self.handle(Opened::File(OpenFile {
                    ino: ino.0,
                    flags,
                    fid,
                    unit,
                    session,
                }));
                reply.opened(FileHandle(fh), FopenFlags::empty());
            }
            Err(e) => reply.error(e),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let read = self.with_file(fh, true, |c, fid, unit| {
            let mut data = Vec::with_capacity(size as usize);
            while data.len() < size as usize {
                let want = (size - data.len() as u32).min(unit);
                let got = c.read_at(fid, offset + data.len() as u64, want)?;
                if got.is_empty() {
                    break;
                }
                data.extend_from_slice(&got);
            }
            Ok(data)
        });
        match read {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(e),
        }
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let written = self.with_file(fh, false, |c, fid, unit| {
            c.write_at(fid, unit, offset, data)
        });
        match written {
            Ok(()) => reply.written(data.len() as u32),
            Err(e) => reply.error(e),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        // Every write was there before it was answered.
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.with_file(fh, false, |c, fid, _| c.fsync(fid, datasync)) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let opened = lock(&self.handles).open.remove(&fh.0);
        if let Some(Opened::File(file)) = opened {
            let _ = self.reach.with_session(false, |c, session| {
                if session == file.session {
                    c.clunk(file.fid)
                } else {
                    Ok(())
                }
            });
        }
        reply.ok();
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let mut asked = flags_of(flags) | (flags as u32 & open::EXCL);
        if asked & open::ACCMODE == open::RDONLY {
            // Made to be written, though not by this caller.
            asked |= open::RDWR;
        }
        let made = self.path_of(parent).and_then(|dir| {
            let name = name.as_bytes();
            let made = self.reach.with_session(false, |c, session| {
                let (fid, unit) = c.create(&dir, name, asked, mode & !umask & 0o7777)?;
                match c.getattr(fid) {
                    Ok(attr) => Ok((fid, unit, session, attr)),
                    Err(e) => {
                        let _ = c.clunk(fid);
                        Err(e)
                    }
                }
            });
            made.map(|m| (join(&dir, name), m)).map_err(|e| errno(&e))
        });
        match made {
            Ok((path, (fid, unit, session, attr))) => {
                let ino = lock(&self.nodes).looked_up(path);
                let fh = self.handle(Opened::File(OpenFile {
                    ino,
                    flags: asked & !(open::TRUNC | open::EXCL),
                    fid,
                    unit,
                    session,
                }));
                reply.created(
                    &TTL,
                    &self.attr(ino, &attr),
                    Generation(0),
                    FileHandle(fh),
                    FopenFlags::empty(),
                );
            }
            Err(e) => reply.error(e),
        }
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let listed = self.path_of(ino).and_then(|path| {
            let entries = self
                .reach
                .with_client(|c| c.list(&path))
                .map_err(|e| errno(&e))?;
            Ok((path, entries))
        });
        let (path, entries) = match listed {
            Ok(listed) => listed,
            Err(e) => {
                reply.error(e);
                return;
            }
        };
        let nodes = lock(&self.nodes);
        let up = match path.iter().rposition(|&b| b == b'/') {
            Some(0) | None => b"/".as_slice(),
            Some(cut) => &path[..cut],
        };
        let up = nodes.by_path.get(up).copied().unwrap_or(INodeNo::ROOT.0);
        let mut all = vec![
            Listed {
                ino: ino.0,
                kind: FileType::Directory,
                name: b".".to_vec(),
            },
            Listed {
                ino: up,
                kind: FileType::Directory,
                name: b"..".to_vec(),
            },
        ];
        all.extend(entries.into_iter().map(|e| {
            Listed {
                // The kernel goes by what a lookup says, not this; only a
                // program listing it sees it.
                ino: nodes
                    .by_path
                    .get(&join(&path, &e.name))
                    .copied()
                    .unwrap_or(e.qid.path.max(1)),
                kind: kind_of_entry(e.kind),
                name: e.name,
            }
        }));
        drop(nodes);
        let fh = self.handle(Opened::Dir(all));
        reply.opened(FileHandle(fh), FopenFlags::empty());
    }

    fn readdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let handles = lock(&self.handles);
        let Some(Opened::Dir(all)) = handles.open.get(&fh.0) else {
            drop(handles);
            reply.error(Errno::EBADF);
            return;
        };
        for (i, e) in all.iter().enumerate().skip(offset as usize) {
            let next = i as u64 + 1;
            if reply.add(INodeNo(e.ino), next, e.kind, OsStr::from_bytes(&e.name)) {
                break;
            }
        }
        drop(handles);
        reply.ok();
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        lock(&self.handles).open.remove(&fh.0);
        reply.ok();
    }

    fn statfs(&self, _req: &Request, ino: INodeNo, reply: ReplyStatfs) {
        let said = self.path_of(ino).and_then(|path| {
            self.reach
                .with_client(|c| c.statfs(&path))
                .map_err(|e| errno(&e))
        });
        match said {
            Ok(s) => reply.statfs(
                s.blocks, s.bfree, s.bavail, s.files, s.ffree, s.bsize, s.namelen, s.bsize,
            ),
            Err(e) => reply.error(e),
        }
    }
}

impl Files {
    fn remove(&self, parent: INodeNo, name: &OsStr, directory: bool, reply: ReplyEmpty) {
        let removed = self.path_of(parent).and_then(|dir| {
            let name = name.as_bytes();
            self.reach
                .with_session(false, |c, _| c.remove(&dir, name, directory))
                .map_err(|e| errno(&e))?;
            Ok(join(&dir, name))
        });
        match removed {
            Ok(path) => {
                lock(&self.nodes).gone(&path);
                reply.ok();
            }
            Err(e) => reply.error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_moved_is_known_at_its_new_path_and_so_is_what_is_in_it() {
        let mut nodes = Nodes::new();
        let dir = nodes.looked_up(b"/a".to_vec());
        let file = nodes.looked_up(b"/a/f".to_vec());
        let near = nodes.looked_up(b"/ab".to_vec());
        nodes.moved(b"/a", b"/b");
        assert_eq!(nodes.path(dir).unwrap(), b"/b");
        assert_eq!(nodes.path(file).unwrap(), b"/b/f");
        assert_eq!(nodes.path(near).unwrap(), b"/ab", "not in it");
        assert_eq!(nodes.looked_up(b"/b/f".to_vec()), file);
        assert!(!nodes.by_path.contains_key(b"/a/f".as_slice()));
    }

    #[test]
    fn a_file_forgotten_as_often_as_it_was_told_of_is_gone() {
        let mut nodes = Nodes::new();
        let a = nodes.looked_up(b"/a".to_vec());
        assert_eq!(nodes.looked_up(b"/a".to_vec()), a);
        nodes.forget(a, 1);
        assert!(nodes.path(a).is_some());
        nodes.forget(a, 1);
        assert!(nodes.path(a).is_none());
        nodes.forget(INodeNo::ROOT.0, 10);
        assert_eq!(nodes.path(INodeNo::ROOT.0).unwrap(), b"/");
    }

    #[test]
    fn a_name_is_joined_to_its_directory() {
        assert_eq!(join(b"/", b"a"), b"/a");
        assert_eq!(join(b"/a", b"b"), b"/a/b");
    }
}
