//! Hof's end of the connection to one host's agent.
//!
//! A [`Host`] is a thread of its own that keeps one connection to the
//! agent while it has panes there, and makes it again when it drops. Each
//! [`Pane`] has a [`Screen`] the output goes into. The output is numbered,
//! so after a reconnect only what was missed comes, or a snapshot when that
//! is gone. Input is numbered too, and what the agent may not have had is
//! sent again, which it types only once.
//!
//! Everything about the connection happens on that one thread, in the
//! order it happens; reading and writing have threads of their own, so
//! that a stalled connection never stops it noticing.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{self, Child, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::queue::{Lane, Lanes};
use crate::relay::{Relays, Socket};
use crate::stream::{Backlog, Received, Resume};
use crate::wire::{ChannelId, Decoder, Message, PaneId, VERSION};

/// Something here that programs on the host may connect to, through the
/// agent: the X server, the sound server.
pub trait Service: Send + Sync + 'static {
    /// What the agent knows it by: `x11`, `sound`.
    fn name(&self) -> &str;
    /// Connects to it here, for a program there that opened with
    /// `opening`, and says what to write to it first: the opening, with
    /// this machine's authorization put in.
    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)>;
}

impl std::fmt::Debug for dyn Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The local copy of a remote pane: where its output goes.
pub trait Screen: Send + Sync + 'static {
    /// More of the program's output.
    fn output(&self, bytes: &[u8]);
    /// The screen as it is now, in place of everything before.
    fn replace(&self, snapshot: &[u8]);
    /// The program ended.
    fn exited(&self, code: Option<i32>);
    /// The connection changed.
    fn state(&self, state: State);
    /// What runs in the foreground of the program's pty, when the agent
    /// says; an agent too old to say never calls it.
    fn foreground(&self, _process: Option<crate::wire::Process>) {}
}

/// How a pane's connection is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Reaching the host, the first time.
    Connecting,
    /// Attached: the agent's pane, its output coming in.
    Up { pane: PaneId },
    /// The connection dropped, and is being made again.
    Down(String),
    /// Over for good: the program could not start, or the agent forgot it.
    Gone(String),
}

/// What a new pane runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Start {
    /// Empty for the user's shell there.
    pub command: Vec<String>,
    /// Empty for their home there.
    pub directory: String,
    /// `NAME=value` each.
    pub environment: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// How often to ask whether the agent is still there.
    pub keepalive: Duration,
    /// How long it may say nothing before the connection counts as dead.
    pub dead_after: Duration,
    /// The wait before trying again, first and longest.
    pub retry_first: Duration,
    pub retry_longest: Duration,
    /// How much of what was typed to keep for sending again.
    pub input_backlog: usize,
    /// What programs on the host may reach here.
    pub services: Vec<Arc<dyn Service>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(10),
            dead_after: Duration::from_secs(30),
            retry_first: Duration::from_millis(250),
            retry_longest: Duration::from_secs(15),
            input_backlog: 1 << 20,
            services: Vec::new(),
        }
    }
}

/// How much decoded input may wait for the driver. Only the reader thread
/// waits for room; the UI and driver never block on this budget.
const EVENT_MEMORY: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct Budget {
    used: Mutex<usize>,
    room: std::sync::Condvar,
}

impl Budget {
    fn take(&self, weight: usize) {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while *used + weight > EVENT_MEMORY {
            used = self.room.wait(used).unwrap_or_else(|e| e.into_inner());
        }
        *used += weight;
    }

    fn give(&self, weight: usize) {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        *used = used.saturating_sub(weight);
        self.room.notify_all();
    }
}

/// How to reach the agent: `ssh`, mostly.
pub trait Transport: Send + 'static {
    fn connect(&mut self) -> io::Result<Connection>;
}

/// One connection to the agent.
pub struct Connection {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    /// Ends it, and says why it ended if it can tell (what ssh printed).
    pub end: Box<dyn FnOnce() -> Option<String> + Send>,
}

/// A program whose stdin and stdout are the connection, its stderr the
/// reason it ended: `ssh -T host -- hof agent --stdio`.
pub struct Spawned {
    pub argv: Vec<String>,
}

impl Transport for Spawned {
    fn connect(&mut self) -> io::Result<Connection> {
        spawn(&self.argv, None)
    }
}

/// How a spawned transport ended: its exit code and the last lines it
/// wrote to stderr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ended {
    pub code: Option<i32>,
    pub said: Vec<String>,
}

/// Runs `argv` as the connection; once it ends, how it did goes in
/// `noted`, if given.
pub fn spawn(argv: &[String], noted: Option<Arc<Mutex<Ended>>>) -> io::Result<Connection> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no command to run"))?;
    let mut child = process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("{program}: {e}")))?;
    let (Some(writer), Some(reader), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(io::Error::other("no pipes to the transport"));
    };
    let said = Arc::new(Mutex::new(VecDeque::new()));
    let (done, finished) = mpsc::channel();
    let keep = said.clone();
    thread::Builder::new()
        .name("hof-remote-stderr".into())
        .spawn(move || {
            keep_last_lines(stderr, &keep);
            let _ = done.send(());
        })?;
    Ok(Connection {
        reader: Box::new(reader),
        writer: Box::new(writer),
        end: Box::new(move || end_child(child, &said, &finished, noted.as_deref())),
    })
}

/// The most of one diagnostic line kept. The rest is discarded as it is
/// read, so a peer that never writes a newline cannot grow this string.
const MOST_LINE: usize = 4 * 1024;

/// The last few lines a program wrote, for saying why it failed.
fn keep_last_lines(from: impl Read, into: &Mutex<VecDeque<String>>) {
    let mut reader = io::BufReader::new(from);
    loop {
        let mut line = Vec::new();
        let read = match read_capped_line(&mut reader, &mut line) {
            Ok(read) => read,
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut lines = into.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() == 3 {
            lines.pop_front();
        }
        lines.push_back(line.to_owned());
    }
}

fn read_capped_line(reader: &mut impl io::BufRead, out: &mut Vec<u8>) -> io::Result<usize> {
    let mut total = 0;
    loop {
        let mut byte = [0];
        if reader.read(&mut byte)? == 0 {
            return Ok(total);
        }
        total += 1;
        if byte[0] == b'\n' {
            return Ok(total);
        }
        if out.len() < MOST_LINE {
            out.push(byte[0]);
        }
    }
}

fn end_child(
    mut child: Child,
    said: &Mutex<VecDeque<String>>,
    finished: &mpsc::Receiver<()>,
    noted: Option<&Mutex<Ended>>,
) -> Option<String> {
    // A connection that closed from the far end is a program about to
    // exit: a moment for it to, so its exit code says why.
    let deadline = Instant::now() + Duration::from_millis(200);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    let status = child.wait().ok();
    // Not forever: a process it left behind (an ssh control master) may
    // hold its stderr open.
    let _ = finished.recv_timeout(Duration::from_millis(500));
    let lines = said.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(noted) = noted {
        *noted.lock().unwrap_or_else(|e| e.into_inner()) = Ended {
            code: status.and_then(|s| s.code()),
            said: lines.iter().cloned().collect(),
        };
    }
    if !lines.is_empty() {
        return Some(lines.iter().cloned().collect::<Vec<_>>().join("; "));
    }
    match status.and_then(|s| s.code()) {
        Some(0) | None => None,
        Some(code) => Some(format!("the connection exited with {code}")),
    }
}

/// The connection to one host, while it has panes.
#[derive(Clone)]
pub struct Host {
    to: mpsc::Sender<Event>,
    next: Arc<AtomicU32>,
    next_channel: Arc<AtomicU32>,
}

/// The channels Hof opens have this bit set, the agent's not: neither end
/// takes a number the other has.
pub const HOF_CHANNEL: ChannelId = 1 << 31;

/// How long [`Host::connect`] waits for the connection to be up.
const CONNECT_WAIT: Duration = Duration::from_secs(2);

/// One pane on a host. Dropping it neither closes nor keeps the program:
/// say which with [`Pane::close`] or [`Pane::detach`].
pub struct Pane {
    to: mpsc::Sender<Event>,
    id: u32,
}

impl Host {
    pub fn new(transport: impl Transport, options: Options) -> Self {
        let (to, events) = mpsc::channel();
        let incoming = Arc::new(Budget::default());
        let driver = Driver::new(Box::new(transport), options, to.clone(), events, incoming);
        // Not joined: it waits for panes for as long as the process lives.
        let _ = thread::Builder::new()
            .name("hof-remote".into())
            .spawn(move || driver.run());
        Self {
            to,
            next: Arc::new(AtomicU32::new(1)),
            next_channel: Arc::new(AtomicU32::new(0)),
        }
    }

    /// A channel to the agent's `service`, begun with `opening`: what is
    /// written to the socket this gives goes to it, and what it answers
    /// comes back. Fails if the connection is not up within a moment;
    /// an agent that has no such service hangs up, and the socket ends.
    pub fn connect(&self, service: &str, opening: Vec<u8>) -> io::Result<UnixStream> {
        let given_up = Instant::now() + CONNECT_WAIT;
        let relays = loop {
            let (answer, answered) = mpsc::channel();
            if self.to.send(Event::Relays(answer)).is_err() {
                return Err(io::ErrorKind::NotConnected.into());
            }
            match answered.recv_timeout(CONNECT_WAIT) {
                Ok(Some(relays)) => break relays,
                Ok(None) if Instant::now() < given_up => {
                    thread::sleep(Duration::from_millis(100));
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "not connected to the agent",
                    ));
                }
            }
        };
        let (ours, theirs) = UnixStream::pair()?;
        let n = self.next_channel.fetch_add(1, Ordering::Relaxed);
        relays.connect_out(
            HOF_CHANNEL | (n & !HOF_CHANNEL),
            service,
            opening,
            Box::new(theirs),
        );
        Ok(ours)
    }

    /// A new pane running `start`.
    pub fn open(&self, start: Start, screen: Arc<dyn Screen>) -> Pane {
        let pane = self.pane();
        pane.send(Command::Open { start, screen });
        pane
    }

    /// The agent's pane `remote`, which runs already: a pane of a saved
    /// session, or of another window.
    pub fn attach(&self, remote: PaneId, cols: u16, rows: u16, screen: Arc<dyn Screen>) -> Pane {
        let pane = self.pane();
        pane.send(Command::Attach {
            remote,
            cols,
            rows,
            screen,
        });
        pane
    }

    /// Hof is going: every pane still open is detached (`keep`), its
    /// program left running, or closed, and the connection hung up once
    /// that is sent. Waits for that up to `within`; what is asked of the
    /// host after is dropped.
    pub fn finish(&self, keep: bool, within: Duration) {
        let (done, finished) = mpsc::channel();
        if self.to.send(Event::Finish { keep, done }).is_ok() {
            let _ = finished.recv_timeout(within);
        }
    }

    fn pane(&self) -> Pane {
        Pane {
            to: self.to.clone(),
            id: self.next.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl Pane {
    /// What the user typed, encoded, for the program.
    pub fn input(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.send(Command::Input(bytes.to_vec()));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.send(Command::Resize { cols, rows });
    }

    /// Ends the program, on its machine.
    pub fn close(&self) {
        self.send(Command::Close);
    }

    /// Leaves the program running on its machine, to attach to later.
    pub fn detach(&self) {
        self.send(Command::Detach);
    }

    fn send(&self, command: Command) {
        let _ = self.to.send(Event::Pane(self.id, command));
    }
}

enum Command {
    Open {
        start: Start,
        screen: Arc<dyn Screen>,
    },
    Attach {
        remote: PaneId,
        cols: u16,
        rows: u16,
        screen: Arc<dyn Screen>,
    },
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Close,
    Detach,
}

enum Event {
    Pane(u32, Command),
    /// From connection `.0`'s reader.
    Heard(u64, Message),
    /// Connection `.0` ended, and why if it could tell.
    Ended(u64, Option<String>),
    /// [`Host::connect`] wants the channels of the connection, once the
    /// agent has said hello on it.
    Relays(mpsc::Sender<Option<Relays>>),
    /// [`Host::finish`].
    Finish {
        keep: bool,
        done: mpsc::Sender<()>,
    },
}

/// One pane, as the connection knows it.
struct Slot {
    screen: Arc<dyn Screen>,
    /// The agent's number for it, once it has one.
    remote: Option<PaneId>,
    /// What to start, until the agent has started it.
    start: Option<Start>,
    /// An `Open` is out and not answered.
    opening: bool,
    /// Closed before its `Open` was answered: close it when it is.
    close_when_opened: bool,
    received: Received,
    input: Backlog,
    cols: u16,
    rows: u16,
    attached: bool,
    /// Nothing more will come: it ended, or went.
    over: bool,
}

/// Bytes of encoded output waiting for the transport, in each lane. Relay
/// readers wait for room; the driver drops a connection whose control
/// traffic can't go.
const TRANSPORT_MEMORY: usize = 4 * 1024 * 1024;
/// Relays waiting for room leave this much of theirs for their hangups.
const HANGUP_ROOM: usize = 64 * 1024;
/// The most bulk written between looks at what is urgent: one relay piece.
const BULK_PER_WRITE: usize = 64 * 1024;
/// The most typed or pasted one `Input` carries; a paste goes in several.
const INPUT_PIECE: usize = 64 * 1024;

/// The connection that is up.
struct Live {
    generation: u64,
    frames: Arc<Lanes>,
    /// Says when the writer has written all it was given, or can't.
    written: mpsc::Receiver<()>,
    end: Option<Box<dyn FnOnce() -> Option<String> + Send>>,
    /// The agent said hello: panes can be opened and attached.
    greeted: bool,
    heard: Instant,
    pinged: Instant,
    /// The connections the agent passed on, over this one.
    relays: Relays,
}

struct Driver {
    transport: Box<dyn Transport>,
    options: Options,
    to: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
    incoming: Arc<Budget>,
    live: Option<Live>,
    generation: u64,
    /// The agent's, once one said hello: another means it started again.
    instance: Option<u64>,
    slots: HashMap<u32, Slot>,
    /// The agent's pane numbers, to ours.
    remotes: HashMap<PaneId, u32>,
    /// Panes to close once there is a connection again; until then the
    /// agent keeps them, and would keep itself for them.
    to_close: Vec<PaneId>,
    retry_at: Option<Instant>,
    retry_wait: Duration,
    /// Why the last connection ended, for the panes that wait on the next.
    why: Option<String>,
}

impl Driver {
    fn new(
        transport: Box<dyn Transport>,
        options: Options,
        to: mpsc::Sender<Event>,
        events: mpsc::Receiver<Event>,
        incoming: Arc<Budget>,
    ) -> Self {
        Self {
            transport,
            retry_wait: options.retry_first,
            options,
            to,
            events,
            incoming,
            live: None,
            generation: 0,
            instance: None,
            slots: HashMap::new(),
            remotes: HashMap::new(),
            to_close: Vec::new(),
            retry_at: None,
            why: None,
        }
    }

    fn run(mut self) {
        loop {
            let wait = self
                .next_due()
                .map(|at| at.saturating_duration_since(Instant::now()));
            let event = match wait {
                Some(wait) => match self.events.recv_timeout(wait) {
                    Ok(event) => Some(event),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                },
                None => match self.events.recv() {
                    Ok(event) => Some(event),
                    Err(_) => return,
                },
            };
            match event {
                Some(Event::Finish { keep, done }) => {
                    self.finish(keep);
                    let _ = done.send(());
                    return;
                }
                Some(Event::Pane(id, command)) => self.command(id, command),
                Some(Event::Relays(answer)) => {
                    let relays = self
                        .live
                        .as_ref()
                        .filter(|l| l.greeted)
                        .map(|l| l.relays.clone());
                    let _ = answer.send(relays);
                }
                Some(Event::Heard(generation, message)) => {
                    self.incoming.give(message_weight(&message));
                    // What an old connection says comes too late to matter.
                    if self.is_live(generation) {
                        self.heard(message);
                    }
                }
                Some(Event::Ended(generation, why)) if self.is_live(generation) => {
                    self.lost(why);
                }
                Some(Event::Ended(..)) | None => {}
            }
            self.tick();
        }
    }

    fn is_live(&self, generation: u64) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.generation == generation)
    }

    /// Whether there is anything to be connected for.
    /// A pane whose program ended is still the agent's until it is
    /// closed, so it still wants the connection to close it on.
    fn wanted(&self) -> bool {
        !self.to_close.is_empty() || self.slots.values().any(|s| !s.over || s.remote.is_some())
    }

    /// When the loop has something to do with no event.
    fn next_due(&self) -> Option<Instant> {
        match &self.live {
            Some(live) if live.greeted => Some(
                (live.pinged + self.options.keepalive).min(live.heard + self.options.dead_after),
            ),
            // Saying hello, or not: only so long.
            Some(live) => Some(live.heard + self.options.dead_after),
            None if self.wanted() => Some(self.retry_at.unwrap_or_else(Instant::now)),
            None => None,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.live.is_none() {
            if self.wanted() && self.retry_at.is_none_or(|at| now >= at) {
                self.connect();
            }
            return;
        }
        if !self.wanted() {
            self.hang_up();
            return;
        }
        let Some(live) = &mut self.live else { return };
        if now >= live.heard + self.options.dead_after {
            let silent = self.options.dead_after.as_secs();
            self.lost(Some(format!("no answer for {silent} s")));
        } else if live.greeted && now >= live.pinged + self.options.keepalive {
            live.pinged = now;
            self.send(Message::Ping { nonce: 0 });
        }
    }

    fn connect(&mut self) {
        self.generation += 1;
        let generation = self.generation;
        let connection = match self.transport.connect() {
            Ok(c) => c,
            Err(e) => {
                self.failed(Some(e.to_string()));
                return;
            }
        };
        let Connection {
            reader,
            writer,
            end,
        } = connection;
        let frames = Arc::new(Lanes::default());
        let (wrote, written) = mpsc::channel();
        let to = self.to.clone();
        let writing = frames.clone();
        let started = thread::Builder::new()
            .name("hof-remote-write".into())
            .spawn(move || {
                write_frames(writer, &writing, &to, generation);
                let _ = wrote.send(());
            })
            .and_then(|_| {
                let to = self.to.clone();
                let incoming = self.incoming.clone();
                thread::Builder::new()
                    .name("hof-remote-read".into())
                    .spawn(move || read_frames(reader, &to, generation, incoming))
            });
        let now = Instant::now();
        let to_relay = frames.clone();
        let relays = Relays::new(move |message, wait| {
            // An acknowledgement is small, and what lets the other end go
            // on sending: it goes with the panes' traffic, not behind data.
            let lane = match message {
                Message::Ack { .. } => Lane::Urgent,
                _ => Lane::Bulk,
            };
            let mut frame = Vec::new();
            message.encode(&mut frame);
            let room = if wait && lane == Lane::Bulk {
                TRANSPORT_MEMORY - HANGUP_ROOM
            } else {
                TRANSPORT_MEMORY
            };
            to_relay.push(lane, frame, room, wait)
        });
        self.live = Some(Live {
            generation,
            frames,
            written,
            end: Some(end),
            greeted: false,
            heard: now,
            pinged: now,
            relays,
        });
        if let Err(e) = started {
            self.lost(Some(e.to_string()));
            return;
        }
        self.send(Message::Hello {
            version: VERSION,
            name: "hof".into(),
            instance: 0,
        });
    }

    /// The connection dropped, or never came.
    fn lost(&mut self, why: Option<String>) {
        let said = self.hang_up();
        self.failed(why.or(said));
    }

    /// Detaches or closes every pane, and hangs up once that is written.
    /// With no connection there is no one to tell: the agent keeps the
    /// panes, as it does for a connection that dropped.
    fn finish(&mut self, keep: bool) {
        let ids: Vec<u32> = self.slots.keys().copied().collect();
        for id in ids {
            let command = if keep {
                Command::Detach
            } else {
                Command::Close
            };
            self.command(id, command);
        }
        self.hang_up();
    }

    /// Ends the connection, if there is one; what it says about why.
    fn hang_up(&mut self) -> Option<String> {
        let Live {
            frames,
            written,
            end,
            relays,
            ..
        } = self.live.take()?;
        // They go with it, and let go of the writer.
        relays.close_all();
        // Closing wakes relay senders as well as the transport writer.
        // What was sent last still drains before the connection ends.
        frames.close();
        let _ = written.recv_timeout(Duration::from_secs(1));
        // Closed while it opened: whether it did is lost with the answer.
        self.slots.retain(|_, s| !s.close_when_opened);
        for slot in self.slots.values_mut() {
            slot.attached = false;
            // An `Open` that wasn't answered is sent again on the next.
            slot.opening = false;
        }
        end.and_then(|end| end())
    }

    fn failed(&mut self, why: Option<String>) {
        let why = why.unwrap_or_else(|| "the connection closed".into());
        for slot in self.slots.values().filter(|s| !s.over) {
            slot.screen.state(State::Down(why.clone()));
        }
        self.why = Some(why);
        self.retry_at = Some(Instant::now() + self.retry_wait);
        self.retry_wait = (self.retry_wait * 2).min(self.options.retry_longest);
    }

    fn send(&mut self, message: Message) {
        let sent = {
            let Some(live) = &self.live else { return };
            let mut frame = Vec::new();
            message.encode(&mut frame);
            live.frames
                .push(Lane::Urgent, frame, TRANSPORT_MEMORY, false)
        };
        if !sent {
            self.lost(Some("the agent is not reading".into()));
        }
    }

    /// What was typed from `at` on, in pieces: a paste neither passes a
    /// frame's limit nor holds up the keystrokes after it for long.
    fn send_input(&mut self, pane: PaneId, mut at: u64, bytes: &[u8]) {
        for piece in bytes.chunks(INPUT_PIECE) {
            self.send(Message::Input {
                pane,
                at,
                bytes: piece.to_vec(),
            });
            at += piece.len() as u64;
        }
    }

    fn greeted(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.greeted)
    }

    fn command(&mut self, id: u32, command: Command) {
        match command {
            Command::Open { start, screen } => {
                screen.state(self.waiting());
                let slot = Slot::new(screen, None, Some(start), &self.options);
                self.slots.insert(id, slot);
                self.open(id);
            }
            Command::Attach {
                remote,
                cols,
                rows,
                screen,
            } => {
                screen.state(self.waiting());
                let mut slot = Slot::new(screen, Some(remote), None, &self.options);
                (slot.cols, slot.rows) = (cols, rows);
                self.slots.insert(id, slot);
                self.remotes.insert(remote, id);
                self.attach(id);
            }
            Command::Input(bytes) => {
                let Some(slot) = self.slots.get_mut(&id) else {
                    return;
                };
                if slot.over {
                    return;
                }
                let at = slot.input.end();
                slot.input.push(&bytes);
                if let (true, Some(pane)) = (slot.attached, slot.remote) {
                    self.send_input(pane, at, &bytes);
                }
            }
            Command::Resize { cols, rows } => {
                let Some(slot) = self.slots.get_mut(&id) else {
                    return;
                };
                if (slot.cols, slot.rows) == (cols, rows) {
                    return;
                }
                (slot.cols, slot.rows) = (cols, rows);
                if let (true, Some(pane)) = (slot.attached, slot.remote) {
                    self.send(Message::Resize { pane, cols, rows });
                }
            }
            Command::Close => {
                let Some(slot) = self.slots.get_mut(&id) else {
                    return;
                };
                if slot.opening {
                    slot.close_when_opened = true;
                    slot.over = true;
                    return;
                }
                let slot = self.forget(id);
                if let Some(pane) = slot.and_then(|s| s.remote) {
                    if self.greeted() {
                        self.send(Message::Close { pane });
                    } else {
                        self.to_close.push(pane);
                    }
                }
            }
            Command::Detach => {
                let slot = self.forget(id);
                if let Some(pane) = slot.and_then(|s| s.remote)
                    && self.greeted()
                {
                    self.send(Message::Detach { pane });
                }
            }
        }
    }

    /// What a new pane waits in: the first connection, or the next.
    fn waiting(&self) -> State {
        match &self.why {
            Some(why) if self.live.is_none() => State::Down(why.clone()),
            _ => State::Connecting,
        }
    }

    fn forget(&mut self, id: u32) -> Option<Slot> {
        let slot = self.slots.remove(&id)?;
        if let Some(pane) = slot.remote {
            self.remotes.remove(&pane);
        }
        Some(slot)
    }

    fn open(&mut self, id: u32) {
        if !self.greeted() {
            return;
        }
        let Some(slot) = self.slots.get_mut(&id) else {
            return;
        };
        let Some(start) = &slot.start else { return };
        slot.opening = true;
        let message = Message::Open {
            request: id,
            command: start.command.clone(),
            directory: start.directory.clone(),
            environment: start.environment.clone(),
            cols: slot.cols.max(1),
            rows: slot.rows.max(1),
        };
        self.send(message);
    }

    fn attach(&mut self, id: u32) {
        if !self.greeted() {
            return;
        }
        let Some(slot) = self.slots.get(&id) else {
            return;
        };
        let Some(pane) = slot.remote else { return };
        let have = slot.received.end();
        self.send(Message::Attach { pane, have });
    }

    fn heard(&mut self, message: Message) {
        if let Some(live) = &mut self.live {
            live.heard = Instant::now();
        }
        match message {
            Message::Hello {
                version, instance, ..
            } => self.hello(version, instance),
            Message::Opened { request, pane } => {
                let Some(slot) = self.slots.get_mut(&request) else {
                    // Closed and forgotten meanwhile: it still runs there.
                    self.send(Message::Close { pane });
                    return;
                };
                slot.opening = false;
                slot.start = None;
                slot.remote = Some(pane);
                if slot.close_when_opened {
                    self.slots.remove(&request);
                    self.send(Message::Close { pane });
                    return;
                }
                self.remotes.insert(pane, request);
                self.attach(request);
            }
            Message::Refused { request, reason } => {
                if let Some(slot) = self.slots.get_mut(&request) {
                    slot.opening = false;
                    slot.end(State::Gone(reason));
                }
            }
            Message::Attached {
                pane,
                input,
                cols,
                rows,
            } => {
                let Some(slot) = self.remote_slot(pane) else {
                    return;
                };
                slot.attached = true;
                let resend = match slot.input.resume(input) {
                    Resume::Rest(bytes) => bytes,
                    // It has more than was sent, or less than is kept: count
                    // on from what it has.
                    Resume::Lost | Resume::Ahead => {
                        slot.input.restart_at(input);
                        Vec::new()
                    }
                };
                slot.input.acknowledge(input);
                let size = (slot.cols, slot.rows);
                slot.screen.state(State::Up { pane });
                self.send_input(pane, input, &resend);
                if size != (cols, rows) && size.0 > 0 && size.1 > 0 {
                    self.send(Message::Resize {
                        pane,
                        cols: size.0,
                        rows: size.1,
                    });
                }
            }
            Message::Output { pane, at, bytes } => {
                let Some(slot) = self.remote_slot(pane) else {
                    return;
                };
                match slot.received.take(at, &bytes) {
                    Ok(new) if !new.is_empty() => slot.screen.output(new),
                    Ok(_) => {}
                    // Bytes went missing on the way: ask from where they did.
                    Err(gap) => self.send(Message::Attach {
                        pane,
                        have: gap.have,
                    }),
                }
            }
            Message::Snapshot { pane, at, bytes } => {
                let Some(slot) = self.remote_slot(pane) else {
                    return;
                };
                slot.screen.replace(&bytes);
                slot.received = Received::at(at);
            }
            Message::Foreground { pane, process } => {
                if let Some(slot) = self.remote_slot(pane) {
                    slot.screen.foreground(process);
                }
            }
            Message::Exited { pane, code } => {
                if let Some(slot) = self.remote_slot(pane) {
                    slot.screen.exited(code);
                    slot.over = true;
                }
            }
            // Not ours any more, to close or to type into.
            Message::Detach { pane } => {
                self.disown(pane, "opened in another window");
            }
            Message::Gone { pane } => {
                self.disown(pane, "the program is gone from its machine");
            }
            Message::Connect {
                channel,
                service,
                opening,
            } => {
                let Some(live) = &self.live else { return };
                let found = self
                    .options
                    .services
                    .iter()
                    .find(|s| s.name() == service)
                    .cloned();
                match found {
                    Some(service) => live.relays.add(channel, move || service.open(&opening)),
                    // Not offered, or no longer.
                    None => self.send(Message::Hangup { channel }),
                }
            }
            Message::Data { channel, bytes } => {
                if let Some(live) = &self.live {
                    live.relays.data(channel, bytes);
                }
            }
            Message::Ack { channel, bytes } => {
                if let Some(live) = &self.live {
                    live.relays.ack(channel, bytes);
                }
            }
            Message::Hangup { channel } => {
                if let Some(live) = &self.live {
                    live.relays.hang_up(channel);
                }
            }
            // Only for being heard.
            Message::Pong { .. } => {}
            _ => {}
        }
    }

    /// Ends a pane that the agent's pane `pane` no longer stands for.
    fn disown(&mut self, pane: PaneId, why: &str) {
        let Some(id) = self.remotes.remove(&pane) else {
            return;
        };
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.remote = None;
            slot.end(State::Gone(why.into()));
        }
    }

    fn remote_slot(&mut self, pane: PaneId) -> Option<&mut Slot> {
        let id = self.remotes.get(&pane)?;
        self.slots.get_mut(id)
    }

    fn hello(&mut self, version: u32, instance: u64) {
        if version != VERSION {
            let why = format!(
                "the agent there speaks version {version} of the protocol, this Hof {VERSION}"
            );
            for slot in self.slots.values_mut() {
                slot.remote = None;
                slot.end(State::Gone(why.clone()));
            }
            self.remotes.clear();
            self.to_close.clear();
            self.hang_up();
            return;
        }
        if self.instance.is_some_and(|known| known != instance) {
            // It started again: the programs it ran went with it, and its
            // numbers for them mean others now.
            for slot in self.slots.values_mut().filter(|s| s.remote.is_some()) {
                slot.remote = None;
                slot.end(State::Gone("the agent there restarted".into()));
            }
            self.remotes.clear();
            self.to_close.clear();
        }
        self.instance = Some(instance);
        if let Some(live) = &mut self.live {
            live.greeted = true;
        }
        self.retry_wait = self.options.retry_first;
        self.retry_at = None;
        self.why = None;
        if !self.options.services.is_empty() {
            // Before any pane is opened: the agent sets what finds them in
            // the panes this connection opens.
            let services = self
                .options
                .services
                .iter()
                .map(|s| s.name().into())
                .collect();
            self.send(Message::Offer { services });
        }
        for pane in std::mem::take(&mut self.to_close) {
            self.send(Message::Close { pane });
        }
        let mut ids: Vec<u32> = self
            .slots
            .iter()
            .filter(|(_, s)| !s.over)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            let has_remote = self.slots[&id].remote.is_some();
            if has_remote {
                self.attach(id);
            } else {
                self.open(id);
            }
        }
    }
}

impl Slot {
    fn new(
        screen: Arc<dyn Screen>,
        remote: Option<PaneId>,
        start: Option<Start>,
        options: &Options,
    ) -> Self {
        let (cols, rows) = start.as_ref().map_or((0, 0), |s| (s.cols, s.rows));
        Self {
            screen,
            remote,
            start,
            opening: false,
            close_when_opened: false,
            received: Received::default(),
            input: Backlog::new(options.input_backlog),
            cols,
            rows,
            attached: false,
            over: false,
        }
    }

    fn end(&mut self, state: State) {
        self.over = true;
        self.attached = false;
        self.screen.state(state);
    }
}

fn read_frames(
    mut reader: Box<dyn Read + Send>,
    to: &mpsc::Sender<Event>,
    generation: u64,
    incoming: Arc<Budget>,
) {
    let mut decoder = Decoder::new();
    let mut buf = vec![0; 64 * 1024];
    let why = loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break None,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break Some(e.to_string()),
        };
        decoder.push(&buf[..n]);
        loop {
            match decoder.next_message() {
                None => break,
                Some(Ok(message)) => {
                    incoming.take(message_weight(&message));
                    if to.send(Event::Heard(generation, message)).is_err() {
                        return;
                    }
                }
                // Past a length that large nothing can be trusted.
                Some(Err(e @ crate::wire::Error::TooLarge(_))) => {
                    let _ = to.send(Event::Ended(generation, Some(e.to_string())));
                    return;
                }
                // One frame that makes no sense: the next may.
                Some(Err(_)) => {}
            }
        }
    };
    let _ = to.send(Event::Ended(generation, why));
}

/// An approximation of the memory a decoded message and its heap data hold.
fn message_weight(message: &Message) -> usize {
    const BASE: usize = 256;
    fn strings(values: &[String]) -> usize {
        values.iter().map(|s| s.len() + 24).sum::<usize>()
    }
    BASE + match message {
        Message::Hello { name, .. } => name.len(),
        Message::Open {
            command,
            directory,
            environment,
            ..
        } => strings(command) + directory.len() + strings(environment),
        Message::Refused { reason, .. } => reason.len(),
        Message::Output { bytes, .. }
        | Message::Snapshot { bytes, .. }
        | Message::Input { bytes, .. }
        | Message::Data { bytes, .. } => bytes.len(),
        Message::Panes { panes } => panes
            .iter()
            .map(|pane| 40 + strings(&pane.command))
            .sum::<usize>(),
        Message::Foreground { process, .. } => process
            .as_ref()
            .map_or(0, |process| process.name.len() + strings(&process.args)),
        Message::Offer { services } => strings(services),
        Message::Connect {
            service, opening, ..
        } => service.len() + opening.len(),
        _ => 0,
    }
}

fn write_frames(
    mut writer: Box<dyn Write + Send>,
    frames: &Lanes,
    to: &mpsc::Sender<Event>,
    generation: u64,
) {
    // Ends when the connection closes and everything before it has drained.
    // Keystrokes come one frame each; a flush per batch is enough.
    while let Some(batch) = frames.take(BULK_PER_WRITE) {
        if let Err(e) = writer.write_all(&batch).and_then(|()| writer.flush()) {
            let _ = to.send(Event::Ended(generation, Some(e.to_string())));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_incoming_budget_blocks_the_reader_until_the_driver_catches_up() {
        let budget = Arc::new(Budget::default());
        budget.take(EVENT_MEMORY);
        let waiting = {
            let budget = budget.clone();
            thread::spawn(move || budget.take(1))
        };
        thread::sleep(Duration::from_millis(20));
        assert!(
            !waiting.is_finished(),
            "the full budget applies backpressure"
        );
        budget.give(EVENT_MEMORY);
        waiting.join().unwrap();
        assert_eq!(message_weight(&Message::List), 256);
        assert_eq!(
            message_weight(&Message::Data {
                channel: 1,
                bytes: vec![0; 100],
            }),
            356
        );
    }

    #[test]
    fn a_stderr_line_without_an_end_stays_bounded() {
        let lines = Mutex::new(VecDeque::new());
        keep_last_lines(io::repeat(b'x').take((3 * MOST_LINE + 100) as u64), &lines);
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MOST_LINE);
    }
}
