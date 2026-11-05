//! Hof's own sound server: enough of PulseAudio's native protocol for
//! programs to play through one sink, this machine's speakers.
//!
//! A client says who it is, asks what the server and its sink are, and
//! opens playback streams. The server asks each stream for as many bytes
//! as its buffer is short of, and the client sends them on the stream's
//! channel. Recording, uploaded samples and modules are not served; a
//! client asking for them is told there are none.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::mixer::{self, Mixer, Setup};
use super::tagstruct::{Malformed, Reader, SampleSpec, Writer};
use crate::client::Service;
use crate::pulse::{DESCRIPTOR, NotOpening, SERVICE, packet_len};
use crate::relay::Socket;

/// The protocol version served: what PulseAudio 12 speaks, which every
/// libpulse since speaks too.
const VERSION: u32 = 32;
/// The oldest version served: the first with property lists.
const OLDEST: u32 = 13;

/// The channel control packets go on.
const CONTROL: u32 = u32::MAX;
/// What is not an index.
const INVALID: u32 = u32::MAX;
/// The most a packet may hold.
const MOST_PACKET: usize = 16 * 1024 * 1024;
/// The volume that leaves a sample as it is.
const NORM: u32 = 0x10000;
/// What the sink's own latency is said to be, in microseconds.
const SINK_LATENCY: u64 = 25_000;
/// The sink's name; its index is 0.
const SINK: &str = "hof";

// Commands.
const ERROR: u32 = 0;
const REPLY: u32 = 2;
const CREATE_PLAYBACK_STREAM: u32 = 3;
const DELETE_PLAYBACK_STREAM: u32 = 4;
const CREATE_RECORD_STREAM: u32 = 5;
const AUTH: u32 = 8;
const SET_CLIENT_NAME: u32 = 9;
const LOOKUP_SINK: u32 = 10;
const LOOKUP_SOURCE: u32 = 11;
const DRAIN_PLAYBACK_STREAM: u32 = 12;
const STAT: u32 = 13;
const GET_PLAYBACK_LATENCY: u32 = 14;
const CREATE_UPLOAD_STREAM: u32 = 15;
const PLAY_SAMPLE: u32 = 18;
const GET_SERVER_INFO: u32 = 20;
const GET_SINK_INFO: u32 = 21;
const GET_SINK_INFO_LIST: u32 = 22;
const GET_SOURCE_INFO: u32 = 23;
const GET_SOURCE_INFO_LIST: u32 = 24;
const GET_MODULE_INFO: u32 = 25;
const GET_MODULE_INFO_LIST: u32 = 26;
const GET_CLIENT_INFO: u32 = 27;
const GET_CLIENT_INFO_LIST: u32 = 28;
const GET_SINK_INPUT_INFO: u32 = 29;
const GET_SINK_INPUT_INFO_LIST: u32 = 30;
const GET_SOURCE_OUTPUT_INFO: u32 = 31;
const GET_SOURCE_OUTPUT_INFO_LIST: u32 = 32;
const GET_SAMPLE_INFO: u32 = 33;
const GET_SAMPLE_INFO_LIST: u32 = 34;
const SUBSCRIBE: u32 = 35;
const SET_SINK_VOLUME: u32 = 36;
const SET_SINK_INPUT_VOLUME: u32 = 37;
const SET_SINK_MUTE: u32 = 39;
const CORK_PLAYBACK_STREAM: u32 = 41;
const FLUSH_PLAYBACK_STREAM: u32 = 42;
const TRIGGER_PLAYBACK_STREAM: u32 = 43;
const SET_DEFAULT_SINK: u32 = 44;
const SET_DEFAULT_SOURCE: u32 = 45;
const SET_PLAYBACK_STREAM_NAME: u32 = 46;
const PREBUF_PLAYBACK_STREAM: u32 = 60;
const REQUEST: u32 = 61;
const SET_SINK_INPUT_MUTE: u32 = 69;
const SUSPEND_SINK: u32 = 70;
const SET_PLAYBACK_STREAM_BUFFER_ATTR: u32 = 72;
const UPDATE_PLAYBACK_STREAM_SAMPLE_RATE: u32 = 74;
const UPDATE_RECORD_STREAM_PROPLIST: u32 = 80;
const UPDATE_PLAYBACK_STREAM_PROPLIST: u32 = 81;
const UPDATE_CLIENT_PROPLIST: u32 = 82;
const REMOVE_RECORD_STREAM_PROPLIST: u32 = 83;
const REMOVE_PLAYBACK_STREAM_PROPLIST: u32 = 84;
const REMOVE_CLIENT_PROPLIST: u32 = 85;
const STARTED: u32 = 86;
const EXTENSION: u32 = 87;
const GET_CARD_INFO: u32 = 88;
const GET_CARD_INFO_LIST: u32 = 89;
const SET_SINK_PORT: u32 = 96;

// Errors.
const ERR_ACCESS: u32 = 1;
const ERR_INVALID: u32 = 3;
const ERR_NOENTITY: u32 = 5;
const ERR_VERSION: u32 = 17;
const ERR_NOTSUPPORTED: u32 = 19;
const ERR_NOEXTENSION: u32 = 21;

/// The sample encoding that is plain samples.
const PCM: u8 = 1;

/// Hof's sound server, playing through a [`Mixer`].
#[derive(Debug)]
pub struct Sound {
    mixer: Arc<Mixer>,
    host: String,
    user: String,
}

/// Each client's index, and each stream's, on every connection.
static CLIENTS: AtomicU32 = AtomicU32::new(0);
static INPUTS: AtomicU32 = AtomicU32::new(0);

impl Sound {
    pub fn new(mixer: Arc<Mixer>) -> Self {
        Self {
            mixer,
            host: crate::x11::host_name(),
            user: std::env::var("USER").unwrap_or_default(),
        }
    }

    pub fn mixer(&self) -> &Arc<Mixer> {
        &self.mixer
    }

    /// Serves one client on `socket` until it goes.
    pub fn serve(&self, socket: UnixStream) -> io::Result<()> {
        let connection = Connection {
            mixer: self.mixer.clone(),
            out: Arc::new(Mutex::new(socket.try_clone()?)),
            streams: Arc::new(Mutex::new(BTreeMap::new())),
            version: VERSION,
            client: CLIENTS.fetch_add(1, Ordering::Relaxed),
            host: &self.host,
            user: &self.user,
        };
        let done = Arc::new(AtomicBool::new(false));
        let asking = {
            let (mixer, out, streams, done) = (
                connection.mixer.clone(),
                connection.out.clone(),
                connection.streams.clone(),
                done.clone(),
            );
            std::thread::Builder::new()
                .name("hof sound requests".into())
                .spawn(move || ask(&mixer, &out, &streams, &done))?
        };
        let mut connection = connection;
        let served = connection.run(socket);
        done.store(true, Ordering::Relaxed);
        let _ = asking.join();
        for p in std::mem::take(&mut *lock(&connection.streams)).into_values() {
            self.mixer.remove(p.input);
        }
        served
    }
}

impl Service for Sound {
    fn name(&self) -> &str {
        SERVICE
    }

    fn open(&self, opening: &[u8]) -> io::Result<(Box<dyn Socket>, Vec<u8>)> {
        let not = || io::Error::new(io::ErrorKind::InvalidData, NotOpening);
        packet_len(opening).map_err(|_| not())?.ok_or_else(not)?;
        let (ours, theirs) = UnixStream::pair()?;
        let sound = Sound {
            mixer: self.mixer.clone(),
            host: self.host.clone(),
            user: self.user.clone(),
        };
        std::thread::Builder::new()
            .name("hof sound".into())
            .spawn(move || sound.serve(theirs))?;
        Ok((Box::new(ours), opening.to_vec()))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A playback stream, as its connection knows it.
#[derive(Debug)]
struct Playback {
    /// Its index as the sink's input, and the mixer's.
    input: u32,
    spec: SampleSpec,
    attr: Attr,
    /// Bytes asked for and not yet sent.
    requested: u64,
    /// The drains to answer once it has played all it holds.
    drains: Vec<u32>,
    /// How many times it had started when last looked at.
    starts: u32,
}

/// How a stream's buffer is to be kept, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Attr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
}

impl Attr {
    /// What a client asked for, made something the server will keep: a
    /// buffer long enough to ride out a connection's jitter, in whole
    /// frames. `u32::MAX` asks for what the server likes.
    fn fit(asked: Attr, spec: SampleSpec) -> Attr {
        let frame = (mixer::sample_size(spec.format).unwrap_or(1) * usize::from(spec.channels))
            .max(1) as u64;
        let per_ms = |ms: u64| u64::from(spec.rate) * frame * ms / 1000;
        let whole = |n: u64| ((n / frame).max(1) * frame) as u32;
        let given = |v: u32| (v != u32::MAX).then_some(u64::from(v));
        let maxlength = given(asked.maxlength)
            .unwrap_or(u64::MAX)
            .min(4 * 1024 * 1024)
            .max(per_ms(500));
        let tlength = given(asked.tlength)
            .unwrap_or(per_ms(300))
            .clamp(per_ms(100), maxlength);
        let minreq = given(asked.minreq)
            .unwrap_or(tlength / 5)
            .clamp(frame, tlength / 2);
        let prebuf = given(asked.prebuf).unwrap_or(tlength / 2).min(tlength);
        Attr {
            maxlength: whole(maxlength),
            tlength: whole(tlength),
            prebuf: if prebuf == 0 { 0 } else { whole(prebuf) },
            minreq: whole(minreq),
        }
    }
}

struct Connection<'a> {
    mixer: Arc<Mixer>,
    out: Arc<Mutex<UnixStream>>,
    /// By channel.
    streams: Arc<Mutex<BTreeMap<u32, Playback>>>,
    version: u32,
    client: u32,
    host: &'a str,
    user: &'a str,
}

fn send(out: &Mutex<UnixStream>, channel: u32, payload: &[u8]) -> io::Result<()> {
    let mut packet = Vec::with_capacity(DESCRIPTOR + payload.len());
    packet.extend((payload.len() as u32).to_be_bytes());
    packet.extend(channel.to_be_bytes());
    packet.extend([0; 12]);
    packet.extend(payload);
    lock(out).write_all(&packet)
}

fn command(cmd: u32, tag: u32) -> Writer {
    let mut w = Writer::new();
    w.u32(cmd).u32(tag);
    w
}

/// Asks each stream for what its buffer is short of, says when it starts
/// playing, and answers its drains once it has played all it holds.
fn ask(
    mixer: &Mixer,
    out: &Mutex<UnixStream>,
    streams: &Mutex<BTreeMap<u32, Playback>>,
    done: &AtomicBool,
) {
    while !done.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10));
        let mut packets = Vec::new();
        for (&channel, p) in lock(streams).iter_mut() {
            let Some(status) = mixer.status(p.input) else {
                continue;
            };
            if status.starts > p.starts {
                p.starts = status.starts;
                let mut w = command(STARTED, u32::MAX);
                w.u32(channel);
                packets.push(w.into_bytes());
            }
            if !p.drains.is_empty() && status.drained {
                for tag in p.drains.drain(..) {
                    packets.push(command(REPLY, tag).into_bytes());
                }
                mixer.end_drain(p.input);
            }
            let held = status.written - status.played + p.requested;
            let missing = u64::from(p.attr.tlength).saturating_sub(held);
            if missing >= u64::from(p.attr.minreq) {
                p.requested += missing;
                let mut w = command(REQUEST, u32::MAX);
                w.u32(channel).u32(missing as u32);
                packets.push(w.into_bytes());
            }
        }
        for packet in packets {
            if send(out, CONTROL, &packet).is_err() {
                return;
            }
        }
    }
}

impl Connection<'_> {
    fn run(&mut self, mut socket: UnixStream) -> io::Result<()> {
        let mut descriptor = [0; DESCRIPTOR];
        loop {
            match socket.read_exact(&mut descriptor) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
            }
            let word = |i: usize| {
                u32::from_be_bytes(descriptor[i * 4..i * 4 + 4].try_into().expect("four"))
            };
            let len = word(0) as usize;
            if len > MOST_PACKET {
                return Err(io::Error::new(io::ErrorKind::InvalidData, Malformed));
            }
            let mut payload = vec![0; len];
            socket.read_exact(&mut payload)?;
            match word(1) {
                CONTROL => self
                    .control(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))??,
                channel => self.play(channel, &payload),
            }
        }
    }

    fn play(&self, channel: u32, data: &[u8]) {
        let mut streams = lock(&self.streams);
        let Some(p) = streams.get_mut(&channel) else {
            return;
        };
        p.requested = p.requested.saturating_sub(data.len() as u64);
        let Some(status) = self.mixer.status(p.input) else {
            return;
        };
        // More than it may hold is thrown away, as PulseAudio does.
        if status.written - status.played + data.len() as u64 > u64::from(p.attr.maxlength) {
            return;
        }
        self.mixer.write(p.input, data);
    }

    fn reply(&self, tag: u32, f: impl FnOnce(&mut Writer)) -> io::Result<()> {
        let mut w = command(REPLY, tag);
        f(&mut w);
        send(&self.out, CONTROL, &w.into_bytes())
    }

    fn ack(&self, tag: u32) -> io::Result<()> {
        self.reply(tag, |_| {})
    }

    fn error(&self, tag: u32, code: u32) -> io::Result<()> {
        let mut w = command(ERROR, tag);
        w.u32(code);
        send(&self.out, CONTROL, &w.into_bytes())
    }

    /// Does what a control packet says. A packet that is not one the
    /// command takes ends the connection, as it does with PulseAudio.
    fn control(&mut self, payload: &[u8]) -> Result<io::Result<()>, Malformed> {
        let mut r = Reader::new(payload);
        let cmd = r.u32()?;
        let tag = r.u32()?;
        Ok(match cmd {
            AUTH => {
                let version = r.u32()? & 0xffff;
                if version < OLDEST {
                    return Ok(self.error(tag, ERR_VERSION));
                }
                self.version = version.min(VERSION);
                self.reply(tag, |w| {
                    w.u32(VERSION);
                })
            }
            SET_CLIENT_NAME => self.reply(tag, |w| {
                w.u32(self.client);
            }),
            CREATE_PLAYBACK_STREAM => self.create(tag, &mut r)?,
            DELETE_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                match lock(&self.streams).remove(&channel) {
                    Some(p) => {
                        self.mixer.remove(p.input);
                        self.ack(tag)
                    }
                    None => self.error(tag, ERR_NOENTITY),
                }
            }
            DRAIN_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                match lock(&self.streams).get_mut(&channel) {
                    Some(p) => {
                        self.mixer.drain(p.input);
                        p.drains.push(tag);
                        Ok(())
                    }
                    None => self.error(tag, ERR_NOENTITY),
                }
            }
            GET_PLAYBACK_LATENCY => {
                let channel = r.u32()?;
                let asked_at = r.timeval()?;
                let input = lock(&self.streams).get(&channel).map(|p| p.input);
                match input.and_then(|i| self.mixer.status(i)) {
                    Some(status) => self.reply(tag, |w| {
                        w.usec(SINK_LATENCY)
                            .usec(0)
                            .bool(status.playing)
                            .timeval(asked_at)
                            .timeval(now())
                            .s64(status.written as i64)
                            .s64(status.played as i64);
                        if self.version >= 13 {
                            w.u64(0).u64(status.played);
                        }
                    }),
                    None => self.error(tag, ERR_NOENTITY),
                }
            }
            CORK_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                let corked = r.bool()?;
                self.on_stream(tag, channel, |m, i| m.cork(i, corked))
            }
            FLUSH_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                self.on_stream(tag, channel, Mixer::flush)
            }
            TRIGGER_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                self.on_stream(tag, channel, Mixer::trigger)
            }
            PREBUF_PLAYBACK_STREAM => {
                let channel = r.u32()?;
                self.on_stream(tag, channel, Mixer::prebuffer)
            }
            UPDATE_PLAYBACK_STREAM_SAMPLE_RATE => {
                let channel = r.u32()?;
                let rate = r.u32()?;
                if !(1..=384_000).contains(&rate) {
                    return Ok(self.error(tag, ERR_INVALID));
                }
                self.on_stream(tag, channel, |m, i| m.set_rate(i, rate))
            }
            SET_PLAYBACK_STREAM_BUFFER_ATTR => {
                let channel = r.u32()?;
                let asked = Attr {
                    maxlength: r.u32()?,
                    tlength: r.u32()?,
                    prebuf: r.u32()?,
                    minreq: r.u32()?,
                };
                let mut streams = lock(&self.streams);
                let Some(p) = streams.get_mut(&channel) else {
                    drop(streams);
                    return Ok(self.error(tag, ERR_NOENTITY));
                };
                p.attr = Attr::fit(asked, p.spec);
                self.mixer.set_prebuf(p.input, u64::from(p.attr.prebuf));
                let attr = p.attr;
                drop(streams);
                self.reply(tag, |w| {
                    w.u32(attr.maxlength)
                        .u32(attr.tlength)
                        .u32(attr.prebuf)
                        .u32(attr.minreq)
                        .usec(SINK_LATENCY);
                })
            }
            SET_SINK_INPUT_VOLUME => {
                let index = r.u32()?;
                let volume = r.cvolume()?;
                self.on_input(tag, index, |m, i| m.set_gain(i, gain(&volume)))
            }
            SET_SINK_INPUT_MUTE => {
                let index = r.u32()?;
                let muted = r.bool()?;
                self.on_input(tag, index, |m, i| m.set_muted(i, muted))
            }
            SET_SINK_VOLUME | SET_SINK_MUTE => {
                let index = r.u32()?;
                let name = r.string()?;
                if !is_sink(index, name.as_deref()) {
                    return Ok(self.error(tag, ERR_NOENTITY));
                }
                if cmd == SET_SINK_VOLUME {
                    self.mixer.set_volume(gain(&r.cvolume()?));
                } else {
                    self.mixer.set_muted_all(r.bool()?);
                }
                self.ack(tag)
            }
            GET_SERVER_INFO => self.reply(tag, |w| {
                w.string(Some("pulseaudio"))
                    .string(Some("17.0.0 (on Hof)"))
                    .string(Some(self.user))
                    .string(Some(self.host))
                    .sample_spec(self.sink_spec())
                    .string(Some(SINK))
                    .string(None)
                    .u32(0x484f_4600 | self.client & 0xff);
                if self.version >= 15 {
                    w.channel_map(&[1, 2]);
                }
            }),
            GET_SINK_INFO => {
                let index = r.u32()?;
                let name = r.string()?;
                if is_sink(index, name.as_deref()) {
                    self.reply(tag, |w| self.sink_info(w))
                } else {
                    self.error(tag, ERR_NOENTITY)
                }
            }
            GET_SINK_INFO_LIST => self.reply(tag, |w| self.sink_info(w)),
            LOOKUP_SINK => {
                let name = r.string()?;
                if is_sink(INVALID, name.as_deref()) {
                    self.reply(tag, |w| {
                        w.u32(0);
                    })
                } else {
                    self.error(tag, ERR_NOENTITY)
                }
            }
            STAT => self.reply(tag, |w| {
                for _ in 0..5 {
                    w.u32(0);
                }
            }),
            // Lists of what this server has none of.
            GET_SOURCE_INFO_LIST
            | GET_MODULE_INFO_LIST
            | GET_CLIENT_INFO_LIST
            | GET_SINK_INPUT_INFO_LIST
            | GET_SOURCE_OUTPUT_INFO_LIST
            | GET_SAMPLE_INFO_LIST
            | GET_CARD_INFO_LIST => self.ack(tag),
            CREATE_RECORD_STREAM
            | LOOKUP_SOURCE
            | GET_SOURCE_INFO
            | GET_MODULE_INFO
            | GET_CLIENT_INFO
            | GET_SINK_INPUT_INFO
            | GET_SOURCE_OUTPUT_INFO
            | GET_SAMPLE_INFO
            | GET_CARD_INFO
            | PLAY_SAMPLE => self.error(tag, ERR_NOENTITY),
            // What changes nothing here: names, properties, events, the
            // one sink being the default.
            SUBSCRIBE
            | SET_DEFAULT_SINK
            | SET_DEFAULT_SOURCE
            | SET_PLAYBACK_STREAM_NAME
            | UPDATE_RECORD_STREAM_PROPLIST
            | UPDATE_PLAYBACK_STREAM_PROPLIST
            | UPDATE_CLIENT_PROPLIST
            | REMOVE_RECORD_STREAM_PROPLIST
            | REMOVE_PLAYBACK_STREAM_PROPLIST
            | REMOVE_CLIENT_PROPLIST
            | SUSPEND_SINK
            | SET_SINK_PORT => self.ack(tag),
            EXTENSION => self.error(tag, ERR_NOEXTENSION),
            CREATE_UPLOAD_STREAM => self.error(tag, ERR_NOTSUPPORTED),
            _ => self.error(tag, ERR_ACCESS),
        })
    }

    fn on_stream(&self, tag: u32, channel: u32, f: impl FnOnce(&Mixer, u32)) -> io::Result<()> {
        let input = lock(&self.streams).get(&channel).map(|p| p.input);
        match input {
            Some(input) => {
                f(&self.mixer, input);
                self.ack(tag)
            }
            None => self.error(tag, ERR_NOENTITY),
        }
    }

    fn on_input(&self, tag: u32, index: u32, f: impl FnOnce(&Mixer, u32)) -> io::Result<()> {
        if self.mixer.status(index).is_none() {
            return self.error(tag, ERR_NOENTITY);
        }
        f(&self.mixer, index);
        self.ack(tag)
    }

    fn sink_spec(&self) -> SampleSpec {
        SampleSpec {
            format: mixer::FLOAT32LE,
            channels: 2,
            rate: self.mixer.rate(),
        }
    }

    fn sink_info(&self, w: &mut Writer) {
        let (volume, muted) = self.mixer.volume();
        let volume = (volume.cbrt() * NORM as f32) as u32;
        let description = format!("Speakers of {} (Hof)", self.host);
        w.u32(0)
            .string(Some(SINK))
            .string(Some(&description))
            .sample_spec(self.sink_spec())
            .channel_map(&[1, 2])
            .u32(INVALID)
            .cvolume(&[volume, volume])
            .bool(muted)
            .u32(INVALID)
            .string(None)
            .usec(SINK_LATENCY)
            .string(Some("hof"))
            // Its latency may be asked.
            .u32(0x2);
        if self.version >= 13 {
            w.proplist(&[
                ("device.description", &description),
                ("device.class", "sound"),
                ("device.icon_name", "audio-speakers"),
            ])
            .usec(SINK_LATENCY);
        }
        if self.version >= 15 {
            let state = if self.mixer.sounds() { 0 } else { 1 };
            w.volume(NORM).u32(state).u32(NORM + 1).u32(INVALID);
        }
        if self.version >= 16 {
            w.u32(0).string(None);
        }
        if self.version >= 21 {
            w.u8(1).format_info(PCM, &[]);
        }
    }

    fn create(&mut self, tag: u32, r: &mut Reader) -> Result<io::Result<()>, Malformed> {
        let mut spec = r.sample_spec()?;
        let mut map = r.channel_map()?;
        let _sink_index = r.u32()?;
        let _sink_name = r.string()?;
        let maxlength = r.u32()?;
        let corked = r.bool()?;
        let tlength = r.u32()?;
        let prebuf = r.u32()?;
        let minreq = r.u32()?;
        let _sync = r.u32()?;
        let volume = r.cvolume()?;
        // Remapping, remixing, fixed format, rate and channels, not
        // moving, a variable rate.
        for _ in 0..7 {
            r.bool()?;
        }
        let mut muted = r.bool()?;
        let _adjust_latency = r.bool()?;
        let _props = r.proplist()?;
        let mut volume_set = true;
        let mut muted_set = true;
        if self.version >= 14 {
            volume_set = r.bool()?;
            let _early_requests = r.bool()?;
        }
        if self.version >= 15 {
            muted_set = r.bool()?;
            let _dont_inhibit_auto_suspend = r.bool()?;
            let _fail_on_suspend = r.bool()?;
        }
        if self.version >= 17 {
            let _relative_volume = r.bool()?;
        }
        if self.version >= 18 {
            let _passthrough = r.bool()?;
        }
        let mut formats = Vec::new();
        if self.version >= 21 {
            for _ in 0..r.u8()? {
                formats.push(r.format_info()?);
            }
        }
        if !mixer::playable(spec) {
            // A client that offers formats rather than a sample spec gets
            // the first plain one, as near as it says.
            match formats.iter().find(|(encoding, _)| *encoding == PCM) {
                Some((_, props)) => {
                    spec = spec_of(props);
                    map.clear();
                }
                None => return Ok(self.error(tag, ERR_NOTSUPPORTED)),
            }
        }
        if map.len() != usize::from(spec.channels) {
            map = default_map(spec.channels);
        }
        muted &= muted_set;
        let gain = if volume_set && !volume.is_empty() {
            gain(&volume)
        } else {
            1.0
        };
        let attr = Attr::fit(
            Attr {
                maxlength,
                tlength,
                prebuf,
                minreq,
            },
            spec,
        );
        let input = INPUTS.fetch_add(1, Ordering::Relaxed);
        let mut streams = lock(&self.streams);
        let channel = (0..)
            .find(|c| !streams.contains_key(c))
            .expect("a free channel");
        self.mixer.add(
            input,
            Setup {
                spec,
                map: map.clone(),
                prebuf: u64::from(attr.prebuf),
                corked,
                gain,
                muted,
            },
        );
        streams.insert(
            channel,
            Playback {
                input,
                spec,
                attr,
                requested: u64::from(attr.tlength),
                drains: Vec::new(),
                starts: 0,
            },
        );
        drop(streams);
        let format_name = format_name(spec.format);
        let rate = spec.rate.to_string();
        let channels = spec.channels.to_string();
        Ok(self.reply(tag, |w| {
            w.u32(channel)
                .u32(input)
                .u32(attr.tlength)
                .u32(attr.maxlength)
                .u32(attr.tlength)
                .u32(attr.prebuf)
                .u32(attr.minreq)
                .sample_spec(spec)
                .channel_map(&map)
                .u32(0)
                .string(Some(SINK))
                .bool(false)
                .usec(SINK_LATENCY);
            if self.version >= 21 {
                w.format_info(
                    PCM,
                    &[
                        ("format.sample_format", &format!("\"{format_name}\"")),
                        ("format.rate", &rate),
                        ("format.channels", &channels),
                    ],
                );
            }
        }))
    }
}

/// Whether `index` or `name` is the one sink.
fn is_sink(index: u32, name: Option<&str>) -> bool {
    match name {
        Some(name) => name == SINK || name == "@DEFAULT_SINK@",
        None => index == 0 || index == INVALID,
    }
}

/// A PulseAudio volume as a gain: its average, cubed, as PulseAudio's
/// volumes are.
fn gain(volume: &[u32]) -> f32 {
    if volume.is_empty() {
        return 1.0;
    }
    let avg = volume.iter().map(|&v| f64::from(v)).sum::<f64>() / volume.len() as f64;
    (avg / f64::from(NORM)).powi(3) as f32
}

fn now() -> (u32, u32) {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (t.as_secs() as u32, t.subsec_micros())
}

fn default_map(channels: u8) -> Vec<u8> {
    match channels {
        1 => vec![0],
        n => (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect(),
    }
}

const FORMAT_NAMES: [&str; 13] = [
    "u8",
    "aLaw",
    "uLaw",
    "s16le",
    "s16be",
    "float32le",
    "float32be",
    "s32le",
    "s32be",
    "s24le",
    "s24be",
    "s24-32le",
    "s24-32be",
];

fn format_name(format: u8) -> &'static str {
    FORMAT_NAMES
        .get(usize::from(format))
        .copied()
        .unwrap_or("s16le")
}

/// The sample spec a format's properties say, as near as they can: their
/// values are JSON, a string or a number, or a list or range of them,
/// where the first is taken.
fn spec_of(props: &[(String, Vec<u8>)]) -> SampleSpec {
    let value = |key: &str| {
        props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| String::from_utf8_lossy(v).trim_end_matches('\0').to_owned())
    };
    let first = |v: String| {
        v.trim_start_matches(['[', '{', ' '])
            .split([',', ']', '}'])
            .next()
            .unwrap_or("")
            .split(':')
            .next_back()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_owned()
    };
    let format = value("format.sample_format")
        .map(first)
        .and_then(|f| FORMAT_NAMES.iter().position(|n| *n == f))
        .map_or(mixer::S16LE, |i| i as u8);
    let rate = value("format.rate")
        .map(first)
        .and_then(|r| r.parse().ok())
        .filter(|r| (1..=384_000).contains(r))
        .unwrap_or(48_000);
    let channels = value("format.channels")
        .map(first)
        .and_then(|c| c.parse().ok())
        .filter(|c| (1..=32).contains(c))
        .unwrap_or(2);
    SampleSpec {
        format,
        channels,
        rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(rate: u32) -> SampleSpec {
        SampleSpec {
            format: mixer::S16LE,
            channels: 2,
            rate,
        }
    }

    #[test]
    fn a_buffer_is_long_enough_for_a_connection_and_in_whole_frames() {
        let asked = Attr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
        };
        // 44100 frames of four bytes a second.
        let got = Attr::fit(asked, spec(44_100));
        assert_eq!(got.tlength, 52_920, "300 ms");
        assert_eq!(got.prebuf, 26_460);
        assert_eq!(got.minreq, 10_584);
        assert_eq!(got.maxlength, 4 * 1024 * 1024);

        let tiny = Attr::fit(
            Attr {
                maxlength: u32::MAX,
                tlength: 400,
                prebuf: 0,
                minreq: 3,
            },
            spec(44_100),
        );
        assert_eq!(tiny.tlength, 17_640, "100 ms at least");
        assert_eq!((tiny.prebuf, tiny.minreq), (0, 4));
    }

    #[test]
    fn the_sink_is_named_by_its_name_its_index_or_the_default() {
        assert!(is_sink(0, None));
        assert!(is_sink(INVALID, Some("@DEFAULT_SINK@")));
        assert!(is_sink(INVALID, Some("hof")));
        assert!(!is_sink(1, None));
        assert!(!is_sink(0, Some("other")));
    }

    #[test]
    fn a_volume_is_cubed_into_a_gain() {
        assert_eq!(gain(&[NORM, NORM]), 1.0);
        assert_eq!(gain(&[NORM / 2]), 0.125);
        assert_eq!(gain(&[]), 1.0);
    }

    #[test]
    fn formats_offered_as_properties_are_read_as_near_as_they_say() {
        let props = |pairs: &[(&str, &str)]| -> Vec<(String, Vec<u8>)> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), format!("{v}\0").into_bytes()))
                .collect()
        };
        assert_eq!(
            spec_of(&props(&[
                ("format.sample_format", "\"float32le\""),
                ("format.rate", "44100"),
                ("format.channels", "1"),
            ])),
            SampleSpec {
                format: mixer::FLOAT32LE,
                channels: 1,
                rate: 44_100
            }
        );
        assert_eq!(
            spec_of(&props(&[
                ("format.sample_format", "[ \"s32le\", \"s16le\" ]"),
                ("format.rate", "{ \"min\": 8000, \"max\": 96000 }"),
            ])),
            SampleSpec {
                format: mixer::S32LE,
                channels: 2,
                rate: 8_000
            }
        );
    }
}
