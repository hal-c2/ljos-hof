//! 9P2000.L, the dialect of 9P Linux mounts: the messages, and how they
//! are written.
//!
//! A message is its size (four bytes, counting themselves), its type (one
//! byte), a tag the reply repeats (two bytes), then its fields. Numbers are
//! little-endian; a string is its length (two bytes) and its bytes. Each
//! request has a type, and its reply the type after it; a request that
//! fails is answered with `Rlerror` and a Linux error number instead.
//!
//! Only what Hof and its agent need is here: walking to files and
//! directories, their attributes, reading, writing, creating and removing
//! them.

use std::io::{self, Read};

/// The dialect asked for in `Tversion`.
pub const VERSION: &str = "9P2000.L";
/// The tag of `Tversion`.
pub const NOTAG: u16 = u16::MAX;
/// No fid: `Tattach`'s `afid` when there is no authentication.
pub const NOFID: u32 = u32::MAX;
/// The most names one `Twalk` may hold.
pub const MAXWELEM: usize = 16;
/// What a `Rread` or `Twrite` spends besides its data: size, type, tag,
/// and the fields before the data.
pub const IOHDRSZ: u32 = 24;

/// A qid's type: a directory.
pub const QT_DIR: u8 = 0x80;
/// A qid's type: a symbolic link.
pub const QT_SYMLINK: u8 = 0x02;
/// A qid's type: a plain file.
pub const QT_FILE: u8 = 0;

/// `Tgetattr`'s mask: the basic attributes, all of a `stat`.
pub const GETATTR_BASIC: u64 = 0x7ff;

/// `Tsetattr`'s valid bits.
pub mod set {
    pub const MODE: u32 = 0x1;
    pub const UID: u32 = 0x2;
    pub const GID: u32 = 0x4;
    pub const SIZE: u32 = 0x8;
    pub const ATIME: u32 = 0x10;
    pub const MTIME: u32 = 0x20;
    pub const CTIME: u32 = 0x40;
    pub const ATIME_SET: u32 = 0x80;
    pub const MTIME_SET: u32 = 0x100;
}

/// Open flags, as Linux numbers them.
pub mod open {
    pub const RDONLY: u32 = 0;
    pub const WRONLY: u32 = 1;
    pub const RDWR: u32 = 2;
    pub const ACCMODE: u32 = 3;
    pub const CREAT: u32 = 0o100;
    pub const EXCL: u32 = 0o200;
    pub const TRUNC: u32 = 0o1000;
    pub const APPEND: u32 = 0o2000;
}

/// `Tunlinkat`'s flag for a directory.
pub const AT_REMOVEDIR: u32 = 0x200;

/// Linux error numbers, which 9P2000.L speaks on every machine.
pub mod errno {
    pub const EPERM: u32 = 1;
    pub const ENOENT: u32 = 2;
    pub const EIO: u32 = 5;
    pub const EBADF: u32 = 9;
    pub const EAGAIN: u32 = 11;
    pub const EACCES: u32 = 13;
    pub const EEXIST: u32 = 17;
    pub const EXDEV: u32 = 18;
    pub const ENOTDIR: u32 = 20;
    pub const EISDIR: u32 = 21;
    pub const EINVAL: u32 = 22;
    pub const EFBIG: u32 = 27;
    pub const ENOSPC: u32 = 28;
    pub const EROFS: u32 = 30;
    pub const EMLINK: u32 = 31;
    pub const ENAMETOOLONG: u32 = 36;
    pub const ENOSYS: u32 = 38;
    pub const ENOTEMPTY: u32 = 39;
    pub const ELOOP: u32 = 40;
    pub const EPROTO: u32 = 71;
    pub const ENOTSUP: u32 = 95;
    pub const ETIMEDOUT: u32 = 110;
}

/// The Linux number for `e`, from what it is rather than this machine's
/// own numbering, which on a Mac is not Linux's.
pub fn errno_of(e: &io::Error) -> u32 {
    use io::ErrorKind as K;
    match e.kind() {
        K::NotFound => errno::ENOENT,
        K::PermissionDenied => errno::EACCES,
        K::AlreadyExists => errno::EEXIST,
        K::NotADirectory => errno::ENOTDIR,
        K::IsADirectory => errno::EISDIR,
        K::DirectoryNotEmpty => errno::ENOTEMPTY,
        K::InvalidInput | K::InvalidData => errno::EINVAL,
        K::ReadOnlyFilesystem => errno::EROFS,
        K::StorageFull => errno::ENOSPC,
        K::FileTooLarge => errno::EFBIG,
        K::CrossesDevices => errno::EXDEV,
        K::TooManyLinks => errno::EMLINK,
        K::InvalidFilename => errno::ENAMETOOLONG,
        K::Unsupported => errno::ENOTSUP,
        K::TimedOut => errno::ETIMEDOUT,
        K::WouldBlock => errno::EAGAIN,
        _ => errno::EIO,
    }
}

/// What the other machine's Linux error number `n` says, as an error here.
pub fn error_of(n: u32) -> io::Error {
    use io::ErrorKind as K;
    let kind = match n {
        errno::ENOENT => K::NotFound,
        errno::EPERM | errno::EACCES => K::PermissionDenied,
        errno::EEXIST => K::AlreadyExists,
        errno::ENOTDIR => K::NotADirectory,
        errno::EISDIR => K::IsADirectory,
        errno::ENOTEMPTY => K::DirectoryNotEmpty,
        errno::EINVAL => K::InvalidInput,
        errno::EROFS => K::ReadOnlyFilesystem,
        errno::ENOSPC => K::StorageFull,
        errno::EFBIG => K::FileTooLarge,
        errno::EXDEV => K::CrossesDevices,
        errno::ENAMETOOLONG => K::InvalidFilename,
        errno::ENOSYS | errno::ENOTSUP => K::Unsupported,
        errno::ETIMEDOUT => K::TimedOut,
        _ => K::Other,
    };
    io::Error::new(kind, format!("{kind} on the other machine (error {n})"))
}

/// What a message held that was not what its type takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a 9P message that does not hold what its type takes")]
pub struct Malformed;

/// The server's name for a file: what kind it is, a version that changes
/// with it, and a number no other file there has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Qid {
    pub kind: u8,
    pub version: u32,
    pub path: u64,
}

/// A time: seconds and nanoseconds since 1970.
pub type Time = (u64, u64);

/// A file's attributes, as `Rgetattr` holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    /// Which of these the server filled in.
    pub valid: u64,
    pub qid: Qid,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub rdev: u64,
    pub size: u64,
    pub blksize: u64,
    pub blocks: u64,
    pub atime: Time,
    pub mtime: Time,
    pub ctime: Time,
    pub btime: Time,
    pub generation: u64,
    pub data_version: u64,
}

/// What `Tsetattr` changes: those of its fields that `valid` names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetAttr {
    pub valid: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: Time,
    pub mtime: Time,
}

/// What a file system says of itself, in `Rstatfs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatFs {
    pub kind: u32,
    pub bsize: u32,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub fsid: u64,
    pub namelen: u32,
}

/// One entry of a directory, as `Rreaddir` holds them: the offset is the
/// one to read on from, to have the entries after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub qid: Qid,
    pub offset: u64,
    pub kind: u8,
    pub name: Vec<u8>,
}

impl Entry {
    /// How many bytes it takes in a reply.
    pub fn encoded_len(&self) -> usize {
        13 + 8 + 1 + 2 + self.name.len()
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut w = Writer(out);
        w.qid(self.qid);
        w.u64(self.offset);
        w.u8(self.kind);
        w.bytes(&self.name);
    }

    /// The entries `data` holds.
    pub fn decode_all(data: &[u8]) -> Result<Vec<Entry>, Malformed> {
        let mut r = Reader(data);
        let mut entries = Vec::new();
        while !r.0.is_empty() {
            entries.push(Entry {
                qid: r.qid()?,
                offset: r.u64()?,
                kind: r.u8()?,
                name: r.bytes()?,
            });
        }
        Ok(entries)
    }
}

/// What a client asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Version {
        msize: u32,
        version: String,
    },
    Attach {
        fid: u32,
        afid: u32,
        uname: String,
        aname: Vec<u8>,
        n_uname: u32,
    },
    Walk {
        fid: u32,
        newfid: u32,
        names: Vec<Vec<u8>>,
    },
    GetAttr {
        fid: u32,
        mask: u64,
    },
    SetAttr {
        fid: u32,
        attr: SetAttr,
    },
    Lopen {
        fid: u32,
        flags: u32,
    },
    Lcreate {
        fid: u32,
        name: Vec<u8>,
        flags: u32,
        mode: u32,
        gid: u32,
    },
    Read {
        fid: u32,
        offset: u64,
        count: u32,
    },
    Write {
        fid: u32,
        offset: u64,
        data: Vec<u8>,
    },
    Clunk {
        fid: u32,
    },
    Readdir {
        fid: u32,
        offset: u64,
        count: u32,
    },
    Mkdir {
        dfid: u32,
        name: Vec<u8>,
        mode: u32,
        gid: u32,
    },
    Unlinkat {
        dfid: u32,
        name: Vec<u8>,
        flags: u32,
    },
    Renameat {
        olddfid: u32,
        oldname: Vec<u8>,
        newdfid: u32,
        newname: Vec<u8>,
    },
    Readlink {
        fid: u32,
    },
    Fsync {
        fid: u32,
        datasync: u32,
    },
    Statfs {
        fid: u32,
    },
    Flush {
        oldtag: u16,
    },
    /// A type this end doesn't know, to be answered `ENOSYS`.
    Unknown(u8),
}

/// What a server answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Error(u32),
    Version {
        msize: u32,
        version: String,
    },
    Attach(Qid),
    Walk(Vec<Qid>),
    GetAttr(Box<Attr>),
    SetAttr,
    Lopen {
        qid: Qid,
        iounit: u32,
    },
    Lcreate {
        qid: Qid,
        iounit: u32,
    },
    Read(Vec<u8>),
    Write(u32),
    Clunk,
    /// The entries, encoded.
    Readdir(Vec<u8>),
    Mkdir(Qid),
    Unlinkat,
    Renameat,
    Readlink(Vec<u8>),
    Fsync,
    Statfs(StatFs),
    Flush,
}

mod kind {
    pub const RLERROR: u8 = 7;
    pub const TSTATFS: u8 = 8;
    pub const TLOPEN: u8 = 12;
    pub const TLCREATE: u8 = 14;
    pub const TREADLINK: u8 = 22;
    pub const TGETATTR: u8 = 24;
    pub const TSETATTR: u8 = 26;
    pub const TREADDIR: u8 = 40;
    pub const TFSYNC: u8 = 50;
    pub const TMKDIR: u8 = 72;
    pub const TRENAMEAT: u8 = 74;
    pub const TUNLINKAT: u8 = 76;
    pub const TVERSION: u8 = 100;
    pub const TATTACH: u8 = 104;
    pub const TFLUSH: u8 = 108;
    pub const TWALK: u8 = 110;
    pub const TREAD: u8 = 116;
    pub const TWRITE: u8 = 118;
    pub const TCLUNK: u8 = 120;
}

impl Request {
    fn kind(&self) -> u8 {
        use kind::*;
        match self {
            Request::Version { .. } => TVERSION,
            Request::Attach { .. } => TATTACH,
            Request::Walk { .. } => TWALK,
            Request::GetAttr { .. } => TGETATTR,
            Request::SetAttr { .. } => TSETATTR,
            Request::Lopen { .. } => TLOPEN,
            Request::Lcreate { .. } => TLCREATE,
            Request::Read { .. } => TREAD,
            Request::Write { .. } => TWRITE,
            Request::Clunk { .. } => TCLUNK,
            Request::Readdir { .. } => TREADDIR,
            Request::Mkdir { .. } => TMKDIR,
            Request::Unlinkat { .. } => TUNLINKAT,
            Request::Renameat { .. } => TRENAMEAT,
            Request::Readlink { .. } => TREADLINK,
            Request::Fsync { .. } => TFSYNC,
            Request::Statfs { .. } => TSTATFS,
            Request::Flush { .. } => TFLUSH,
            Request::Unknown(k) => *k,
        }
    }

    /// Writes it, tagged `tag`, onto `out`.
    pub fn encode(&self, tag: u16, out: &mut Vec<u8>) {
        let start = begin(out, self.kind(), tag);
        let mut w = Writer(out);
        match self {
            Request::Version { msize, version } => {
                w.u32(*msize);
                w.bytes(version.as_bytes());
            }
            Request::Attach {
                fid,
                afid,
                uname,
                aname,
                n_uname,
            } => {
                w.u32(*fid);
                w.u32(*afid);
                w.bytes(uname.as_bytes());
                w.bytes(aname);
                w.u32(*n_uname);
            }
            Request::Walk { fid, newfid, names } => {
                w.u32(*fid);
                w.u32(*newfid);
                w.u16(names.len() as u16);
                for name in names {
                    w.bytes(name);
                }
            }
            Request::GetAttr { fid, mask } => {
                w.u32(*fid);
                w.u64(*mask);
            }
            Request::SetAttr { fid, attr } => {
                w.u32(*fid);
                w.u32(attr.valid);
                w.u32(attr.mode);
                w.u32(attr.uid);
                w.u32(attr.gid);
                w.u64(attr.size);
                w.time(attr.atime);
                w.time(attr.mtime);
            }
            Request::Lopen { fid, flags } => {
                w.u32(*fid);
                w.u32(*flags);
            }
            Request::Lcreate {
                fid,
                name,
                flags,
                mode,
                gid,
            } => {
                w.u32(*fid);
                w.bytes(name);
                w.u32(*flags);
                w.u32(*mode);
                w.u32(*gid);
            }
            Request::Read { fid, offset, count } | Request::Readdir { fid, offset, count } => {
                w.u32(*fid);
                w.u64(*offset);
                w.u32(*count);
            }
            Request::Write { fid, offset, data } => {
                w.u32(*fid);
                w.u64(*offset);
                w.u32(data.len() as u32);
                w.0.extend_from_slice(data);
            }
            Request::Clunk { fid } | Request::Readlink { fid } | Request::Statfs { fid } => {
                w.u32(*fid);
            }
            Request::Mkdir {
                dfid,
                name,
                mode,
                gid,
            } => {
                w.u32(*dfid);
                w.bytes(name);
                w.u32(*mode);
                w.u32(*gid);
            }
            Request::Unlinkat { dfid, name, flags } => {
                w.u32(*dfid);
                w.bytes(name);
                w.u32(*flags);
            }
            Request::Renameat {
                olddfid,
                oldname,
                newdfid,
                newname,
            } => {
                w.u32(*olddfid);
                w.bytes(oldname);
                w.u32(*newdfid);
                w.bytes(newname);
            }
            Request::Fsync { fid, datasync } => {
                w.u32(*fid);
                w.u32(*datasync);
            }
            Request::Flush { oldtag } => w.u16(*oldtag),
            Request::Unknown(_) => {}
        }
        end(out, start);
    }

    /// The tag and request a whole message holds.
    pub fn decode(frame: &[u8]) -> Result<(u16, Request), Malformed> {
        use kind::*;
        let (kind, tag, mut r) = open_frame(frame)?;
        let request = match kind {
            TVERSION => Request::Version {
                msize: r.u32()?,
                version: r.string()?,
            },
            TATTACH => Request::Attach {
                fid: r.u32()?,
                afid: r.u32()?,
                uname: r.string()?,
                aname: r.bytes()?,
                n_uname: r.u32()?,
            },
            TWALK => {
                let fid = r.u32()?;
                let newfid = r.u32()?;
                let n = r.u16()? as usize;
                if n > MAXWELEM {
                    return Err(Malformed);
                }
                let names = (0..n).map(|_| r.bytes()).collect::<Result<_, _>>()?;
                Request::Walk { fid, newfid, names }
            }
            TGETATTR => Request::GetAttr {
                fid: r.u32()?,
                mask: r.u64()?,
            },
            TSETATTR => Request::SetAttr {
                fid: r.u32()?,
                attr: SetAttr {
                    valid: r.u32()?,
                    mode: r.u32()?,
                    uid: r.u32()?,
                    gid: r.u32()?,
                    size: r.u64()?,
                    atime: r.time()?,
                    mtime: r.time()?,
                },
            },
            TLOPEN => Request::Lopen {
                fid: r.u32()?,
                flags: r.u32()?,
            },
            TLCREATE => Request::Lcreate {
                fid: r.u32()?,
                name: r.bytes()?,
                flags: r.u32()?,
                mode: r.u32()?,
                gid: r.u32()?,
            },
            TREAD => Request::Read {
                fid: r.u32()?,
                offset: r.u64()?,
                count: r.u32()?,
            },
            TREADDIR => Request::Readdir {
                fid: r.u32()?,
                offset: r.u64()?,
                count: r.u32()?,
            },
            TWRITE => {
                let fid = r.u32()?;
                let offset = r.u64()?;
                let count = r.u32()? as usize;
                Request::Write {
                    fid,
                    offset,
                    data: r.take(count)?.to_vec(),
                }
            }
            TCLUNK => Request::Clunk { fid: r.u32()? },
            TREADLINK => Request::Readlink { fid: r.u32()? },
            TSTATFS => Request::Statfs { fid: r.u32()? },
            TMKDIR => Request::Mkdir {
                dfid: r.u32()?,
                name: r.bytes()?,
                mode: r.u32()?,
                gid: r.u32()?,
            },
            TUNLINKAT => Request::Unlinkat {
                dfid: r.u32()?,
                name: r.bytes()?,
                flags: r.u32()?,
            },
            TRENAMEAT => Request::Renameat {
                olddfid: r.u32()?,
                oldname: r.bytes()?,
                newdfid: r.u32()?,
                newname: r.bytes()?,
            },
            TFSYNC => Request::Fsync {
                fid: r.u32()?,
                datasync: r.u32()?,
            },
            TFLUSH => Request::Flush { oldtag: r.u16()? },
            other => return Ok((tag, Request::Unknown(other))),
        };
        Ok((tag, request))
    }
}

impl Reply {
    /// The reply type for request type `kind`, which is the next one.
    fn kind(&self) -> u8 {
        use kind::*;
        1 + match self {
            Reply::Error(_) => RLERROR - 1,
            Reply::Version { .. } => TVERSION,
            Reply::Attach(_) => TATTACH,
            Reply::Walk(_) => TWALK,
            Reply::GetAttr(_) => TGETATTR,
            Reply::SetAttr => TSETATTR,
            Reply::Lopen { .. } => TLOPEN,
            Reply::Lcreate { .. } => TLCREATE,
            Reply::Read(_) => TREAD,
            Reply::Write(_) => TWRITE,
            Reply::Clunk => TCLUNK,
            Reply::Readdir(_) => TREADDIR,
            Reply::Mkdir(_) => TMKDIR,
            Reply::Unlinkat => TUNLINKAT,
            Reply::Renameat => TRENAMEAT,
            Reply::Readlink(_) => TREADLINK,
            Reply::Fsync => TFSYNC,
            Reply::Statfs(_) => TSTATFS,
            Reply::Flush => TFLUSH,
        }
    }

    /// Writes it, tagged `tag`, onto `out`.
    pub fn encode(&self, tag: u16, out: &mut Vec<u8>) {
        let start = begin(out, self.kind(), tag);
        let mut w = Writer(out);
        match self {
            Reply::Error(n) => w.u32(*n),
            Reply::Version { msize, version } => {
                w.u32(*msize);
                w.bytes(version.as_bytes());
            }
            Reply::Attach(qid) | Reply::Mkdir(qid) => w.qid(*qid),
            Reply::Walk(qids) => {
                w.u16(qids.len() as u16);
                for qid in qids {
                    w.qid(*qid);
                }
            }
            Reply::GetAttr(a) => {
                w.u64(a.valid);
                w.qid(a.qid);
                w.u32(a.mode);
                w.u32(a.uid);
                w.u32(a.gid);
                w.u64(a.nlink);
                w.u64(a.rdev);
                w.u64(a.size);
                w.u64(a.blksize);
                w.u64(a.blocks);
                w.time(a.atime);
                w.time(a.mtime);
                w.time(a.ctime);
                w.time(a.btime);
                w.u64(a.generation);
                w.u64(a.data_version);
            }
            Reply::Lopen { qid, iounit } | Reply::Lcreate { qid, iounit } => {
                w.qid(*qid);
                w.u32(*iounit);
            }
            Reply::Read(data) | Reply::Readdir(data) => {
                w.u32(data.len() as u32);
                w.0.extend_from_slice(data);
            }
            Reply::Write(n) => w.u32(*n),
            Reply::Readlink(target) => w.bytes(target),
            Reply::Statfs(s) => {
                w.u32(s.kind);
                w.u32(s.bsize);
                w.u64(s.blocks);
                w.u64(s.bfree);
                w.u64(s.bavail);
                w.u64(s.files);
                w.u64(s.ffree);
                w.u64(s.fsid);
                w.u32(s.namelen);
            }
            Reply::SetAttr
            | Reply::Clunk
            | Reply::Unlinkat
            | Reply::Renameat
            | Reply::Fsync
            | Reply::Flush => {}
        }
        end(out, start);
    }

    /// The tag and reply a whole message holds.
    pub fn decode(frame: &[u8]) -> Result<(u16, Reply), Malformed> {
        use kind::*;
        let (kind, tag, mut r) = open_frame(frame)?;
        let reply = match kind.wrapping_sub(1) {
            k if k == RLERROR - 1 => Reply::Error(r.u32()?),
            TVERSION => Reply::Version {
                msize: r.u32()?,
                version: r.string()?,
            },
            TATTACH => Reply::Attach(r.qid()?),
            TMKDIR => Reply::Mkdir(r.qid()?),
            TWALK => {
                let n = r.u16()? as usize;
                if n > MAXWELEM {
                    return Err(Malformed);
                }
                Reply::Walk((0..n).map(|_| r.qid()).collect::<Result<_, _>>()?)
            }
            TGETATTR => Reply::GetAttr(Box::new(Attr {
                valid: r.u64()?,
                qid: r.qid()?,
                mode: r.u32()?,
                uid: r.u32()?,
                gid: r.u32()?,
                nlink: r.u64()?,
                rdev: r.u64()?,
                size: r.u64()?,
                blksize: r.u64()?,
                blocks: r.u64()?,
                atime: r.time()?,
                mtime: r.time()?,
                ctime: r.time()?,
                btime: r.time()?,
                generation: r.u64()?,
                data_version: r.u64()?,
            })),
            TLOPEN => Reply::Lopen {
                qid: r.qid()?,
                iounit: r.u32()?,
            },
            TLCREATE => Reply::Lcreate {
                qid: r.qid()?,
                iounit: r.u32()?,
            },
            TREAD | TREADDIR => {
                let count = r.u32()? as usize;
                let data = r.take(count)?.to_vec();
                if kind - 1 == TREAD {
                    Reply::Read(data)
                } else {
                    Reply::Readdir(data)
                }
            }
            TWRITE => Reply::Write(r.u32()?),
            TREADLINK => Reply::Readlink(r.bytes()?),
            TSTATFS => Reply::Statfs(StatFs {
                kind: r.u32()?,
                bsize: r.u32()?,
                blocks: r.u64()?,
                bfree: r.u64()?,
                bavail: r.u64()?,
                files: r.u64()?,
                ffree: r.u64()?,
                fsid: r.u64()?,
                namelen: r.u32()?,
            }),
            TSETATTR => Reply::SetAttr,
            TCLUNK => Reply::Clunk,
            TUNLINKAT => Reply::Unlinkat,
            TRENAMEAT => Reply::Renameat,
            TFSYNC => Reply::Fsync,
            TFLUSH => Reply::Flush,
            _ => return Err(Malformed),
        };
        Ok((tag, reply))
    }
}

/// Reads one whole message off `from`, no longer than `max`.
pub fn read_frame(from: &mut impl Read, max: u32) -> io::Result<Vec<u8>> {
    let mut size = [0; 4];
    from.read_exact(&mut size)?;
    let len = u32::from_le_bytes(size);
    if !(7..=max).contains(&len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a 9P message of {len} bytes"),
        ));
    }
    let mut frame = vec![0; len as usize];
    frame[..4].copy_from_slice(&size);
    from.read_exact(&mut frame[4..])?;
    Ok(frame)
}

fn begin(out: &mut Vec<u8>, kind: u8, tag: u16) -> usize {
    let start = out.len();
    out.extend([0; 4]);
    out.push(kind);
    out.extend(tag.to_le_bytes());
    start
}

fn end(out: &mut [u8], start: usize) {
    let len = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

fn open_frame(frame: &[u8]) -> Result<(u8, u16, Reader<'_>), Malformed> {
    let mut r = Reader(frame);
    let len = r.u32()? as usize;
    if len != frame.len() {
        return Err(Malformed);
    }
    let kind = r.u8()?;
    let tag = r.u16()?;
    Ok((kind, tag, r))
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes());
    }
    /// A string: what doesn't fit its two-byte length is cut.
    fn bytes(&mut self, v: &[u8]) {
        let v = &v[..v.len().min(u16::MAX as usize)];
        self.u16(v.len() as u16);
        self.0.extend_from_slice(v);
    }
    fn qid(&mut self, q: Qid) {
        self.u8(q.kind);
        self.u32(q.version);
        self.u64(q.path);
    }
    fn time(&mut self, (sec, nsec): Time) {
        self.u64(sec);
        self.u64(nsec);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.0.len() < n {
            return Err(Malformed);
        }
        let (taken, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(taken)
    }
    fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two")))
    }
    fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four")))
    }
    fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight")))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, Malformed> {
        let n = self.u16()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, Malformed> {
        String::from_utf8(self.bytes()?).map_err(|_| Malformed)
    }
    fn qid(&mut self) -> Result<Qid, Malformed> {
        Ok(Qid {
            kind: self.u8()?,
            version: self.u32()?,
            path: self.u64()?,
        })
    }
    fn time(&mut self) -> Result<Time, Malformed> {
        Ok((self.u64()?, self.u64()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(request: Request) {
        let mut out = Vec::new();
        request.encode(9, &mut out);
        assert_eq!(Request::decode(&out), Ok((9, request)));
    }

    #[test]
    fn requests_read_back_as_they_were_written() {
        round_trip(Request::Version {
            msize: 8192,
            version: VERSION.into(),
        });
        round_trip(Request::Attach {
            fid: 0,
            afid: NOFID,
            uname: "me".into(),
            aname: b"/".to_vec(),
            n_uname: 1000,
        });
        round_trip(Request::Walk {
            fid: 0,
            newfid: 1,
            names: vec![b"home".to_vec(), b"me".to_vec()],
        });
        round_trip(Request::SetAttr {
            fid: 1,
            attr: SetAttr {
                valid: set::SIZE | set::MTIME_SET,
                size: 5,
                mtime: (7, 8),
                ..SetAttr::default()
            },
        });
        round_trip(Request::Write {
            fid: 1,
            offset: 3,
            data: b"hello".to_vec(),
        });
        round_trip(Request::Renameat {
            olddfid: 1,
            oldname: b"a".to_vec(),
            newdfid: 2,
            newname: b"b".to_vec(),
        });
        round_trip(Request::Flush { oldtag: 4 });
    }

    #[test]
    fn replies_read_back_as_they_were_written() {
        let qid = Qid {
            kind: QT_DIR,
            version: 2,
            path: 99,
        };
        for reply in [
            Reply::Error(errno::ENOENT),
            Reply::Version {
                msize: 8192,
                version: VERSION.into(),
            },
            Reply::Walk(vec![qid, qid]),
            Reply::GetAttr(Box::new(Attr {
                valid: GETATTR_BASIC,
                qid,
                mode: 0o40755,
                size: 4096,
                mtime: (1, 2),
                ..Attr::default()
            })),
            Reply::Lopen { qid, iounit: 100 },
            Reply::Read(b"some bytes".to_vec()),
            Reply::Readdir(Vec::new()),
            Reply::Write(5),
            Reply::Clunk,
            Reply::Statfs(StatFs {
                bsize: 4096,
                namelen: 255,
                ..StatFs::default()
            }),
        ] {
            let mut out = Vec::new();
            reply.encode(3, &mut out);
            assert_eq!(Reply::decode(&out), Ok((3, reply)));
        }
    }

    #[test]
    fn the_wire_is_little_endian_and_counts_its_own_size() {
        let mut out = Vec::new();
        Request::Clunk { fid: 0x0102_0304 }.encode(0x0506, &mut out);
        assert_eq!(out, [11, 0, 0, 0, 120, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn directory_entries_read_back() {
        let entries = vec![
            Entry {
                qid: Qid::default(),
                offset: 1,
                kind: 8,
                name: b"a.txt".to_vec(),
            },
            Entry {
                qid: Qid {
                    kind: QT_DIR,
                    ..Qid::default()
                },
                offset: 2,
                kind: 4,
                name: b"dir".to_vec(),
            },
        ];
        let mut data = Vec::new();
        for e in &entries {
            e.encode(&mut data);
        }
        assert_eq!(
            data.len(),
            entries.iter().map(Entry::encoded_len).sum::<usize>()
        );
        assert_eq!(Entry::decode_all(&data), Ok(entries));
    }

    #[test]
    fn a_short_or_mislabelled_message_is_malformed_and_an_unknown_one_is_named() {
        let mut out = Vec::new();
        Request::Clunk { fid: 1 }.encode(0, &mut out);
        assert_eq!(Request::decode(&out[..8]), Err(Malformed));
        out[4] = 200;
        assert_eq!(Request::decode(&out), Ok((0, Request::Unknown(200))));
    }

    #[test]
    fn errors_go_by_linux_numbers_whatever_the_machine() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(errno_of(&e), errno::ENOENT);
        assert_eq!(
            error_of(errno::ENOTEMPTY).kind(),
            io::ErrorKind::DirectoryNotEmpty
        );
    }
}
