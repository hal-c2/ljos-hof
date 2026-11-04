//! Connections relayed over the connection between Hof and the agent: an
//! X11 client's, a sound client's, each a channel of its own beside the
//! panes.
//!
//! Each end of a channel is a socket on its machine. What one end's socket
//! says goes to the other as [`Message::Data`], and is written to the
//! socket there. The writing end acknowledges what it wrote, and a reader
//! has at most [`WINDOW`] bytes on the way that are not acknowledged. So a
//! program that floods its channel waits for the slow side, as it would
//! on its own socket, instead of piling up in memory, and the panes'
//! output keeps its turn on the connection.
//!
//! A channel ends when either socket does: the end that saw it says
//! [`Message::Hangup`], and the other closes its socket once it has
//! written what came before. All of a connection's channels end with it,
//! as `ssh -X`'s do: the programs at the far end see their server go.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

use crate::wire::{ChannelId, Message};

/// The most of a channel's data on the way and not acknowledged: enough
/// to keep a link with a long round trip busy.
pub const WINDOW: usize = 1 << 20;

/// The most one message carries.
const PIECE: usize = 64 * 1024;

/// A connected socket, of whatever kind.
pub trait Socket: Read + Write + Send + 'static {
    /// Another handle to the same socket, for reading on one thread and
    /// writing on another.
    fn another(&self) -> io::Result<Box<dyn Socket>>;
    /// Ends it both ways, for every handle.
    fn close(&self);
    /// How long a read may wait; `None` for as long as it takes.
    fn read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

impl Socket for UnixStream {
    fn another(&self) -> io::Result<Box<dyn Socket>> {
        Ok(Box::new(self.try_clone()?))
    }

    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }

    fn read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(timeout)
    }
}

impl Socket for TcpStream {
    fn another(&self) -> io::Result<Box<dyn Socket>> {
        Ok(Box::new(self.try_clone()?))
    }

    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }

    fn read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(timeout)
    }
}

type Sender = Box<dyn Fn(Message) -> bool + Send + Sync>;

/// One connection's channels. Clones share them.
#[derive(Clone)]
pub struct Relays(Arc<Shared>);

struct Shared {
    /// How messages go to the other end; gone once the connection is.
    send: Mutex<Option<Sender>>,
    channels: Mutex<HashMap<ChannelId, Arc<Channel>>>,
}

struct Channel {
    /// Once it is connected.
    socket: Mutex<Option<Box<dyn Socket>>>,
    /// To the thread that writes to the socket; taken when the other end
    /// hangs up, so the writer ends once it has written what came.
    to_write: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    /// Read and sent, and not acknowledged yet.
    unacked: Mutex<usize>,
    room: Condvar,
    closed: AtomicBool,
}

impl Channel {
    fn close(&self) {
        // Before the socket is looked at: one connected after this sees it.
        self.closed.store(true, Ordering::Release);
        if let Some(socket) = lock(&self.socket).as_ref() {
            socket.close();
        }
        // Under the lock the reader waits with, so it can't miss it.
        let _unacked = lock(&self.unacked);
        self.room.notify_all();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Relays {
    /// Channels whose messages go out through `send`, which says whether
    /// it could send.
    pub fn new(send: impl Fn(Message) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared {
            send: Mutex::new(Some(Box::new(send))),
            channels: Mutex::default(),
        }))
    }

    fn send(&self, message: Message) -> bool {
        lock(&self.0.send)
            .as_ref()
            .is_some_and(|send| send(message))
    }

    /// A program here connected to `service` and began with `opening`:
    /// tells the other end, and relays the rest as `channel`.
    pub fn connect_out(
        &self,
        channel: ChannelId,
        service: &str,
        opening: Vec<u8>,
        socket: Box<dyn Socket>,
    ) {
        let told = self.send(Message::Connect {
            channel,
            service: service.into(),
            opening,
        });
        if !told {
            socket.close();
            return;
        }
        self.add(channel, move || Ok((socket, Vec::new())));
    }

    /// Relays `channel` to the socket `connect` makes, and writes what it
    /// gives with it there first. What comes for the channel while it
    /// connects waits; if it can't, the channel ends.
    pub fn add<F>(&self, channel: ChannelId, connect: F)
    where
        F: FnOnce() -> io::Result<(Box<dyn Socket>, Vec<u8>)> + Send + 'static,
    {
        let (to_write, written) = mpsc::channel();
        let ch = Arc::new(Channel {
            socket: Mutex::new(None),
            to_write: Mutex::new(Some(to_write)),
            unacked: Mutex::new(0),
            room: Condvar::new(),
            closed: AtomicBool::new(false),
        });
        if let Some(old) = lock(&self.0.channels).insert(channel, ch.clone()) {
            old.close();
        }
        let relays = self.clone();
        let started = thread::Builder::new()
            .name("hof-relay-write".into())
            .spawn(move || relays.run(channel, &ch, connect, &written));
        if started.is_err() {
            self.end(channel);
        }
    }

    /// Connects, starts the reader, and writes what comes for the channel
    /// until the other end hangs up or the socket fails.
    fn run<F>(
        &self,
        id: ChannelId,
        channel: &Arc<Channel>,
        connect: F,
        written: &mpsc::Receiver<Vec<u8>>,
    ) where
        F: FnOnce() -> io::Result<(Box<dyn Socket>, Vec<u8>)>,
    {
        let Ok((socket, first)) = connect() else {
            self.end(id);
            return;
        };
        let (Ok(reader), Ok(mut writer)) = (socket.another(), socket.another()) else {
            socket.close();
            self.end(id);
            return;
        };
        *lock(&channel.socket) = Some(socket);
        if channel.closed.load(Ordering::Acquire) {
            // Ended while it connected.
            channel.close();
            return;
        }
        let relays = self.clone();
        let ch = channel.clone();
        let reading = thread::Builder::new()
            .name("hof-relay-read".into())
            .spawn(move || relays.read(id, &ch, reader));
        if reading.is_err() {
            self.end(id);
            return;
        }
        let mut failed = !first.is_empty() && writer.write_all(&first).is_err();
        while !failed && let Ok(bytes) = written.recv() {
            if writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .is_err()
            {
                failed = true;
                break;
            }
            self.send(Message::Ack {
                channel: id,
                bytes: bytes.len() as u32,
            });
        }
        if failed {
            self.end(id);
        }
        // Hung up at the other end, and all of it written: or failed.
        channel.close();
        if let Ok(reading) = reading {
            let _ = reading.join();
        }
    }

    fn read(&self, id: ChannelId, channel: &Channel, mut reader: Box<dyn Socket>) {
        let mut buf = vec![0; PIECE];
        loop {
            let room = {
                let mut unacked = lock(&channel.unacked);
                while *unacked >= WINDOW && !channel.closed.load(Ordering::Acquire) {
                    unacked = channel
                        .room
                        .wait(unacked)
                        .unwrap_or_else(|e| e.into_inner());
                }
                if channel.closed.load(Ordering::Acquire) {
                    return;
                }
                WINDOW - *unacked
            };
            let n = match reader.read(&mut buf[..room.min(PIECE)]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            *lock(&channel.unacked) += n;
            let sent = self.send(Message::Data {
                channel: id,
                bytes: buf[..n].to_vec(),
            });
            if !sent {
                break;
            }
        }
        self.end(id);
    }

    /// The channel ended here: the other end is told, if it is still open.
    fn end(&self, id: ChannelId) {
        let Some(channel) = lock(&self.0.channels).remove(&id) else {
            return;
        };
        self.send(Message::Hangup { channel: id });
        lock(&channel.to_write).take();
        channel.close();
    }

    /// The other end's [`Message::Data`].
    pub fn data(&self, id: ChannelId, bytes: Vec<u8>) {
        let channel = lock(&self.0.channels).get(&id).cloned();
        match channel {
            Some(channel) => {
                if let Some(to_write) = lock(&channel.to_write).as_ref() {
                    let _ = to_write.send(bytes);
                }
            }
            // Ended here, and the hangup is on its way; or never was.
            None => {
                self.send(Message::Hangup { channel: id });
            }
        }
    }

    /// The other end's [`Message::Ack`].
    pub fn ack(&self, id: ChannelId, bytes: u32) {
        let channel = lock(&self.0.channels).get(&id).cloned();
        if let Some(channel) = channel {
            let mut unacked = lock(&channel.unacked);
            *unacked = unacked.saturating_sub(bytes as usize);
            channel.room.notify_all();
        }
    }

    /// The other end's [`Message::Hangup`]: the socket here is closed once
    /// what came before is written.
    pub fn hang_up(&self, id: ChannelId) {
        let channel = lock(&self.0.channels).remove(&id);
        if let Some(channel) = channel {
            lock(&channel.to_write).take();
            // Stop reading at once: nothing more is wanted from it.
            channel.closed.store(true, Ordering::Release);
            let _unacked = lock(&channel.unacked);
            channel.room.notify_all();
        }
    }

    /// The connection is gone: every channel ends at once, and nothing
    /// more is sent.
    pub fn close_all(&self) {
        lock(&self.0.send).take();
        let channels: Vec<_> = lock(&self.0.channels).drain().map(|(_, c)| c).collect();
        for channel in channels {
            lock(&channel.to_write).take();
            channel.close();
        }
    }

    /// How many channels are open.
    pub fn len(&self) -> usize {
        lock(&self.0.channels).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two ends whose messages go to each other, as over a connection.
    fn pair() -> (Relays, Relays) {
        let (to_a, for_a) = mpsc::channel::<Message>();
        let (to_b, for_b) = mpsc::channel::<Message>();
        let a = Relays::new(move |m| to_b.send(m).is_ok());
        let b = Relays::new(move |m| to_a.send(m).is_ok());
        deliver(&a, for_a);
        deliver(&b, for_b);
        (a, b)
    }

    fn deliver(to: &Relays, messages: mpsc::Receiver<Message>) {
        let to = to.clone();
        thread::spawn(move || {
            for m in messages {
                match m {
                    Message::Connect { channel, .. } => {
                        panic!("the test connects channel {channel} itself")
                    }
                    Message::Data { channel, bytes } => to.data(channel, bytes),
                    Message::Ack { channel, bytes } => to.ack(channel, bytes),
                    Message::Hangup { channel } => to.hang_up(channel),
                    other => panic!("not a relay's: {other:?}"),
                }
            }
        });
    }

    /// A channel between `a` and `b`, and the far ends of its sockets.
    fn channel(a: &Relays, b: &Relays, first: &[u8]) -> (UnixStream, UnixStream) {
        let (here, at_a) = UnixStream::pair().unwrap();
        let (there, at_b) = UnixStream::pair().unwrap();
        for s in [&here, &there] {
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        }
        let first = first.to_vec();
        b.add(1, move || Ok((Box::new(at_b) as Box<dyn Socket>, first)));
        a.add(1, move || {
            Ok((Box::new(at_a) as Box<dyn Socket>, Vec::new()))
        });
        (here, there)
    }

    fn read_n(from: &mut UnixStream, n: usize) -> Vec<u8> {
        let mut got = vec![0; n];
        from.read_exact(&mut got).unwrap();
        got
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn what_each_end_says_reaches_the_other_after_the_opening() {
        let (a, b) = pair();
        let (mut here, mut there) = channel(&a, &b, b"opening ");
        here.write_all(b"hello").unwrap();
        assert_eq!(read_n(&mut there, 13), b"opening hello");
        there.write_all(b"and back").unwrap();
        assert_eq!(read_n(&mut here, 8), b"and back");
    }

    #[test]
    fn more_than_the_window_gets_through_and_in_order() {
        let (a, b) = pair();
        let (mut here, mut there) = channel(&a, &b, b"");
        let data: Vec<u8> = (0..WINDOW * 3 + 7).map(|i| (i % 251) as u8).collect();
        let sending = data.clone();
        let writer = thread::spawn(move || here.write_all(&sending).map(|()| here));
        let got = read_n(&mut there, data.len());
        assert!(got == data, "the bytes came as they went");
        writer.join().unwrap().unwrap();
    }

    #[test]
    fn a_socket_that_ends_ends_the_channel_at_both_ends_after_what_it_said() {
        let (a, b) = pair();
        let (mut here, mut there) = channel(&a, &b, b"");
        here.write_all(b"last words").unwrap();
        drop(here);
        let mut rest = Vec::new();
        there.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"last words");
        wait_until("both ends to forget it", || a.is_empty() && b.is_empty());
    }

    #[test]
    fn a_connection_that_goes_ends_its_channels() {
        let (a, b) = pair();
        let (mut here, mut there) = channel(&a, &b, b"");
        // Each end sees the connection go, and says nothing more.
        a.close_all();
        b.close_all();
        let mut rest = Vec::new();
        assert_eq!(here.read_to_end(&mut rest).unwrap(), 0);
        assert_eq!(there.read_to_end(&mut rest).unwrap(), 0);
        assert!(a.is_empty() && b.is_empty());
    }

    #[test]
    fn a_socket_that_cannot_be_reached_ends_the_channel() {
        let (a, b) = pair();
        let (here, at_a) = UnixStream::pair().unwrap();
        here.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        b.add(2, || Err(io::Error::other("no X server here")));
        a.add(2, move || {
            Ok((Box::new(at_a) as Box<dyn Socket>, Vec::new()))
        });
        let mut rest = Vec::new();
        (&here).read_to_end(&mut rest).unwrap();
        wait_until("both ends to forget it", || a.is_empty() && b.is_empty());
    }
}
