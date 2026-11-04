//! What Hof and `hof agent` say to each other, as frames on a byte stream.
//!
//! A frame is a big-endian `u32` length, then that many bytes: a tag that
//! says which [`Message`] it is, and its fields. Integers are big-endian,
//! bytes and strings a `u32` length and then them, lists a `u32` count and
//! then the items, an option a `0` or a `1` and then the value.
//!
//! The length comes first so that a frame of a kind this side doesn't
//! know can be skipped: an older Hof talks to a newer agent, and the other
//! way round, about what both know. [`VERSION`] changes only when that
//! stops being enough.

/// The protocol version. Both ends say theirs in the hello; they talk
/// when it is the same.
pub const VERSION: u32 = 1;

/// The largest frame either end takes. More than a screen's worth of
/// output, so a frame is never larger than one read of a busy pane, less
/// than would let a broken peer make the other allocate without end.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// A pane on the agent's machine, by the number the agent gave it.
pub type PaneId = u32;

/// A connection relayed between the two machines, such as an X11
/// client's, by the number the end that took it gave it.
pub type ChannelId = u32;

/// What a pane is, as the agent lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneInfo {
    pub pane: PaneId,
    /// The program and its arguments; empty for the user's shell.
    pub command: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// How it ended, once it has: its exit code, or `None` for a signal.
    pub exited: Option<Option<i32>>,
}

/// A process on the agent's machine, as `/proc` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Process {
    pub pid: u32,
    /// The kernel's name for it.
    pub name: String,
    /// Its command line, the program first.
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The first thing either end sends: its protocol version and what it
    /// is (Hof's version, the agent's host name).
    Hello {
        version: u32,
        name: String,
        /// The agent's: different each time it starts, so a client that
        /// sees a new one knows the panes it had there are gone, even
        /// where a new pane has the same number. The client's is 0.
        instance: u64,
    },
    /// Start a program on the agent's machine. `request` is the client's
    /// own number, to match the answer to.
    Open {
        request: u32,
        command: Vec<String>,
        /// Where to start; empty for the home directory.
        directory: String,
        /// Extra variables, each `NAME=value`.
        environment: Vec<String>,
        cols: u16,
        rows: u16,
    },
    /// The agent started it, as `pane`. The client attaches to watch it.
    Opened {
        request: u32,
        pane: PaneId,
    },
    /// The agent could not do what was asked of it, or of a pane.
    Refused {
        request: u32,
        reason: String,
    },
    /// Send me `pane`'s output from byte `have` on, and its input applied
    /// so far.
    Attach {
        pane: PaneId,
        have: u64,
    },
    /// Attached. `input` is how much of the client's input the pane has
    /// applied: the client sends again what it sent after that.
    Attached {
        pane: PaneId,
        input: u64,
        cols: u16,
        rows: u16,
    },
    /// Stop sending `pane`'s output; the pane runs on.
    Detach {
        pane: PaneId,
    },
    /// Bytes `at` onward of what the pane's program wrote.
    Output {
        pane: PaneId,
        at: u64,
        bytes: Vec<u8>,
    },
    /// The screen and its scrollback as of byte `at`, as sequences that draw
    /// them: what the client gets in place of output the agent no longer
    /// has. Output carries on from `at`.
    Snapshot {
        pane: PaneId,
        at: u64,
        bytes: Vec<u8>,
    },
    /// Bytes `at` onward of what the user typed into the pane.
    Input {
        pane: PaneId,
        at: u64,
        bytes: Vec<u8>,
    },
    Resize {
        pane: PaneId,
        cols: u16,
        rows: u16,
    },
    /// Hang up on the pane's program.
    Close {
        pane: PaneId,
    },
    /// The pane's program ended: its code, or `None` for a signal.
    Exited {
        pane: PaneId,
        code: Option<i32>,
    },
    /// What panes are there?
    List,
    Panes {
        panes: Vec<PaneInfo>,
    },
    /// Are you there? Either end asks; the other answers with the same
    /// number.
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    /// The agent has no such pane: it ended and was closed, or the agent
    /// started again since.
    Gone {
        pane: PaneId,
    },
    /// What runs in the foreground of the pane's pty now, so Hof can tell
    /// the agents in remote panes as in its own; `None` when the kernel
    /// won't say. Sent on attach, and when it has changed by the next
    /// output, ahead of that output.
    Foreground {
        pane: PaneId,
        process: Option<Process>,
    },
    /// Hof takes connections for these services, `x11` and `sound`, from
    /// programs on the agent's machine. Sent after the hello. The agent
    /// passes each connection on to the newest connection that offered
    /// its service, and sets what finds the service in the panes this one
    /// opens.
    Offer {
        services: Vec<String>,
    },
    /// A program connected to `service`, as `channel`. `opening` is what
    /// it began with, whole (an X11 client's setup, with its
    /// authorization taken out): what the other end may have to rewrite.
    Connect {
        channel: ChannelId,
        service: String,
        opening: Vec<u8>,
    },
    /// What came on the channel, for the other end of it.
    Data {
        channel: ChannelId,
        bytes: Vec<u8>,
    },
    /// `bytes` more of the channel's data reached where it was going:
    /// room for that much more on the way.
    Ack {
        channel: ChannelId,
        bytes: u32,
    },
    /// The channel ended at this end; it ends at the other once what came
    /// before is written.
    Hangup {
        channel: ChannelId,
    },
    /// A frame of a kind this end doesn't know, skipped.
    Unknown {
        tag: u8,
    },
}

// The tags, which never change meaning: a new message gets a new one.
const HELLO: u8 = 1;
const OPEN: u8 = 2;
const OPENED: u8 = 3;
const REFUSED: u8 = 4;
const ATTACH: u8 = 5;
const ATTACHED: u8 = 6;
const DETACH: u8 = 7;
const OUTPUT: u8 = 8;
const SNAPSHOT: u8 = 9;
const INPUT: u8 = 10;
const RESIZE: u8 = 11;
const CLOSE: u8 = 12;
const EXITED: u8 = 13;
const LIST: u8 = 14;
const PANES: u8 = 15;
const PING: u8 = 16;
const PONG: u8 = 17;
const GONE: u8 = 18;
const FOREGROUND: u8 = 19;
const OFFER: u8 = 20;
const CONNECT: u8 = 21;
const DATA: u8 = 22;
const ACK: u8 = 23;
const HANGUP: u8 = 24;

/// A frame that is not what its tag says.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("a frame of {0} bytes, more than {MAX_FRAME}")]
    TooLarge(usize),
    #[error("a frame ends inside its fields")]
    Short,
    #[error("a frame has {0} bytes after its fields")]
    Long(usize),
    #[error("a frame has text that is not UTF-8")]
    NotText,
    #[error("a frame has {0} where a flag is 0 or 1")]
    NotFlag(u8),
    #[error("an empty frame, without even a tag")]
    Empty,
}

impl Message {
    /// Appends the message to `out` as one frame.
    ///
    /// # Panics
    ///
    /// On [`Message::Unknown`], which stands for a frame that came in and
    /// cannot go out again.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend([0; 4]);
        let mut w = Writer(out);
        use Message::*;
        match self {
            Hello {
                version,
                name,
                instance,
            } => {
                w.u8(HELLO);
                w.u32(*version);
                w.str(name);
                w.u64(*instance);
            }
            Open {
                request,
                command,
                directory,
                environment,
                cols,
                rows,
            } => {
                w.u8(OPEN);
                w.u32(*request);
                w.strs(command);
                w.str(directory);
                w.strs(environment);
                w.u16(*cols);
                w.u16(*rows);
            }
            Opened { request, pane } => {
                w.u8(OPENED);
                w.u32(*request);
                w.u32(*pane);
            }
            Refused { request, reason } => {
                w.u8(REFUSED);
                w.u32(*request);
                w.str(reason);
            }
            Attach { pane, have } => {
                w.u8(ATTACH);
                w.u32(*pane);
                w.u64(*have);
            }
            Attached {
                pane,
                input,
                cols,
                rows,
            } => {
                w.u8(ATTACHED);
                w.u32(*pane);
                w.u64(*input);
                w.u16(*cols);
                w.u16(*rows);
            }
            Detach { pane } => {
                w.u8(DETACH);
                w.u32(*pane);
            }
            Output { pane, at, bytes }
            | Snapshot { pane, at, bytes }
            | Input { pane, at, bytes } => {
                w.u8(match self {
                    Output { .. } => OUTPUT,
                    Snapshot { .. } => SNAPSHOT,
                    _ => INPUT,
                });
                w.u32(*pane);
                w.u64(*at);
                w.bytes(bytes);
            }
            Resize { pane, cols, rows } => {
                w.u8(RESIZE);
                w.u32(*pane);
                w.u16(*cols);
                w.u16(*rows);
            }
            Close { pane } => {
                w.u8(CLOSE);
                w.u32(*pane);
            }
            Exited { pane, code } => {
                w.u8(EXITED);
                w.u32(*pane);
                w.option(code.as_ref(), |w, c| w.i32(*c));
            }
            List => w.u8(LIST),
            Panes { panes } => {
                w.u8(PANES);
                w.u32(panes.len() as u32);
                for p in panes {
                    w.u32(p.pane);
                    w.strs(&p.command);
                    w.u16(p.cols);
                    w.u16(p.rows);
                    w.option(p.exited.as_ref(), |w, e| {
                        w.option(e.as_ref(), |w, c| w.i32(*c));
                    });
                }
            }
            Ping { nonce } => {
                w.u8(PING);
                w.u64(*nonce);
            }
            Pong { nonce } => {
                w.u8(PONG);
                w.u64(*nonce);
            }
            Gone { pane } => {
                w.u8(GONE);
                w.u32(*pane);
            }
            Foreground { pane, process } => {
                w.u8(FOREGROUND);
                w.u32(*pane);
                w.option(process.as_ref(), |w, p| {
                    w.u32(p.pid);
                    w.str(&p.name);
                    w.strs(&p.args);
                });
            }
            Offer { services } => {
                w.u8(OFFER);
                w.strs(services);
            }
            Connect {
                channel,
                service,
                opening,
            } => {
                w.u8(CONNECT);
                w.u32(*channel);
                w.str(service);
                w.bytes(opening);
            }
            Data { channel, bytes } => {
                w.u8(DATA);
                w.u32(*channel);
                w.bytes(bytes);
            }
            Ack { channel, bytes } => {
                w.u8(ACK);
                w.u32(*channel);
                w.u32(*bytes);
            }
            Hangup { channel } => {
                w.u8(HANGUP);
                w.u32(*channel);
            }
            Unknown { tag } => panic!("a frame of unknown kind {tag} cannot be sent"),
        }
        let len = (out.len() - start - 4) as u32;
        out[start..start + 4].copy_from_slice(&len.to_be_bytes());
    }

    /// The message one frame's bytes (after its length) make.
    pub fn decode(frame: &[u8]) -> Result<Message, Error> {
        let mut r = Reader(frame);
        let tag = r.u8().map_err(|_| Error::Empty)?;
        let message = match tag {
            HELLO => Message::Hello {
                version: r.u32()?,
                name: r.str()?,
                instance: r.u64()?,
            },
            OPEN => Message::Open {
                request: r.u32()?,
                command: r.strs()?,
                directory: r.str()?,
                environment: r.strs()?,
                cols: r.u16()?,
                rows: r.u16()?,
            },
            OPENED => Message::Opened {
                request: r.u32()?,
                pane: r.u32()?,
            },
            REFUSED => Message::Refused {
                request: r.u32()?,
                reason: r.str()?,
            },
            ATTACH => Message::Attach {
                pane: r.u32()?,
                have: r.u64()?,
            },
            ATTACHED => Message::Attached {
                pane: r.u32()?,
                input: r.u64()?,
                cols: r.u16()?,
                rows: r.u16()?,
            },
            DETACH => Message::Detach { pane: r.u32()? },
            OUTPUT | SNAPSHOT | INPUT => {
                let (pane, at, bytes) = (r.u32()?, r.u64()?, r.bytes()?.to_vec());
                match tag {
                    OUTPUT => Message::Output { pane, at, bytes },
                    SNAPSHOT => Message::Snapshot { pane, at, bytes },
                    _ => Message::Input { pane, at, bytes },
                }
            }
            RESIZE => Message::Resize {
                pane: r.u32()?,
                cols: r.u16()?,
                rows: r.u16()?,
            },
            CLOSE => Message::Close { pane: r.u32()? },
            EXITED => Message::Exited {
                pane: r.u32()?,
                code: r.option(Reader::i32)?,
            },
            LIST => Message::List,
            PANES => {
                let n = r.u32()? as usize;
                // Each takes at least 13 bytes, so a count no frame could
                // hold allocates nothing.
                let mut panes = Vec::with_capacity(n.min(r.0.len() / 13));
                for _ in 0..n {
                    panes.push(PaneInfo {
                        pane: r.u32()?,
                        command: r.strs()?,
                        cols: r.u16()?,
                        rows: r.u16()?,
                        exited: r.option(|r| r.option(Reader::i32))?,
                    });
                }
                Message::Panes { panes }
            }
            PING => Message::Ping { nonce: r.u64()? },
            PONG => Message::Pong { nonce: r.u64()? },
            GONE => Message::Gone { pane: r.u32()? },
            FOREGROUND => Message::Foreground {
                pane: r.u32()?,
                process: r.option(|r| {
                    Ok(Process {
                        pid: r.u32()?,
                        name: r.str()?,
                        args: r.strs()?,
                    })
                })?,
            },
            OFFER => Message::Offer {
                services: r.strs()?,
            },
            CONNECT => Message::Connect {
                channel: r.u32()?,
                service: r.str()?,
                opening: r.bytes()?.to_vec(),
            },
            DATA => Message::Data {
                channel: r.u32()?,
                bytes: r.bytes()?.to_vec(),
            },
            ACK => Message::Ack {
                channel: r.u32()?,
                bytes: r.u32()?,
            },
            HANGUP => Message::Hangup { channel: r.u32()? },
            tag => return Ok(Message::Unknown { tag }),
        };
        match r.0.len() {
            0 => Ok(message),
            extra => Err(Error::Long(extra)),
        }
    }
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend(v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_be_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn strs(&mut self, v: &[String]) {
        self.u32(v.len() as u32);
        for s in v {
            self.str(s);
        }
    }
    fn option<T>(&mut self, v: Option<&T>, f: impl FnOnce(&mut Self, &T)) {
        match v {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                f(self, v);
            }
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(Error::Short)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take()?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u32()? as usize;
        if n > self.0.len() {
            return Err(Error::Short);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn str(&mut self) -> Result<String, Error> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::NotText)
    }
    fn strs(&mut self) -> Result<Vec<String>, Error> {
        let n = self.u32()? as usize;
        // Each takes at least its 4-byte length.
        let mut v = Vec::with_capacity(n.min(self.0.len() / 4));
        for _ in 0..n {
            v.push(self.str()?);
        }
        Ok(v)
    }
    fn option<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        match self.u8()? {
            0 => Ok(None),
            1 => f(self).map(Some),
            other => Err(Error::NotFlag(other)),
        }
    }
}

/// Cuts a byte stream into messages, however it arrives: a frame split
/// across reads, or many in one.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Where the next frame starts in `buf`.
    at: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes as they were read.
    pub fn push(&mut self, bytes: &[u8]) {
        // Move what is left to the front before growing, so a long session
        // doesn't keep everything it ever read.
        if self.at > 0 && self.at * 2 >= self.buf.len() {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, if one has come in.
    ///
    /// An error is for that frame only; the next call goes on with the one
    /// after it. [`Error::TooLarge`] is the exception: past a length that
    /// large nothing can be trusted, so drop the connection.
    pub fn next_message(&mut self) -> Option<Result<Message, Error>> {
        let rest = &self.buf[self.at..];
        let (len, body) = rest.split_first_chunk::<4>()?;
        let len = u32::from_be_bytes(*len) as usize;
        if len > MAX_FRAME {
            return Some(Err(Error::TooLarge(len)));
        }
        let frame = body.get(..len)?;
        let message = Message::decode(frame);
        self.at += 4 + len;
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_message() -> Vec<Message> {
        vec![
            Message::Hello {
                version: VERSION,
                name: "dev".into(),
                instance: 0x0123_4567_89ab_cdef,
            },
            Message::Open {
                request: 7,
                command: vec!["htop".into(), "-d".into(), "10".into()],
                directory: "/srv/ljós".into(),
                environment: vec!["A=b".into()],
                cols: 80,
                rows: 24,
            },
            Message::Opened {
                request: 7,
                pane: 3,
            },
            Message::Refused {
                request: 7,
                reason: "no such program".into(),
            },
            Message::Attach {
                pane: 3,
                have: 1 << 40,
            },
            Message::Attached {
                pane: 3,
                input: 12,
                cols: 80,
                rows: 24,
            },
            Message::Detach { pane: 3 },
            Message::Output {
                pane: 3,
                at: 9,
                bytes: b"\x1b[31mred\x00".to_vec(),
            },
            Message::Snapshot {
                pane: 3,
                at: 9,
                bytes: Vec::new(),
            },
            Message::Input {
                pane: 3,
                at: 0,
                bytes: b"ls\r".to_vec(),
            },
            Message::Resize {
                pane: 3,
                cols: 120,
                rows: 40,
            },
            Message::Close { pane: 3 },
            Message::Exited {
                pane: 3,
                code: Some(-2),
            },
            Message::Exited {
                pane: 4,
                code: None,
            },
            Message::List,
            Message::Panes {
                panes: vec![
                    PaneInfo {
                        pane: 3,
                        command: vec![],
                        cols: 80,
                        rows: 24,
                        exited: None,
                    },
                    PaneInfo {
                        pane: 4,
                        command: vec!["make".into()],
                        cols: 1,
                        rows: 1,
                        exited: Some(None),
                    },
                ],
            },
            Message::Ping { nonce: u64::MAX },
            Message::Pong { nonce: 0 },
            Message::Gone { pane: 4 },
            Message::Foreground {
                pane: 3,
                process: Some(Process {
                    pid: 4242,
                    name: "claude".into(),
                    args: vec!["node".into(), "/usr/bin/claude".into()],
                }),
            },
            Message::Foreground {
                pane: 3,
                process: None,
            },
            Message::Offer {
                services: vec!["x11".into(), "sound".into()],
            },
            Message::Connect {
                channel: 5,
                service: "x11".into(),
                opening: b"l\0\x0b\0".to_vec(),
            },
            Message::Data {
                channel: 5,
                bytes: vec![0, 255, 1],
            },
            Message::Ack {
                channel: 5,
                bytes: 3,
            },
            Message::Hangup { channel: 5 },
        ]
    }

    #[test]
    fn every_message_comes_back_as_it_went() {
        for message in every_message() {
            let mut out = Vec::new();
            message.encode(&mut out);
            let len = u32::from_be_bytes(out[..4].try_into().unwrap()) as usize;
            assert_eq!(len, out.len() - 4, "{message:?}");
            assert_eq!(Message::decode(&out[4..]), Ok(message));
        }
    }

    #[test]
    fn frames_come_out_however_the_bytes_arrive() {
        let messages = every_message();
        let mut stream = Vec::new();
        for m in &messages {
            m.encode(&mut stream);
        }
        // A byte at a time, then all at once.
        for chunk in [1, stream.len()] {
            let mut d = Decoder::new();
            let mut out = Vec::new();
            for piece in stream.chunks(chunk) {
                d.push(piece);
                while let Some(m) = d.next_message() {
                    out.push(m.unwrap());
                }
            }
            assert_eq!(out, messages);
        }
    }

    #[test]
    fn a_frame_of_a_kind_not_known_is_skipped() {
        let mut stream = vec![0, 0, 0, 3, 200, 1, 2];
        Message::List.encode(&mut stream);
        let mut d = Decoder::new();
        d.push(&stream);
        assert_eq!(d.next_message(), Some(Ok(Message::Unknown { tag: 200 })));
        assert_eq!(d.next_message(), Some(Ok(Message::List)));
        assert_eq!(d.next_message(), None);
    }

    #[test]
    fn a_broken_frame_is_an_error_for_it_alone() {
        let mut stream = vec![0, 0, 0, 2, PING, 1];
        Message::Pong { nonce: 5 }.encode(&mut stream);
        let mut d = Decoder::new();
        d.push(&stream);
        assert_eq!(d.next_message(), Some(Err(Error::Short)));
        assert_eq!(d.next_message(), Some(Ok(Message::Pong { nonce: 5 })));
    }

    #[test]
    fn a_length_no_frame_may_have_is_refused_before_it_is_read() {
        let mut d = Decoder::new();
        d.push(&u32::MAX.to_be_bytes());
        assert_eq!(
            d.next_message(),
            Some(Err(Error::TooLarge(u32::MAX as usize)))
        );
    }

    #[test]
    fn lengths_inside_a_frame_cannot_reach_past_it() {
        // A string claiming more bytes than the frame has.
        let frame = [HELLO, 0, 0, 0, 1, 0, 0, 0, 9, b'x'];
        assert_eq!(Message::decode(&frame), Err(Error::Short));
        // A list claiming a billion strings allocates nothing for them.
        let frame = [OPEN, 0, 0, 0, 1, 0x40, 0, 0, 0];
        assert_eq!(Message::decode(&frame), Err(Error::Short));
    }
}
