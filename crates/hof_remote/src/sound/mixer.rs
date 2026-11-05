//! What Hof's sound server plays: each client's stream, held as stereo
//! frames at the stream's own rate, and mixed into what the speakers ask
//! for at theirs.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::tagstruct::SampleSpec;

pub const U8: u8 = 0;
pub const ALAW: u8 = 1;
pub const ULAW: u8 = 2;
pub const S16LE: u8 = 3;
pub const S16BE: u8 = 4;
pub const FLOAT32LE: u8 = 5;
pub const FLOAT32BE: u8 = 6;
pub const S32LE: u8 = 7;
pub const S32BE: u8 = 8;
pub const S24LE: u8 = 9;
pub const S24BE: u8 = 10;
pub const S24_32LE: u8 = 11;
pub const S24_32BE: u8 = 12;

/// How many bytes a sample of `format` takes; `None` for a format that is
/// not one.
pub fn sample_size(format: u8) -> Option<usize> {
    match format {
        U8 | ALAW | ULAW => Some(1),
        S16LE | S16BE => Some(2),
        S24LE | S24BE => Some(3),
        FLOAT32LE | FLOAT32BE | S32LE | S32BE | S24_32LE | S24_32BE => Some(4),
        _ => None,
    }
}

/// Whether a stream may be played as `spec` says.
pub fn playable(spec: SampleSpec) -> bool {
    sample_size(spec.format).is_some()
        && (1..=32).contains(&spec.channels)
        && (1..=384_000).contains(&spec.rate)
}

/// What a stream starts as.
#[derive(Clone, Debug)]
pub struct Setup {
    pub spec: SampleSpec,
    /// Where each channel is meant to be heard: front left, rear right…
    pub map: Vec<u8>,
    /// How many bytes to hold before playing, so a late packet is not a
    /// gap.
    pub prebuf: u64,
    pub corked: bool,
    pub gain: f32,
    pub muted: bool,
}

/// Where a stream has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// Bytes it has been given.
    pub written: u64,
    /// Bytes of those played, or thrown away.
    pub played: u64,
    pub playing: bool,
    /// It was told to drain, and has played everything.
    pub drained: bool,
    /// How many times it has started playing.
    pub starts: u32,
}

#[derive(Debug)]
struct Stream {
    spec: SampleSpec,
    frame: usize,
    /// How much of each channel goes left and right.
    weights: Vec<(f32, f32)>,
    frames: VecDeque<[f32; 2]>,
    /// Bytes of a frame not all here yet.
    partial: Vec<u8>,
    /// How far into the first frame the next one to play is.
    pos: f64,
    gain: f32,
    muted: bool,
    corked: bool,
    prebuf: u64,
    prebuffering: bool,
    draining: bool,
    written: u64,
    played: u64,
    starts: u32,
}

impl Stream {
    fn queued(&self) -> u64 {
        self.written - self.played
    }

    fn start_when_full(&mut self) {
        if self.prebuffering && self.queued() >= self.prebuf.max(1) {
            self.prebuffering = false;
            self.starts += 1;
        }
    }

    fn sounds(&self) -> bool {
        !self.corked && !self.frames.is_empty()
    }
}

#[derive(Debug)]
struct Inner {
    streams: HashMap<u32, Stream>,
    volume: f32,
    muted: bool,
    rate: u32,
    /// When a stream was last given something, or played.
    last_sound: Option<Instant>,
}

/// The streams being played, for the speakers to take from.
#[derive(Debug)]
pub struct Mixer {
    inner: Mutex<Inner>,
    sounding: Condvar,
}

impl Mixer {
    /// A mixer whose speakers play `rate` frames a second, until they say
    /// otherwise.
    pub fn new(rate: u32) -> Self {
        Self {
            inner: Mutex::new(Inner {
                streams: HashMap::new(),
                volume: 1.0,
                muted: false,
                rate,
                last_sound: None,
            }),
            sounding: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The rate the speakers play at.
    pub fn rate(&self) -> u32 {
        self.lock().rate
    }

    pub fn add(&self, id: u32, setup: Setup) {
        let spec = setup.spec;
        let frame = sample_size(spec.format).unwrap_or(1) * usize::from(spec.channels);
        let stream = Stream {
            spec,
            frame,
            weights: weights(spec.channels, &setup.map),
            frames: VecDeque::new(),
            partial: Vec::new(),
            pos: 0.0,
            gain: setup.gain,
            muted: setup.muted,
            corked: setup.corked,
            prebuf: setup.prebuf,
            prebuffering: true,
            draining: false,
            written: 0,
            played: 0,
            starts: 0,
        };
        self.lock().streams.insert(id, stream);
    }

    pub fn remove(&self, id: u32) {
        self.lock().streams.remove(&id);
    }

    fn with<T>(&self, id: u32, f: impl FnOnce(&mut Stream) -> T) -> Option<T> {
        let mut inner = self.lock();
        let t = inner.streams.get_mut(&id).map(f);
        let sounds = inner.streams.values().any(Stream::sounds);
        if sounds {
            inner.last_sound = Some(Instant::now());
            drop(inner);
            self.sounding.notify_all();
        }
        t
    }

    /// Gives a stream more to play.
    pub fn write(&self, id: u32, bytes: &[u8]) {
        self.with(id, |s| {
            s.written += bytes.len() as u64;
            let mut data = std::mem::take(&mut s.partial);
            data.extend_from_slice(bytes);
            let size = sample_size(s.spec.format).unwrap_or(1);
            let mut frames = data.chunks_exact(s.frame);
            for frame in &mut frames {
                let mut lr = [0.0; 2];
                for (sample, (l, r)) in frame.chunks_exact(size).zip(&s.weights) {
                    let v = decode(s.spec.format, sample);
                    lr[0] += v * l;
                    lr[1] += v * r;
                }
                s.frames.push_back(lr);
            }
            s.partial = frames.remainder().to_vec();
            s.start_when_full();
        });
    }

    pub fn cork(&self, id: u32, corked: bool) {
        self.with(id, |s| s.corked = corked);
    }

    /// Throws away what a stream has not played, and holds it until it
    /// has enough again.
    pub fn flush(&self, id: u32) {
        self.with(id, |s| {
            s.frames.clear();
            s.partial.clear();
            s.pos = 0.0;
            s.played = s.written;
            s.prebuffering = s.prebuf > 0;
        });
    }

    /// Plays a stream now, however little it holds.
    pub fn trigger(&self, id: u32) {
        self.with(id, |s| {
            if s.prebuffering {
                s.prebuffering = false;
                s.starts += 1;
            }
        });
    }

    /// Holds a stream until it has enough again.
    pub fn prebuffer(&self, id: u32) {
        self.with(id, |s| s.prebuffering = s.prebuf > 0);
    }

    /// Plays all a stream holds; [`Status::drained`] once it has.
    pub fn drain(&self, id: u32) {
        self.with(id, |s| {
            s.draining = true;
            if s.prebuffering {
                s.prebuffering = false;
                s.starts += 1;
            }
        });
    }

    /// The drain is answered: the stream holds until it has enough again.
    pub fn end_drain(&self, id: u32) {
        self.with(id, |s| {
            s.draining = false;
            s.prebuffering = s.prebuf > 0;
        });
    }

    pub fn set_prebuf(&self, id: u32, prebuf: u64) {
        self.with(id, |s| s.prebuf = prebuf);
    }

    pub fn set_gain(&self, id: u32, gain: f32) {
        self.with(id, |s| s.gain = gain);
    }

    pub fn set_muted(&self, id: u32, muted: bool) {
        self.with(id, |s| s.muted = muted);
    }

    pub fn set_rate(&self, id: u32, rate: u32) {
        self.with(id, |s| s.spec.rate = rate);
    }

    pub fn set_volume(&self, volume: f32) {
        self.lock().volume = volume;
    }

    pub fn set_muted_all(&self, muted: bool) {
        self.lock().muted = muted;
    }

    /// The speakers' volume, and whether they are muted.
    pub fn volume(&self) -> (f32, bool) {
        let inner = self.lock();
        (inner.volume, inner.muted)
    }

    pub fn status(&self, id: u32) -> Option<Status> {
        let inner = self.lock();
        inner.streams.get(&id).map(|s| Status {
            written: s.written,
            played: s.played,
            playing: !s.corked && !s.prebuffering && !s.frames.is_empty(),
            drained: s.draining && s.frames.is_empty(),
            starts: s.starts,
        })
    }

    /// Whether a stream has something to play.
    pub fn sounds(&self) -> bool {
        self.lock().streams.values().any(Stream::sounds)
    }

    /// Waits until a stream has something to play.
    pub fn wait_for_sound(&self) {
        let mut inner = self.lock();
        while !inner.streams.values().any(Stream::sounds) {
            inner = self
                .sounding
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
        inner.last_sound = Some(Instant::now());
    }

    /// How long since a stream was last given something or played.
    pub fn quiet_for(&self) -> Duration {
        self.lock()
            .last_sound
            .map_or(Duration::MAX, |at| at.elapsed())
    }

    /// Mixes the streams into `out`: frames of `channels` samples, `rate`
    /// a second.
    pub fn render(&self, out: &mut [f32], channels: usize, rate: u32) {
        out.fill(0.0);
        let channels = channels.max(1);
        let mut inner = self.lock();
        inner.rate = rate;
        let all = if inner.muted { 0.0 } else { inner.volume };
        let mut sounded = false;
        for s in inner.streams.values_mut() {
            if s.corked || s.prebuffering || s.frames.is_empty() {
                continue;
            }
            sounded = true;
            let gain = if s.muted { 0.0 } else { s.gain * all };
            let step = f64::from(s.spec.rate) / f64::from(rate.max(1));
            let mut ran_out = false;
            for frame in out.chunks_exact_mut(channels) {
                let at = s.pos as usize;
                let Some(a) = s.frames.get(at) else {
                    ran_out = true;
                    break;
                };
                let b = match s.frames.get(at + 1) {
                    Some(b) => b,
                    // The last frame waits for the next to come, unless
                    // there is no next.
                    None if s.draining => a,
                    None => {
                        ran_out = true;
                        break;
                    }
                };
                let t = (s.pos - at as f64) as f32;
                let l = (a[0] + (b[0] - a[0]) * t) * gain;
                let r = (a[1] + (b[1] - a[1]) * t) * gain;
                if channels == 1 {
                    frame[0] += (l + r) * 0.5;
                } else {
                    frame[0] += l;
                    frame[1] += r;
                }
                s.pos += step;
            }
            let done = (s.pos as usize).min(s.frames.len());
            s.frames.drain(..done);
            s.pos -= done as f64;
            s.played += (done * s.frame) as u64;
            if s.frames.is_empty() && s.draining {
                s.pos = 0.0;
            }
            // A stream that ran dry holds again until it has enough, so it
            // plays on smoothly rather than a packet at a time.
            if ran_out && !s.draining && s.prebuf > 0 {
                s.prebuffering = true;
            }
        }
        if sounded {
            inner.last_sound = Some(Instant::now());
        }
        for v in out.iter_mut() {
            *v = v.clamp(-1.0, 1.0);
        }
    }
}

/// How much of each of `channels`, placed as `map` says, goes left and
/// right.
fn weights(channels: u8, map: &[u8]) -> Vec<(f32, f32)> {
    const MONO: u8 = 0;
    const LEFT: [u8; 6] = [1, 5, 8, 10, 45, 48];
    const RIGHT: [u8; 6] = [2, 6, 9, 11, 46, 49];
    let n = usize::from(channels);
    if n == 1 {
        return vec![(1.0, 1.0)];
    }
    let map: Vec<u8> = if map.len() == n {
        map.to_vec()
    } else {
        // Left and right in turn.
        (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect()
    };
    map.iter()
        .map(|p| match *p {
            MONO => (1.0, 1.0),
            p if LEFT.contains(&p) => (1.0, 0.0),
            p if RIGHT.contains(&p) => (0.0, 1.0),
            _ => (
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ),
        })
        .collect()
}

/// A sample of `format` as a number from -1 to 1.
fn decode(format: u8, b: &[u8]) -> f32 {
    let s24 = |v: i32| ((v << 8) >> 8) as f32 / 8_388_608.0;
    match format {
        U8 => (f32::from(b[0]) - 128.0) / 128.0,
        ALAW => f32::from(alaw(b[0])) / 32_768.0,
        ULAW => f32::from(ulaw(b[0])) / 32_768.0,
        S16LE => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
        S16BE => f32::from(i16::from_be_bytes([b[0], b[1]])) / 32_768.0,
        FLOAT32LE => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        FLOAT32BE => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        S32LE => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        S32BE => i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        S24LE => s24(i32::from_le_bytes([b[0], b[1], b[2], 0])),
        S24BE => s24(i32::from_be_bytes([0, b[0], b[1], b[2]])),
        S24_32LE => s24(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        S24_32BE => s24(i32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        _ => 0.0,
    }
    .clamp(-1.0, 1.0)
}

/// G.711 A-law, as a 16-bit sample.
fn alaw(a: u8) -> i16 {
    let a = a ^ 0x55;
    let mantissa = i16::from(a & 0x0f) << 4;
    let segment = (a & 0x70) >> 4;
    let t = match segment {
        0 => mantissa + 8,
        1 => mantissa + 0x108,
        _ => (mantissa + 0x108) << (segment - 1),
    };
    if a & 0x80 != 0 { t } else { -t }
}

/// G.711 µ-law, as a 16-bit sample.
fn ulaw(u: u8) -> i16 {
    let u = !u;
    let t = ((i16::from(u & 0x0f) << 3) + 0x84) << ((u & 0x70) >> 4);
    if u & 0x80 != 0 { 0x84 - t } else { t - 0x84 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(format: u8, channels: u8, rate: u32, prebuf: u64) -> Setup {
        Setup {
            spec: SampleSpec {
                format,
                channels,
                rate,
            },
            map: Vec::new(),
            prebuf,
            corked: false,
            gain: 1.0,
            muted: false,
        }
    }

    fn s16(frames: &[(i16, i16)]) -> Vec<u8> {
        frames
            .iter()
            .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
            .flatten()
            .collect()
    }

    #[test]
    fn samples_of_each_format_are_numbers_from_minus_one_to_one() {
        assert_eq!(decode(U8, &[128]), 0.0);
        assert_eq!(decode(S16LE, &(-16_384i16).to_le_bytes()), -0.5);
        assert_eq!(decode(S16BE, &16_384i16.to_be_bytes()), 0.5);
        assert_eq!(decode(FLOAT32BE, &0.25f32.to_be_bytes()), 0.25);
        assert_eq!(decode(S32LE, &(i32::MIN / 2).to_le_bytes()), -0.5);
        assert_eq!(decode(S24LE, &[0, 0, 0x40]), 0.5);
        assert_eq!(decode(S24BE, &[0xc0, 0, 0]), -0.5);
        assert_eq!(
            decode(S24_32LE, &[0, 0, 0xc0, 0x7f]),
            -0.5,
            "top byte unused"
        );
        assert!(decode(ULAW, &[0xff]).abs() < 0.001, "µ-law's zero");
        assert!(decode(ALAW, &[0xd5]).abs() < 0.001, "A-law's zero");
        assert!(decode(ULAW, &[0x00]) < -0.9 && decode(ALAW, &[0xaa]) > 0.9);
    }

    #[test]
    fn a_stream_plays_once_it_holds_enough_and_counts_what_it_played() {
        let mixer = Mixer::new(48_000);
        mixer.add(1, setup(S16LE, 2, 48_000, 16));
        let mut out = [0.0; 8];
        mixer.write(1, &s16(&[(16_384, -16_384); 3]));
        mixer.render(&mut out, 2, 48_000);
        assert_eq!(out, [0.0; 8], "12 bytes is less than 16");
        assert_eq!(mixer.status(1).unwrap().starts, 0);

        // Two bytes of a frame come first, then the rest.
        mixer.write(1, &s16(&[(16_384, -16_384)])[..2]);
        mixer.write(1, &s16(&[(16_384, -16_384)])[2..]);
        mixer.render(&mut out, 2, 48_000);
        // The last frame waits for the next, to go between them.
        assert_eq!(out, [0.5, -0.5, 0.5, -0.5, 0.5, -0.5, 0.0, 0.0]);
        let status = mixer.status(1).unwrap();
        assert_eq!((status.written, status.played, status.starts), (16, 12, 1));
    }

    #[test]
    fn a_drained_stream_plays_to_its_last_frame() {
        let mixer = Mixer::new(48_000);
        mixer.add(1, setup(S16LE, 2, 48_000, 1_000));
        mixer.write(1, &s16(&[(16_384, 16_384); 2]));
        mixer.drain(1);
        assert!(!mixer.status(1).unwrap().drained);
        let mut out = [0.0; 6];
        mixer.render(&mut out, 2, 48_000);
        assert_eq!(out, [0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        let status = mixer.status(1).unwrap();
        assert!(status.drained);
        assert_eq!(status.played, 8);
    }

    #[test]
    fn a_stream_at_half_the_rate_is_played_twice_as_long_between_its_frames() {
        let mixer = Mixer::new(48_000);
        mixer.add(1, setup(S16LE, 1, 24_000, 0));
        let mono: Vec<u8> = [0i16, 16_384, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        mixer.write(1, &mono);
        let mut out = [0.0; 4];
        mixer.render(&mut out, 1, 48_000);
        assert_eq!(out, [0.0, 0.25, 0.5, 0.25]);
    }

    #[test]
    fn corked_flushed_and_muted_streams_are_silent() {
        let mixer = Mixer::new(48_000);
        let mut corked = setup(S16LE, 2, 48_000, 0);
        corked.corked = true;
        mixer.add(1, corked);
        mixer.write(1, &s16(&[(16_384, 16_384); 4]));
        let mut out = [0.0; 4];
        mixer.render(&mut out, 2, 48_000);
        assert_eq!(out, [0.0; 4]);
        mixer.flush(1);
        assert_eq!(mixer.status(1).unwrap().played, 16);

        mixer.add(2, setup(S16LE, 2, 48_000, 0));
        mixer.set_muted(2, true);
        mixer.write(2, &s16(&[(16_384, 16_384); 4]));
        mixer.render(&mut out, 2, 48_000);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(mixer.status(2).unwrap().played, 8, "played, unheard");
    }

    #[test]
    fn channels_are_placed_left_right_or_both() {
        assert_eq!(weights(1, &[]), [(1.0, 1.0)]);
        assert_eq!(weights(2, &[]), [(1.0, 0.0), (0.0, 1.0)]);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            weights(4, &[1, 2, 3, 6]),
            [(1.0, 0.0), (0.0, 1.0), (h, h), (0.0, 1.0)]
        );
    }
}
