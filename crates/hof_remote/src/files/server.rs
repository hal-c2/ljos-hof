//! The agent's end of the files channel: a 9P2000.L server of this
//! machine's files, as the user running the agent may reach them.
//!
//! It is the user's own shell's reach and no more: the agent runs as them,
//! over their own login. Requests are answered in turn, as they come.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::ninep::{
    self, Attr, Entry, GETATTR_BASIC, IOHDRSZ, QT_DIR, QT_FILE, QT_SYMLINK, Qid, Reply, Request,
    SetAttr, StatFs, errno, open, set,
};

/// The most a message may be, whatever the client asks.
pub const MAX_MSIZE: u32 = 1 << 20;
/// What a server starts with, until `Tversion` says otherwise.
const FIRST_MSIZE: u32 = 8192;
/// A client may not make this process hold fids without end.
const MAX_FIDS: usize = 4096;

/// Serves the files over `socket` until it closes.
pub fn serve(mut socket: impl Read + Write) -> io::Result<()> {
    let mut server = Server::default();
    loop {
        let frame = match ninep::read_frame(&mut socket, server.msize) {
            Ok(frame) => frame,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        match Request::decode(&frame) {
            Ok((tag, request)) => server.answer(request).encode(tag, &mut out),
            // Its tag can still be read, to say so.
            Err(_) => {
                let tag = u16::from_le_bytes([frame[5], frame[6]]);
                Reply::Error(errno::EPROTO).encode(tag, &mut out);
            }
        }
        socket.write_all(&out)?;
        socket.flush()?;
    }
}

/// What a fid stands for.
struct Fid {
    path: PathBuf,
    open: Option<Opened>,
}

enum Opened {
    File(File),
    /// The entries, read when it was opened.
    Dir(Vec<Entry>),
}

struct Server {
    msize: u32,
    fids: HashMap<u32, Fid>,
    /// Where the session was attached: `..` goes no higher.
    root: PathBuf,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            msize: FIRST_MSIZE,
            fids: HashMap::new(),
            root: PathBuf::from("/"),
        }
    }
}

type Answer = Result<Reply, u32>;

fn err(e: &io::Error) -> u32 {
    ninep::errno_of(e)
}

impl Server {
    fn answer(&mut self, request: Request) -> Reply {
        let answered = match request {
            Request::Version { msize, version } => Ok(self.version(msize, &version)),
            Request::Attach { fid, aname, .. } => self.attach(fid, &aname),
            Request::Walk { fid, newfid, names } => self.walk(fid, newfid, &names),
            Request::GetAttr { fid, .. } => self.getattr(fid),
            Request::SetAttr { fid, attr } => self.setattr(fid, &attr),
            Request::Lopen { fid, flags } => self.lopen(fid, flags),
            Request::Lcreate {
                fid,
                name,
                flags,
                mode,
                ..
            } => self.lcreate(fid, &name, flags, mode),
            Request::Read { fid, offset, count } => self.read(fid, offset, count),
            Request::Write { fid, offset, data } => self.write(fid, offset, &data),
            Request::Clunk { fid } => self
                .fids
                .remove(&fid)
                .map(|_| Reply::Clunk)
                .ok_or(errno::EBADF),
            Request::Readdir { fid, offset, count } => self.readdir(fid, offset, count),
            Request::Mkdir {
                dfid, name, mode, ..
            } => self.mkdir(dfid, &name, mode),
            Request::Unlinkat { dfid, name, flags } => self.unlinkat(dfid, &name, flags),
            Request::Renameat {
                olddfid,
                oldname,
                newdfid,
                newname,
            } => self.renameat(olddfid, &oldname, newdfid, &newname),
            Request::Readlink { fid } => self.path(fid).and_then(|p| {
                fs::read_link(p)
                    .map(|t| Reply::Readlink(t.as_os_str().as_bytes().to_vec()))
                    .map_err(|e| err(&e))
            }),
            Request::Fsync { fid, datasync } => self.fsync(fid, datasync != 0),
            Request::Statfs { fid } => self.path(fid).map(|_| {
                Reply::Statfs(StatFs {
                    bsize: 4096,
                    namelen: 255,
                    ..StatFs::default()
                })
            }),
            // Each is answered before the next is read: none is left.
            Request::Flush { .. } => Ok(Reply::Flush),
            Request::Unknown(_) => Err(errno::ENOSYS),
        };
        answered.unwrap_or_else(Reply::Error)
    }

    fn version(&mut self, msize: u32, version: &str) -> Reply {
        // A new session: what the last one had goes.
        self.fids.clear();
        self.msize = msize.clamp(FIRST_MSIZE, MAX_MSIZE);
        let version = if version.starts_with(ninep::VERSION) {
            ninep::VERSION
        } else {
            "unknown"
        };
        Reply::Version {
            msize: self.msize,
            version: version.into(),
        }
    }

    fn fid(&self, fid: u32) -> Result<&Fid, u32> {
        self.fids.get(&fid).ok_or(errno::EBADF)
    }

    fn path(&self, fid: u32) -> Result<&Path, u32> {
        Ok(&self.fid(fid)?.path)
    }

    fn add(&mut self, fid: u32, path: PathBuf) -> Result<(), u32> {
        if self.fids.contains_key(&fid) {
            return Err(errno::EBADF);
        }
        if self.fids.len() >= MAX_FIDS {
            return Err(errno::ENOSPC);
        }
        self.fids.insert(fid, Fid { path, open: None });
        Ok(())
    }

    /// The root is `aname` when it is a whole path, else `/`. It must be
    /// a directory; walks go no higher than it.
    fn attach(&mut self, fid: u32, aname: &[u8]) -> Answer {
        let root = Path::new(OsStr::from_bytes(aname));
        let root = if root.is_absolute() {
            fs::canonicalize(root).map_err(|e| err(&e))?
        } else {
            PathBuf::from("/")
        };
        let qid = qid_of(&root).map_err(|e| err(&e))?;
        if qid.kind & QT_DIR == 0 {
            return Err(errno::ENOTDIR);
        }
        self.add(fid, root.clone())?;
        self.root = root;
        Ok(Reply::Attach(qid))
    }

    fn walk(&mut self, fid: u32, newfid: u32, names: &[Vec<u8>]) -> Answer {
        let from = self.path(fid)?.to_path_buf();
        if newfid != fid && self.fids.contains_key(&newfid) {
            return Err(errno::EBADF);
        }
        let mut path = from;
        let mut qids = Vec::new();
        for name in names {
            let Some(next) = step(&self.root, &path, name) else {
                if qids.is_empty() {
                    return Err(errno::ENOENT);
                }
                break;
            };
            match qid_of(&next) {
                Ok(qid) => qids.push(qid),
                Err(e) if qids.is_empty() => return Err(err(&e)),
                // As far as it went: the client sees where it stopped.
                Err(_) => break,
            }
            path = next;
        }
        if qids.len() == names.len() {
            if newfid == fid {
                self.fids.insert(fid, Fid { path, open: None });
            } else {
                self.add(newfid, path)?;
            }
        }
        Ok(Reply::Walk(qids))
    }

    fn getattr(&self, fid: u32) -> Answer {
        let fid = self.fid(fid)?;
        let meta = match &fid.open {
            Some(Opened::File(file)) => file.metadata(),
            _ => fs::symlink_metadata(&fid.path),
        }
        .map_err(|e| err(&e))?;
        Ok(Reply::GetAttr(Box::new(attr_of(&meta))))
    }

    fn setattr(&self, fid: u32, attr: &SetAttr) -> Answer {
        let path = self.path(fid)?;
        let mapped = |r: io::Result<()>| r.map_err(|e| err(&e));
        if attr.valid & set::MODE != 0 {
            mapped(fs::set_permissions(
                path,
                fs::Permissions::from_mode(attr.mode & 0o7777),
            ))?;
        }
        if attr.valid & (set::UID | set::GID) != 0 {
            let uid = (attr.valid & set::UID != 0).then_some(attr.uid);
            let gid = (attr.valid & set::GID != 0).then_some(attr.gid);
            mapped(std::os::unix::fs::chown(path, uid, gid))?;
        }
        if attr.valid & set::SIZE != 0 {
            let file = OpenOptions::new().write(true).open(path);
            mapped(file.and_then(|f| f.set_len(attr.size)))?;
        }
        if attr.valid & (set::ATIME | set::MTIME) != 0 {
            let now = SystemTime::now();
            let at = |given: bool, t: ninep::Time| {
                if given {
                    UNIX_EPOCH + Duration::new(t.0, t.1.min(999_999_999) as u32)
                } else {
                    now
                }
            };
            let mut times = fs::FileTimes::new();
            if attr.valid & set::ATIME != 0 {
                times = times.set_accessed(at(attr.valid & set::ATIME_SET != 0, attr.atime));
            }
            if attr.valid & set::MTIME != 0 {
                times = times.set_modified(at(attr.valid & set::MTIME_SET != 0, attr.mtime));
            }
            let file = File::open(path);
            mapped(file.and_then(|f| f.set_times(times)))?;
        }
        Ok(Reply::SetAttr)
    }

    fn lopen(&mut self, fid: u32, flags: u32) -> Answer {
        let iounit = self.msize - IOHDRSZ;
        let entry = self.fids.get_mut(&fid).ok_or(errno::EBADF)?;
        if entry.open.is_some() {
            return Err(errno::EBADF);
        }
        let meta = fs::metadata(&entry.path).map_err(|e| err(&e))?;
        let qid = qid_from(&meta, false);
        entry.open = Some(if meta.is_dir() {
            Opened::Dir(entries_of(&entry.path).map_err(|e| err(&e))?)
        } else {
            Opened::File(options_of(flags).open(&entry.path).map_err(|e| err(&e))?)
        });
        Ok(Reply::Lopen { qid, iounit })
    }

    fn lcreate(&mut self, fid: u32, name: &[u8], flags: u32, mode: u32) -> Answer {
        let iounit = self.msize - IOHDRSZ;
        let entry = self.fids.get_mut(&fid).ok_or(errno::EBADF)?;
        if entry.open.is_some() {
            return Err(errno::EBADF);
        }
        let path = child(&entry.path, name).ok_or(errno::EINVAL)?;
        let file = options_of(flags | open::CREAT)
            .mode(mode & 0o7777)
            .open(&path)
            .map_err(|e| err(&e))?;
        let qid = file
            .metadata()
            .map(|m| qid_from(&m, false))
            .map_err(|e| err(&e))?;
        *entry = Fid {
            path,
            open: Some(Opened::File(file)),
        };
        Ok(Reply::Lcreate { qid, iounit })
    }

    fn opened(&self, fid: u32) -> Result<&Opened, u32> {
        self.fid(fid)?.open.as_ref().ok_or(errno::EBADF)
    }

    fn read(&self, fid: u32, offset: u64, count: u32) -> Answer {
        let Opened::File(file) = self.opened(fid)? else {
            return Err(errno::EISDIR);
        };
        let mut data = vec![0; count.min(self.msize - IOHDRSZ) as usize];
        let n = loop {
            match file.read_at(&mut data, offset) {
                Ok(n) => break n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(err(&e)),
            }
        };
        data.truncate(n);
        Ok(Reply::Read(data))
    }

    fn write(&self, fid: u32, offset: u64, data: &[u8]) -> Answer {
        let Opened::File(file) = self.opened(fid)? else {
            return Err(errno::EISDIR);
        };
        file.write_all_at(data, offset).map_err(|e| err(&e))?;
        Ok(Reply::Write(data.len() as u32))
    }

    fn readdir(&self, fid: u32, offset: u64, count: u32) -> Answer {
        let Opened::Dir(entries) = self.opened(fid)? else {
            return Err(errno::ENOTDIR);
        };
        let room = count.min(self.msize - IOHDRSZ) as usize;
        let mut data = Vec::new();
        for entry in entries.iter().skip(offset as usize) {
            if data.len() + entry.encoded_len() > room {
                break;
            }
            entry.encode(&mut data);
        }
        Ok(Reply::Readdir(data))
    }

    fn mkdir(&self, dfid: u32, name: &[u8], mode: u32) -> Answer {
        let path = child(self.path(dfid)?, name).ok_or(errno::EINVAL)?;
        fs::DirBuilder::new()
            .mode(mode & 0o7777)
            .create(&path)
            .map_err(|e| err(&e))?;
        qid_of(&path).map(Reply::Mkdir).map_err(|e| err(&e))
    }

    fn unlinkat(&self, dfid: u32, name: &[u8], flags: u32) -> Answer {
        let path = child(self.path(dfid)?, name).ok_or(errno::EINVAL)?;
        if flags & ninep::AT_REMOVEDIR != 0 {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        }
        .map_err(|e| err(&e))?;
        Ok(Reply::Unlinkat)
    }

    fn renameat(&self, olddfid: u32, oldname: &[u8], newdfid: u32, newname: &[u8]) -> Answer {
        let from = child(self.path(olddfid)?, oldname).ok_or(errno::EINVAL)?;
        let to = child(self.path(newdfid)?, newname).ok_or(errno::EINVAL)?;
        fs::rename(from, to).map_err(|e| err(&e))?;
        Ok(Reply::Renameat)
    }

    fn fsync(&self, fid: u32, data_only: bool) -> Answer {
        let Opened::File(file) = self.opened(fid)? else {
            return Ok(Reply::Fsync);
        };
        if data_only {
            file.sync_data()
        } else {
            file.sync_all()
        }
        .map_err(|e| err(&e))?;
        Ok(Reply::Fsync)
    }
}

/// `name` in the directory `dir`: one name, not a path, and not `.` or
/// `..`.
fn child(dir: &Path, name: &[u8]) -> Option<PathBuf> {
    let name = Path::new(OsStr::from_bytes(name));
    let mut parts = name.components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(part)), None) if !name.as_os_str().as_bytes().contains(&b'/') => {
            Some(dir.join(part))
        }
        _ => None,
    }
}

/// Walking `name` from `path`: into a child, or `..` up (and not above
/// `root`, where the session was attached), or `.` where it is.
fn step(root: &Path, path: &Path, name: &[u8]) -> Option<PathBuf> {
    match name {
        b"." => Some(path.to_path_buf()),
        b".." if path == root => Some(path.to_path_buf()),
        b".." => Some(path.parent().unwrap_or(path).to_path_buf()),
        _ => child(path, name),
    }
}

fn options_of(flags: u32) -> OpenOptions {
    let mut options = OpenOptions::new();
    match flags & open::ACCMODE {
        open::WRONLY => options.write(true),
        open::RDWR => options.read(true).write(true),
        _ => options.read(true),
    };
    if flags & open::APPEND != 0 {
        options.append(true);
    }
    if flags & open::TRUNC != 0 {
        options.truncate(true);
    }
    if flags & open::CREAT != 0 {
        if flags & open::EXCL != 0 {
            options.create_new(true);
        } else {
            options.create(true);
        }
    }
    options
}

fn qid_of(path: &Path) -> io::Result<Qid> {
    fs::symlink_metadata(path).map(|m| qid_from(&m, true))
}

fn qid_from(meta: &fs::Metadata, links: bool) -> Qid {
    let kind = if meta.is_dir() {
        QT_DIR
    } else if links && meta.file_type().is_symlink() {
        QT_SYMLINK
    } else {
        QT_FILE
    };
    Qid {
        kind,
        // Changes when the file does, as far as its times say.
        version: (meta.mtime() as u32) ^ (meta.mtime_nsec() as u32),
        path: meta.ino(),
    }
}

fn attr_of(meta: &fs::Metadata) -> Attr {
    let time = |s: i64, ns: i64| (s.max(0) as u64, ns.max(0) as u64);
    Attr {
        valid: GETATTR_BASIC,
        qid: qid_from(meta, true),
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
        nlink: meta.nlink(),
        rdev: meta.rdev(),
        size: meta.size(),
        blksize: meta.blksize(),
        blocks: meta.blocks(),
        atime: time(meta.atime(), meta.atime_nsec()),
        mtime: time(meta.mtime(), meta.mtime_nsec()),
        ctime: time(meta.ctime(), meta.ctime_nsec()),
        btime: (0, 0),
        generation: 0,
        data_version: 0,
    }
}

/// A directory's entries, each with the offset of the one after it, as
/// Linux's `d_type` names their kinds.
fn entries_of(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for found in fs::read_dir(dir)? {
        let found = found?;
        let Ok(meta) = found.metadata() else { continue };
        let file_type = meta.file_type();
        let kind = if file_type.is_dir() {
            4
        } else if file_type.is_symlink() {
            10
        } else if file_type.is_file() {
            8
        } else {
            0
        };
        entries.push(Entry {
            qid: qid_from(&meta, true),
            offset: entries.len() as u64 + 1,
            kind,
            name: found.file_name().as_bytes().to_vec(),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_takes_one_name_at_a_time_and_stops_at_the_top() {
        let top = Path::new("/");
        assert_eq!(step(top, top, b".."), Some(top.to_path_buf()));
        assert_eq!(step(top, Path::new("/a/b"), b".."), Some("/a".into()));
        assert_eq!(step(top, Path::new("/a"), b"b"), Some("/a/b".into()));
        assert_eq!(step(top, Path::new("/a"), b"b/c"), None);
        assert_eq!(step(top, Path::new("/a"), b""), None);
        // Attached below the top, `..` stops where it was attached.
        let root = Path::new("/a/b");
        assert_eq!(step(root, root, b".."), Some(root.to_path_buf()));
        assert_eq!(
            step(root, Path::new("/a/b/c"), b".."),
            Some(root.to_path_buf())
        );
        assert_eq!(child(Path::new("/a"), b".."), None);
        assert_eq!(child(Path::new("/a"), b"/etc"), None);
    }

    #[test]
    fn open_flags_are_linux_numbers() {
        // Nothing to look at in an OpenOptions but what it opens.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        assert!(options_of(open::RDONLY).open(&path).is_err());
        options_of(open::WRONLY | open::CREAT).open(&path).unwrap();
        assert_eq!(
            options_of(open::WRONLY | open::CREAT | open::EXCL)
                .open(&path)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
