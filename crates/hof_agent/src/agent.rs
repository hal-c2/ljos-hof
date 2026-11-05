//! The panes, and what the agent says to each connection about them.
//!
//! Each connection has two threads. The reader acts on what the client
//! sends. The writer sends what the client should get: answers, and the
//! output of the panes it is attached to. A pane's output goes into its
//! backlog as the program writes it, and wakes the writer of the
//! connection attached to it, which sends whatever that connection lacks.
//! A connection that falls behind further than the backlog reaches gets a
//! snapshot of the screen in place of what it missed, so a slow client
//! costs the agent no more memory than a fast one.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak, mpsc};
use std::thread;

use hof_remote::wire::{self, Decoder, Message, PaneId, PaneInfo, VERSION};
use hof_remote::{Backlog, Received, Relays, Resume};

use crate::forward::Forwards;
use hof_vt::{Event, Session, Size, Spawn};

/// The most output one message carries; more goes in several, so one
/// pane's flood doesn't hold up the others' for long.
const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    /// Bytes of each pane's newest output kept for a client that comes
    /// back. Past that, it gets a snapshot of the screen instead.
    pub backlog: usize,
    /// Lines of history each pane keeps above its screen.
    pub scrollback: usize,
    /// What the agent says it is: the machine's name.
    pub name: String,
    /// The `PATH` its panes get, when it is not the agent's own.
    pub path: Option<OsString>,
    /// This machine's sessions, for a Hof elsewhere to open
    /// (`hof attach --host`).
    pub sessions: hof_session::service::Places,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            backlog: 4 * 1024 * 1024,
            scrollback: 10_000,
            name: host_name(),
            path: path_with(
                std::env::current_exe().ok().as_deref(),
                std::env::var_os("PATH"),
            ),
            sessions: hof_session::service::Places::here(),
        }
    }
}

/// `path` with the directory of `program` after it, when `program` is a
/// `hof` that `path` doesn't reach: one Hof copied to the machine, so
/// `hof copy` works in its panes.
pub(crate) fn path_with(
    program: Option<&std::path::Path>,
    path: Option<OsString>,
) -> Option<OsString> {
    let program = program?;
    if program.file_name()? != "hof" {
        return None;
    }
    let dir = program.parent()?;
    let path = path.unwrap_or_default();
    if std::env::split_paths(&path).any(|p| p == dir || p.join("hof").exists()) {
        return None;
    }
    let dirs = std::env::split_paths(&path)
        .filter(|p| !p.as_os_str().is_empty())
        .chain([dir.to_owned()]);
    std::env::join_paths(dirs).ok()
}

fn host_name() -> String {
    rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned()
}

/// The panes on this machine, and the protocol for reaching them. Clones
/// share the panes.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

struct Inner {
    options: Options,
    instance: u64,
    panes: Mutex<BTreeMap<PaneId, Arc<Pane>>>,
    next_pane: AtomicU32,
    next_connection: AtomicU64,
    connections: AtomicUsize,
    /// Where a pane's session says it has news (its end, mostly).
    changed: mpsc::Sender<PaneId>,
    /// The X11 and sound clients here, passed on to Hof.
    forwards: Arc<Forwards>,
}

struct Pane {
    command: Vec<String>,
    session: Session,
    /// Shared with the session's tap, which fills it.
    stream: Arc<Mutex<Stream>>,
    /// How much of the client's input reached the program.
    input: Mutex<Received>,
    /// How it ended, once it has.
    exited: Mutex<Option<Option<i32>>>,
}

struct Stream {
    backlog: Backlog,
    /// The connection the output goes to, if one is attached.
    watcher: Option<Watcher>,
    /// A wake is on its way to the watcher; another would say nothing new.
    woken: bool,
}

impl Stream {
    fn wake(&mut self, pane: PaneId) {
        if let Some(w) = &self.watcher
            && !self.woken
        {
            self.woken = w.wake.send(Wake::Pane(pane)).is_ok();
        }
    }
}

struct Watcher {
    connection: u64,
    wake: mpsc::Sender<Wake>,
}

/// What a connection's writer is asked to do.
enum Wake {
    Send(Message),
    /// Send the pane's output from `have` on, and keep sending it.
    Attach {
        pane: PaneId,
        have: u64,
    },
    /// Stop sending the pane's output.
    Detach(PaneId),
    /// The pane has news: output, or its end.
    Pane(PaneId),
}

/// How far a writer got with one attached pane.
struct Sent {
    at: u64,
    told_exit: bool,
    /// The foreground process last told; `None` until told.
    told_foreground: Option<Option<wire::Process>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere left the data as it was; the agent keeps serving.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Agent {
    pub fn new(options: Options) -> Self {
        let (changed, news) = mpsc::channel();
        let instance = instance();
        let inner = Arc::new(Inner {
            options,
            instance,
            panes: Mutex::default(),
            // Not from 1: a saved session names panes by number, and an
            // agent started again must not have the same numbers for other
            // programs. Small enough for QML's numbers to hold exactly.
            next_pane: AtomicU32::new(1000 + (instance % 1_000_000_000) as u32),
            next_connection: AtomicU64::new(1),
            connections: AtomicUsize::new(0),
            changed,
            forwards: Arc::new(Forwards::new(instance)),
        });
        let weak = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("hof-agent-news".into())
            .spawn(move || watch_news(&weak, &news))
            .expect("a thread for the panes' news");
        Self { inner }
    }

    /// Different for every agent: what tells a client its panes are gone.
    pub fn instance(&self) -> u64 {
        self.inner.instance
    }

    /// Takes away the files the agent made for its panes' programs (the
    /// X11 authorization, the sound socket): it is going.
    pub fn remove_files(&self) {
        self.inner.forwards.remove_files();
    }

    /// No connection is open and no pane is left: nothing to stay for.
    pub fn is_idle(&self) -> bool {
        self.inner.connections.load(Ordering::Acquire) == 0 && lock(&self.inner.panes).is_empty()
    }

    /// Speaks the protocol to one client, until it hangs up. Its panes
    /// carry on.
    pub fn serve<R, W>(&self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: Read,
        W: Write + Send + 'static,
    {
        let me = self.inner.next_connection.fetch_add(1, Ordering::Relaxed);
        self.inner.connections.fetch_add(1, Ordering::AcqRel);
        let (wake, wakes) = mpsc::channel();
        let inner = self.inner.clone();
        let writing = thread::Builder::new()
            .name("hof-agent-write".into())
            .spawn(move || write_loop(&inner, &wakes, writer))?;
        let relaying = wake.clone();
        let relays = Relays::new(move |m, _| relaying.send(Wake::Send(m)).is_ok());

        let result = self.read_loop(me, &wake, &relays, &mut reader);

        // Its channels go with it, and let go of the writer.
        self.inner.forwards.withdraw(me);
        relays.close_all();
        // Stop the output, so that the writer's last sender is ours.
        for pane in lock(&self.inner.panes).values() {
            let mut stream = lock(&pane.stream);
            if stream.watcher.as_ref().is_some_and(|w| w.connection == me) {
                stream.watcher = None;
            }
        }
        drop(wake);
        let _ = writing.join();
        self.inner.connections.fetch_sub(1, Ordering::AcqRel);
        result
    }

    fn read_loop(
        &self,
        me: u64,
        wake: &mpsc::Sender<Wake>,
        relays: &Relays,
        reader: &mut impl Read,
    ) -> io::Result<()> {
        let mut decoder = Decoder::new();
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            decoder.push(&buf[..n]);
            while let Some(message) = decoder.next_message() {
                match message {
                    Ok(message) => self.handle(me, wake, relays, message),
                    Err(e @ wire::Error::TooLarge(_)) => {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, e));
                    }
                    // That frame only; the next is fine.
                    Err(_) => {}
                }
            }
        }
    }

    fn handle(&self, me: u64, wake: &mpsc::Sender<Wake>, relays: &Relays, message: Message) {
        let send = |m: Message| {
            let _ = wake.send(Wake::Send(m));
        };
        let pane = |id: PaneId| {
            let found = self.pane(id);
            if found.is_none() {
                send(Message::Gone { pane: id });
            }
            found
        };
        match message {
            Message::Hello { .. } => send(Message::Hello {
                version: VERSION,
                name: self.inner.options.name.clone(),
                instance: self.inner.instance,
            }),
            Message::Open {
                request,
                command,
                directory,
                environment,
                cols,
                rows,
            } => send(
                match self.open(me, command, &directory, &environment, cols, rows) {
                    Ok(pane) => Message::Opened { request, pane },
                    Err(reason) => Message::Refused { request, reason },
                },
            ),
            Message::Attach { pane: id, have } => {
                let Some(p) = pane(id) else { return };
                let taken = lock(&p.stream).watcher.replace(Watcher {
                    connection: me,
                    wake: wake.clone(),
                });
                // Whoever watched it before has lost it: a client that came
                // back before the agent saw it go, or another machine.
                if let Some(old) = taken
                    && old.connection != me
                {
                    let _ = old.wake.send(Wake::Detach(id));
                    let _ = old.wake.send(Wake::Send(Message::Detach { pane: id }));
                }
                let _ = wake.send(Wake::Attach { pane: id, have });
            }
            Message::Detach { pane: id } => {
                let Some(p) = pane(id) else { return };
                let mut stream = lock(&p.stream);
                if stream.watcher.as_ref().is_some_and(|w| w.connection == me) {
                    stream.watcher = None;
                }
                drop(stream);
                let _ = wake.send(Wake::Detach(id));
            }
            Message::Input {
                pane: id,
                at,
                bytes,
            } => {
                let Some(p) = pane(id) else { return };
                let mut input = lock(&p.input);
                // Sent again after a reconnect: only what is new is typed.
                if let Ok(new) = input.take(at, &bytes) {
                    p.session.write(new);
                }
            }
            Message::Resize {
                pane: id,
                cols,
                rows,
            } => {
                if let Some(p) = pane(id) {
                    let _ = p.session.resize(Size::new(cols, rows));
                }
            }
            Message::Close { pane: id } => {
                // Dropped outside the lock: that hangs up on the program.
                let gone = lock(&self.inner.panes).remove(&id);
                drop(gone);
            }
            Message::List => send(Message::Panes {
                panes: lock(&self.inner.panes)
                    .iter()
                    .map(|(&id, p)| {
                        let size = p.session.size();
                        PaneInfo {
                            pane: id,
                            command: p.command.clone(),
                            cols: size.cols,
                            rows: size.rows,
                            exited: *lock(&p.exited),
                        }
                    })
                    .collect(),
            }),
            Message::Ping { nonce } => send(Message::Pong { nonce }),
            Message::Offer { services } => {
                self.inner.forwards.offer(me, services, relays.clone());
            }
            Message::Data { channel, bytes } => relays.data(channel, bytes),
            Message::Ack { channel, bytes } => relays.ack(channel, bytes),
            Message::Hangup { channel } => relays.hang_up(channel),
            // Hof reaching this machine's files.
            Message::Connect {
                channel, service, ..
            } if service == hof_remote::files::SERVICE => {
                relays.add(channel, hof_remote::files::open_server);
            }
            // The other connections the agent takes; Hof only passes them on.
            Message::Connect {
                channel, service, ..
            } if service == hof_session::service::SERVICE => {
                let places = self.inner.options.sessions.clone();
                relays.add(channel, move || open_sessions(places));
            }
            Message::Connect { channel, .. } => send(Message::Hangup { channel }),
            // What the agent sends, not what it is sent, and what it can't
            // read: nothing to do.
            Message::Opened { .. }
            | Message::Refused { .. }
            | Message::Attached { .. }
            | Message::Output { .. }
            | Message::Snapshot { .. }
            | Message::Exited { .. }
            | Message::Panes { .. }
            | Message::Pong { .. }
            | Message::Gone { .. }
            | Message::Foreground { .. }
            | Message::Unknown { .. } => {}
        }
    }

    fn pane(&self, id: PaneId) -> Option<Arc<Pane>> {
        lock(&self.inner.panes).get(&id).cloned()
    }

    fn open(
        &self,
        me: u64,
        command: Vec<String>,
        directory: &str,
        environment: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<PaneId, String> {
        let id = self.inner.next_pane.fetch_add(1, Ordering::Relaxed);
        let mut spawn = match command.split_first() {
            None => Spawn::shell(),
            Some((program, args)) => Spawn::command(program, args),
        };
        spawn.cwd = if directory.is_empty() {
            std::env::var_os("HOME").map(PathBuf::from)
        } else {
            Some(PathBuf::from(directory))
        };
        // What finds Hof's services first, so Hof's own say wins.
        let path = self.inner.options.path.clone();
        spawn.env = path
            .map(|p| (OsString::from("PATH"), p))
            .into_iter()
            .chain(
                self.inner
                    .forwards
                    .environment(me)
                    .into_iter()
                    .map(|(k, v)| (OsString::from(k), OsString::from(v))),
            )
            .chain(
                environment
                    .iter()
                    .filter_map(|v| v.split_once('='))
                    .map(|(k, v)| (OsString::from(k), OsString::from(v))),
            )
            .collect();

        let stream = Arc::new(Mutex::new(Stream {
            backlog: Backlog::new(self.inner.options.backlog),
            watcher: None,
            woken: false,
        }));
        let tapped = stream.clone();
        let tap: hof_vt::Tap = Box::new(move |bytes| {
            let mut stream = lock(&tapped);
            stream.backlog.push(bytes);
            stream.wake(id);
        });
        let changed = self.inner.changed.clone();
        let options = hof_vt::Options {
            size: Size::new(cols, rows),
            scrollback: self.inner.options.scrollback,
            ..hof_vt::Options::default()
        };
        let session = Session::spawn_tapped(
            &spawn,
            options,
            move || {
                let _ = changed.send(id);
            },
            Some(tap),
        )
        .map_err(|e| e.to_string())?;
        lock(&self.inner.panes).insert(
            id,
            Arc::new(Pane {
                command,
                session,
                stream,
                input: Mutex::default(),
                exited: Mutex::new(None),
            }),
        );
        Ok(id)
    }
}

/// A number no other agent is likely to have picked.
fn instance() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    nanos ^ (u64::from(std::process::id()) << 32)
}

/// Takes each pane's news as its session posts it. The session wakes again
/// only once its news is taken, and its end is news a client needs.
fn watch_news(inner: &Weak<Inner>, news: &mpsc::Receiver<PaneId>) {
    for id in news {
        let Some(inner) = inner.upgrade() else { return };
        let Some(pane) = lock(&inner.panes).get(&id).cloned() else {
            continue;
        };
        pane.session.take_changes();
        for event in pane.session.take_events() {
            if let Event::Exited(code) = event {
                // Before the wake, which the writer answers by reading it.
                *lock(&pane.exited) = Some(code);
                lock(&pane.stream).wake(id);
            }
        }
    }
}

/// The most relay traffic written between looks at the panes: one piece.
const BULK_PER_WRITE: usize = 64 * 1024;

/// Writes what the connection's panes and relays send. What a relay carries
/// goes behind the panes' output, a piece at a time, so a file or an X11
/// image on its way doesn't hold up what a pane shows; each relay's own
/// messages stay in order. An acknowledgement is small, and lets Hof send
/// on: it goes with the panes'.
fn write_loop(inner: &Inner, wakes: &mpsc::Receiver<Wake>, mut out: impl Write) {
    let mut sent: HashMap<PaneId, Sent> = HashMap::new();
    let mut frames = Vec::new();
    let mut bulk: VecDeque<Message> = VecDeque::new();
    let mut open = true;
    loop {
        // Waits for news only with no bulk left to send.
        let mut next = if bulk.is_empty() {
            match wakes.recv() {
                Ok(wake) => Some(wake),
                Err(_) => return,
            }
        } else {
            None
        };
        // Whatever else is waiting goes in the same write.
        loop {
            let wake = match next.take() {
                Some(wake) => wake,
                None if !open => break,
                None => match wakes.try_recv() {
                    Ok(wake) => wake,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        open = false;
                        break;
                    }
                },
            };
            match wake {
                Wake::Send(
                    message @ (Message::Data { .. }
                    | Message::Connect { .. }
                    | Message::Hangup { .. }),
                ) => bulk.push_back(message),
                wake => serve_wake(inner, &mut sent, wake, &mut frames),
            }
        }
        let start = frames.len();
        while frames.len() - start < BULK_PER_WRITE
            && let Some(message) = bulk.pop_front()
        {
            message.encode(&mut frames);
        }
        if out.write_all(&frames).and_then(|()| out.flush()).is_err() {
            return;
        }
        frames.clear();
        if !open && bulk.is_empty() {
            return;
        }
    }
}

fn serve_wake(inner: &Inner, sent: &mut HashMap<PaneId, Sent>, wake: Wake, out: &mut Vec<u8>) {
    let pane = |id| lock(&inner.panes).get(&id).cloned();
    match wake {
        Wake::Send(message) => message.encode(out),
        Wake::Attach { pane: id, have } => {
            let Some(p) = pane(id) else {
                Message::Gone { pane: id }.encode(out);
                return;
            };
            let size = p.session.size();
            Message::Attached {
                pane: id,
                input: lock(&p.input).end(),
                cols: size.cols,
                rows: size.rows,
            }
            .encode(out);
            let at = sent.entry(id).insert_entry(Sent {
                at: have,
                told_exit: false,
                told_foreground: None,
            });
            catch_up(id, &p, at.into_mut(), out);
        }
        Wake::Detach(id) => {
            sent.remove(&id);
        }
        Wake::Pane(id) => match (pane(id), sent.get_mut(&id)) {
            (Some(p), Some(at)) => catch_up(id, &p, at, out),
            // Closed since.
            (None, _) => drop(sent.remove(&id)),
            // Not attached here (any more): nothing to send.
            (Some(_), None) => {}
        },
    }
}

/// Sends the client what it lacks of `pane`'s output, and its end once
/// that is all sent.
fn catch_up(id: PaneId, pane: &Pane, sent: &mut Sent, out: &mut Vec<u8>) {
    // In this order. Cleared first, so news from here on wakes again. The
    // end read before the output: once the program has ended, all of its
    // output is in, so the end is never sent ahead of any of it.
    lock(&pane.stream).woken = false;
    let exited = *lock(&pane.exited);
    // Before the output: a program writes once it has the foreground, so
    // what it says of itself in that output is heard with it in front. The
    // whole process is compared, not its group: `exec` keeps the number.
    let process = pane.session.foreground().map(|p| wire::Process {
        pid: p.pid,
        name: p.name,
        args: p.args,
    });
    if sent.told_foreground.as_ref() != Some(&process) {
        sent.told_foreground = Some(process.clone());
        Message::Foreground { pane: id, process }.encode(out);
    }
    let resume = lock(&pane.stream).backlog.resume(sent.at);
    match resume {
        Resume::Rest(bytes) => {
            for chunk in bytes.chunks(CHUNK) {
                Message::Output {
                    pane: id,
                    at: sent.at,
                    bytes: chunk.to_vec(),
                }
                .encode(out);
                sent.at += chunk.len() as u64;
            }
        }
        // Missed more than is kept, or has a stream that is not this one:
        // the screen as it is stands for all of it.
        Resume::Lost | Resume::Ahead => {
            let snapshot = pane
                .session
                .snapshot_with(|| lock(&pane.stream).backlog.end());
            if let Ok((bytes, at)) = snapshot {
                Message::Snapshot {
                    pane: id,
                    at,
                    bytes,
                }
                .encode(out);
                sent.at = at;
            }
        }
    }
    if let Some(code) = exited
        && !sent.told_exit
    {
        Message::Exited { pane: id, code }.encode(out);
        sent.told_exit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_copied_hof_puts_its_directory_last_on_the_path() {
        let path = path_with(
            Some(Path::new("/nowhere/hof/0.1/hof")),
            Some("/usr/bin:/bin".into()),
        );
        assert_eq!(path, Some("/usr/bin:/bin:/nowhere/hof/0.1".into()));
    }

    #[test]
    fn a_hof_the_path_reaches_leaves_the_path_alone() {
        let on = path_with(Some(Path::new("/opt/x/hof")), Some("/bin:/opt/x".into()));
        assert_eq!(on, None);
        let other = path_with(Some(Path::new("/opt/x/hof-agent")), Some("/bin".into()));
        assert_eq!(other, None, "only a program called hof");
    }

    /// Each write the writer makes, apart.
    #[derive(Default)]
    struct Writes(Vec<Vec<u8>>);

    impl Write for &mut Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.push(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn what_the_panes_need_goes_ahead_of_relay_data_sent_before_it() {
        let agent = Agent::new(Options::default());
        let (wake, wakes) = mpsc::channel();
        for n in 0..8u8 {
            let bytes = vec![n; 60 * 1024];
            wake.send(Wake::Send(Message::Data { channel: 1, bytes }))
                .unwrap();
        }
        wake.send(Wake::Send(Message::Hangup { channel: 1 }))
            .unwrap();
        wake.send(Wake::Send(Message::Pong { nonce: 7 })).unwrap();
        drop(wake);
        let mut writes = Writes::default();
        write_loop(&agent.inner, &wakes, &mut writes);

        assert!(writes.0.len() > 2, "the data went a piece at a time");
        let mut decoder = Decoder::new();
        decoder.push(&writes.0.concat());
        let mut heard = Vec::new();
        while let Some(message) = decoder.next_message() {
            heard.push(message.unwrap());
        }
        assert!(
            matches!(heard[0], Message::Pong { nonce: 7 }),
            "{:?}",
            heard[0]
        );
        // The relay's own, as they were sent.
        for (n, message) in heard[1..9].iter().enumerate() {
            assert!(
                matches!(message, Message::Data { bytes, .. } if bytes[0] == n as u8),
                "{n}"
            );
        }
        assert!(matches!(heard[9], Message::Hangup { channel: 1 }));
        assert_eq!(heard.len(), 10);
    }
}

/// The agent's end of a new sessions channel: a socket whose other end a
/// thread serves from `places`.
fn open_sessions(
    places: hof_session::service::Places,
) -> io::Result<(Box<dyn hof_remote::Socket>, Vec<u8>)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    std::thread::Builder::new()
        .name("hof-sessions".into())
        .spawn(move || {
            let _ = hof_session::service::serve(theirs, &places);
        })?;
    Ok((Box::new(ours), Vec::new()))
}
