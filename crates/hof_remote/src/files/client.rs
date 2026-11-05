//! Hof's end of the files channel: a 9P2000.L client, asking one thing at
//! a time.

use std::io::{self, Read, Write};

use super::ninep::{
    self, Attr, MAXWELEM, NOFID, NOTAG, Qid, Reply, Request, SetAttr, StatFs, open,
};

/// What Hof asks for: enough for a file's worth in a few messages.
const MSIZE: u32 = 256 * 1024;
/// The fid of the root.
const ROOT: u32 = 0;

/// A session with a 9P server: attached to its root, `/`.
pub struct Client<S> {
    socket: S,
    msize: u32,
    tag: u16,
    next_fid: u32,
    /// The socket failed, or said what isn't 9P: nothing more can be
    /// asked of it.
    broken: bool,
}

/// A path there, as names from the root.
fn names_of(path: &[u8]) -> Vec<Vec<u8>> {
    path.split(|&b| b == b'/')
        .filter(|n| !n.is_empty() && *n != b".")
        .map(<[u8]>::to_vec)
        .collect()
}

fn protocol(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("9P: {what}"))
}

impl<S: Read + Write> Client<S> {
    /// Agrees on the dialect over `socket`, and attaches to the root.
    pub fn new(socket: S, user: &str) -> io::Result<Self> {
        Self::attached(socket, user, b"/")
    }

    /// Agrees on the dialect over `socket`, and attaches at `root`, a
    /// directory there: paths are then from it, and go no higher.
    pub fn attached(socket: S, user: &str, root: &[u8]) -> io::Result<Self> {
        let mut client = Self {
            socket,
            msize: MSIZE,
            tag: 0,
            next_fid: ROOT + 1,
            broken: false,
        };
        let asked = Request::Version {
            msize: MSIZE,
            version: ninep::VERSION.into(),
        };
        match client.ask_tagged(NOTAG, asked)? {
            Reply::Version { msize, version } if version == ninep::VERSION => {
                client.msize = msize.min(MSIZE);
            }
            Reply::Version { version, .. } => {
                return Err(protocol(&format!("the server speaks {version:?}")));
            }
            other => return Err(protocol(&format!("{other:?} to Tversion"))),
        }
        client.ask(Request::Attach {
            fid: ROOT,
            afid: NOFID,
            uname: user.into(),
            aname: root.to_vec(),
            n_uname: u32::MAX,
        })?;
        Ok(client)
    }

    fn ask(&mut self, request: Request) -> io::Result<Reply> {
        self.tag = self.tag.wrapping_add(1) % NOTAG;
        self.ask_tagged(self.tag, request)
    }

    /// Whether the session is over: its socket failed, or it heard what
    /// isn't 9P.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    fn ask_tagged(&mut self, tag: u16, request: Request) -> io::Result<Reply> {
        if self.broken {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let reply = self.exchange(tag, request);
        if reply.is_err() {
            self.broken = true;
        }
        match reply? {
            Reply::Error(n) => Err(ninep::error_of(n)),
            reply => Ok(reply),
        }
    }

    fn exchange(&mut self, tag: u16, request: Request) -> io::Result<Reply> {
        let mut out = Vec::new();
        request.encode(tag, &mut out);
        self.socket.write_all(&out)?;
        self.socket.flush()?;
        let frame = ninep::read_frame(&mut self.socket, self.msize.max(MSIZE))?;
        let (got, reply) = Reply::decode(&frame).map_err(|e| protocol(&e.to_string()))?;
        if got != tag {
            return Err(protocol("a reply to another request"));
        }
        Ok(reply)
    }

    fn fid(&mut self) -> u32 {
        let fid = self.next_fid;
        self.next_fid = self.next_fid.wrapping_add(1).max(ROOT + 1);
        fid
    }

    /// A new fid for `path`, from the root.
    pub fn walk(&mut self, path: &[u8]) -> io::Result<u32> {
        let names = names_of(path);
        let fid = self.fid();
        let mut from = ROOT;
        let mut chunks = names.chunks(MAXWELEM).peekable();
        if chunks.peek().is_none() {
            self.ask(Request::Walk {
                fid: ROOT,
                newfid: fid,
                names: Vec::new(),
            })?;
            return Ok(fid);
        }
        for chunk in chunks {
            let walked = self.ask(Request::Walk {
                fid: from,
                newfid: fid,
                names: chunk.to_vec(),
            });
            let walked = match walked {
                Ok(Reply::Walk(qids)) => qids.len(),
                Ok(other) => {
                    self.forget(from, fid);
                    return Err(protocol(&format!("{other:?} to Twalk")));
                }
                Err(e) => {
                    self.forget(from, fid);
                    return Err(e);
                }
            };
            if walked < chunk.len() {
                self.forget(from, fid);
                return Err(io::ErrorKind::NotFound.into());
            }
            from = fid;
        }
        Ok(fid)
    }

    /// After a walk that failed partway: `fid`, if the walk got that far.
    fn forget(&mut self, from: u32, fid: u32) {
        if from == fid {
            let _ = self.clunk(fid);
        }
    }

    pub fn clunk(&mut self, fid: u32) -> io::Result<()> {
        self.ask(Request::Clunk { fid }).map(drop)
    }

    pub fn getattr(&mut self, fid: u32) -> io::Result<Attr> {
        match self.ask(Request::GetAttr {
            fid,
            mask: ninep::GETATTR_BASIC,
        })? {
            Reply::GetAttr(attr) => Ok(*attr),
            other => Err(protocol(&format!("{other:?} to Tgetattr"))),
        }
    }

    /// The attributes of the file at `path`, in one round trip.
    pub fn stat(&mut self, path: &[u8]) -> io::Result<Attr> {
        match self.at(path, |fid| Request::GetAttr {
            fid,
            mask: ninep::GETATTR_BASIC,
        })? {
            Reply::GetAttr(attr) => Ok(*attr),
            other => Err(protocol(&format!("{other:?} to Tgetattr"))),
        }
    }

    /// Asks what `request` makes of a fid for the file at `path`, in one
    /// round trip: the walk, the request and the letting go are sent
    /// together, and the server answers them in order. A walk that stops
    /// short makes no fid, and the two after it say so.
    fn at(&mut self, path: &[u8], request: impl FnOnce(u32) -> Request) -> io::Result<Reply> {
        let names = names_of(path);
        if names.len() > MAXWELEM {
            let fid = self.walk(path)?;
            let reply = self.ask(request(fid));
            let _ = self.clunk(fid);
            return reply;
        }
        let wanted = names.len();
        let fid = self.fid();
        let asked = request(fid);
        let mut replies = self
            .ask_all(vec![
                Request::Walk {
                    fid: ROOT,
                    newfid: fid,
                    names,
                },
                asked,
                Request::Clunk { fid },
            ])?
            .into_iter();
        match replies.next() {
            Some(Ok(Reply::Walk(qids))) if qids.len() == wanted => {}
            Some(Ok(Reply::Walk(_))) => return Err(io::ErrorKind::NotFound.into()),
            Some(Ok(other)) => return Err(protocol(&format!("{other:?} to Twalk"))),
            Some(Err(e)) => return Err(e),
            None => return Err(protocol("no reply to Twalk")),
        }
        replies
            .next()
            .unwrap_or_else(|| Err(protocol("no reply to a request")))
    }

    /// Changes what `attr` says of the file at `path`.
    pub fn setattr(&mut self, path: &[u8], attr: SetAttr) -> io::Result<()> {
        match self.at(path, |fid| Request::SetAttr { fid, attr })? {
            Reply::SetAttr => Ok(()),
            other => Err(protocol(&format!("{other:?} to Tsetattr"))),
        }
    }

    /// Where the link at `path` points.
    pub fn readlink(&mut self, path: &[u8]) -> io::Result<Vec<u8>> {
        match self.at(path, |fid| Request::Readlink { fid })? {
            Reply::Readlink(target) => Ok(target),
            other => Err(protocol(&format!("{other:?} to Treadlink"))),
        }
    }

    /// What the file system holding `path` says of itself.
    pub fn statfs(&mut self, path: &[u8]) -> io::Result<StatFs> {
        match self.at(path, |fid| Request::Statfs { fid })? {
            Reply::Statfs(fs) => Ok(fs),
            other => Err(protocol(&format!("{other:?} to Tstatfs"))),
        }
    }

    /// Makes the directory `name` in the directory `dir`.
    pub fn mkdir(&mut self, dir: &[u8], name: &[u8], mode: u32) -> io::Result<Qid> {
        let name = name.to_vec();
        match self.at(dir, |dfid| Request::Mkdir {
            dfid,
            name,
            mode,
            gid: u32::MAX,
        })? {
            Reply::Mkdir(qid) => Ok(qid),
            other => Err(protocol(&format!("{other:?} to Tmkdir"))),
        }
    }

    /// Takes `name` out of the directory `dir`: a directory, if
    /// `directory`, else anything else.
    pub fn remove(&mut self, dir: &[u8], name: &[u8], directory: bool) -> io::Result<()> {
        let name = name.to_vec();
        let flags = if directory { ninep::AT_REMOVEDIR } else { 0 };
        match self.at(dir, |dfid| Request::Unlinkat { dfid, name, flags })? {
            Reply::Unlinkat => Ok(()),
            other => Err(protocol(&format!("{other:?} to Tunlinkat"))),
        }
    }

    /// Moves `name` in the directory `from` to `new_name` in `to`.
    pub fn rename(
        &mut self,
        from: &[u8],
        name: &[u8],
        to: &[u8],
        new_name: &[u8],
    ) -> io::Result<()> {
        let olddfid = self.walk(from)?;
        let newdfid = match self.walk(to) {
            Ok(fid) => fid,
            Err(e) => {
                let _ = self.clunk(olddfid);
                return Err(e);
            }
        };
        let renamed = self.ask(Request::Renameat {
            olddfid,
            oldname: name.to_vec(),
            newdfid,
            newname: new_name.to_vec(),
        });
        let _ = self.clunk(olddfid);
        let _ = self.clunk(newdfid);
        match renamed? {
            Reply::Renameat => Ok(()),
            other => Err(protocol(&format!("{other:?} to Trenameat"))),
        }
    }

    /// Sends `requests` together, and reads their replies, each the answer
    /// or the error it was.
    fn ask_all(&mut self, requests: Vec<Request>) -> io::Result<Vec<io::Result<Reply>>> {
        if self.broken {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut out = Vec::new();
        let mut tags = Vec::with_capacity(requests.len());
        for request in requests {
            self.tag = self.tag.wrapping_add(1) % NOTAG;
            request.encode(self.tag, &mut out);
            tags.push(self.tag);
        }
        let replies = (|| {
            self.socket.write_all(&out)?;
            self.socket.flush()?;
            let mut replies = Vec::with_capacity(tags.len());
            for &tag in &tags {
                let frame = ninep::read_frame(&mut self.socket, self.msize.max(MSIZE))?;
                let (got, reply) = Reply::decode(&frame).map_err(|e| protocol(&e.to_string()))?;
                if got != tag {
                    return Err(protocol("a reply to another request"));
                }
                replies.push(match reply {
                    Reply::Error(n) => Err(ninep::error_of(n)),
                    reply => Ok(reply),
                });
            }
            Ok(replies)
        })();
        if replies.is_err() {
            self.broken = true;
        }
        replies
    }

    /// Opens the file at `path` as `flags` say: a fid of it open, and the
    /// most to read or write of it at once. The fid is the caller's to
    /// [`clunk`](Self::clunk).
    pub fn open(&mut self, path: &[u8], flags: u32) -> io::Result<(u32, u32)> {
        let fid = self.walk(path)?;
        match self.lopen(fid, flags) {
            Ok(unit) => Ok((fid, unit)),
            Err(e) => {
                let _ = self.clunk(fid);
                Err(e)
            }
        }
    }

    fn lopen(&mut self, fid: u32, flags: u32) -> io::Result<u32> {
        match self.ask(Request::Lopen { fid, flags })? {
            Reply::Lopen { iounit, .. } => Ok(self.io_unit(iounit)),
            other => Err(protocol(&format!("{other:?} to Tlopen"))),
        }
    }

    fn io_unit(&self, iounit: u32) -> u32 {
        let most = self.msize - ninep::IOHDRSZ;
        if iounit == 0 { most } else { iounit.min(most) }
    }

    /// Writes the file at `path` into `into`, and says what it was when
    /// it was opened.
    pub fn read_file(&mut self, path: &[u8], into: &mut impl Write) -> io::Result<Attr> {
        let fid = self.walk(path)?;
        let read = self.read_open(fid, into);
        let _ = self.clunk(fid);
        read
    }

    fn read_open(&mut self, fid: u32, into: &mut impl Write) -> io::Result<Attr> {
        let unit = self.lopen(fid, open::RDONLY)?;
        let attr = self.getattr(fid)?;
        if attr.mode & 0o170000 == 0o040000 {
            return Err(io::ErrorKind::IsADirectory.into());
        }
        let mut offset = 0;
        loop {
            let data = match self.ask(Request::Read {
                fid,
                offset,
                count: unit,
            })? {
                Reply::Read(data) => data,
                other => return Err(protocol(&format!("{other:?} to Tread"))),
            };
            if data.is_empty() {
                return Ok(attr);
            }
            into.write_all(&data)?;
            offset += data.len() as u64;
        }
    }

    /// Makes the file at `path` hold `bytes`, making it if it is gone, and
    /// says what it is then.
    pub fn write_file(&mut self, path: &[u8], bytes: &[u8]) -> io::Result<Attr> {
        let (fid, unit) = match self.walk(path) {
            Ok(fid) => match self.lopen(fid, open::WRONLY | open::TRUNC) {
                Ok(unit) => (fid, unit),
                Err(e) => {
                    let _ = self.clunk(fid);
                    return Err(e);
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.create_path(path)?,
            Err(e) => return Err(e),
        };
        let written = self.write_open(fid, unit, bytes);
        let _ = self.clunk(fid);
        written
    }

    fn create_path(&mut self, path: &[u8]) -> io::Result<(u32, u32)> {
        let cut = path.iter().rposition(|&b| b == b'/').unwrap_or(0);
        let (dir, name) = (&path[..cut], &path[cut + 1..]);
        self.create(dir, name, open::WRONLY | open::TRUNC, 0o644)
    }

    /// Makes the file `name` in the directory `dir`, open as `flags` say:
    /// a fid of it, and the most to read or write of it at once. The fid
    /// is the caller's to [`clunk`](Self::clunk).
    pub fn create(
        &mut self,
        dir: &[u8],
        name: &[u8],
        flags: u32,
        mode: u32,
    ) -> io::Result<(u32, u32)> {
        let fid = self.walk(dir)?;
        let created = self.ask(Request::Lcreate {
            fid,
            name: name.to_vec(),
            flags,
            mode,
            gid: u32::MAX,
        });
        match created {
            Ok(Reply::Lcreate { iounit, .. }) => Ok((fid, self.io_unit(iounit))),
            Ok(other) => {
                let _ = self.clunk(fid);
                Err(protocol(&format!("{other:?} to Tlcreate")))
            }
            Err(e) => {
                let _ = self.clunk(fid);
                Err(e)
            }
        }
    }

    /// Up to `count` bytes of the open `fid`, from `offset`: none at its
    /// end.
    pub fn read_at(&mut self, fid: u32, offset: u64, count: u32) -> io::Result<Vec<u8>> {
        match self.ask(Request::Read { fid, offset, count })? {
            Reply::Read(data) => Ok(data),
            other => Err(protocol(&format!("{other:?} to Tread"))),
        }
    }

    /// Writes all of `bytes` to the open `fid` at `offset`, `unit` at a
    /// time.
    pub fn write_at(&mut self, fid: u32, unit: u32, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let mut offset = offset;
        for piece in bytes.chunks(unit.max(1) as usize) {
            let mut piece = piece;
            while !piece.is_empty() {
                let n = match self.ask(Request::Write {
                    fid,
                    offset,
                    data: piece.to_vec(),
                })? {
                    Reply::Write(n) => n as usize,
                    other => return Err(protocol(&format!("{other:?} to Twrite"))),
                };
                if n == 0 || n > piece.len() {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                piece = &piece[n..];
                offset += n as u64;
            }
        }
        Ok(())
    }

    /// Has what was written to the open `fid` kept where it lasts.
    pub fn fsync(&mut self, fid: u32, data_only: bool) -> io::Result<()> {
        match self.ask(Request::Fsync {
            fid,
            datasync: u32::from(data_only),
        })? {
            Reply::Fsync => Ok(()),
            other => Err(protocol(&format!("{other:?} to Tfsync"))),
        }
    }

    fn write_open(&mut self, fid: u32, unit: u32, bytes: &[u8]) -> io::Result<Attr> {
        self.write_at(fid, unit, 0, bytes)?;
        self.getattr(fid)
    }

    /// The names in the directory at `path`.
    pub fn list(&mut self, path: &[u8]) -> io::Result<Vec<ninep::Entry>> {
        let fid = self.walk(path)?;
        let listed = self.list_open(fid);
        let _ = self.clunk(fid);
        listed
    }

    fn list_open(&mut self, fid: u32) -> io::Result<Vec<ninep::Entry>> {
        let unit = self.lopen(fid, open::RDONLY)?;
        let mut all = Vec::new();
        let mut offset = 0;
        loop {
            let data = match self.ask(Request::Readdir {
                fid,
                offset,
                count: unit,
            })? {
                Reply::Readdir(data) => data,
                other => return Err(protocol(&format!("{other:?} to Treaddir"))),
            };
            let entries = ninep::Entry::decode_all(&data).map_err(|e| protocol(&e.to_string()))?;
            let Some(last) = entries.last() else {
                return Ok(all);
            };
            offset = last.offset;
            all.extend(entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_its_names() {
        assert_eq!(
            names_of(b"/home//me/./a"),
            [b"home".to_vec(), b"me".to_vec(), b"a".to_vec()]
        );
        assert!(names_of(b"/").is_empty());
    }
}
