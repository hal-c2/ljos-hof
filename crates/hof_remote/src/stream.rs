//! Streams numbered by byte, so either end can pick one up again after a
//! dropped connection without losing a byte or seeing one twice.
//!
//! The sender keeps a [`Backlog`] of what it sent. The receiver keeps
//! [`Received`], how far it got. On reconnect the receiver says how far,
//! and the sender sends the rest, or says it no longer has it.

use std::collections::VecDeque;

/// The newest bytes of a stream, numbered from the stream's start: what the
/// other end may not have yet. The oldest go once it holds `capacity`.
#[derive(Debug, Clone)]
pub struct Backlog {
    /// The number of the first byte kept.
    start: u64,
    bytes: VecDeque<u8>,
    capacity: usize,
}

/// What a [`Backlog`] can send to an end that has the stream up to a point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// The bytes from there on: send them and carry on.
    Rest(Vec<u8>),
    /// Some of them are gone. Something else has to bring the other end up
    /// to date, a snapshot of the screen for a pane.
    Lost,
    /// The other end claims bytes never sent: it is talking about another
    /// stream, a pane of an agent that restarted.
    Ahead,
}

impl Backlog {
    pub fn new(capacity: usize) -> Self {
        Self {
            start: 0,
            bytes: VecDeque::new(),
            capacity,
        }
    }

    /// The number the next byte gets: how much was ever sent.
    pub fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }

    /// The number of the oldest byte still kept.
    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.capacity {
            // Only the tail fits; the rest is gone as soon as it came.
            let keep = &bytes[bytes.len() - self.capacity..];
            self.start = self.end() + (bytes.len() - keep.len()) as u64;
            self.bytes.clear();
            self.bytes.extend(keep);
            return;
        }
        let over = (self.bytes.len() + bytes.len()).saturating_sub(self.capacity);
        self.bytes.drain(..over);
        self.start += over as u64;
        self.bytes.extend(bytes);
    }

    /// What to send an end that has the stream up to `have`.
    pub fn resume(&self, have: u64) -> Resume {
        if have > self.end() {
            Resume::Ahead
        } else if have < self.start {
            Resume::Lost
        } else {
            let skip = (have - self.start) as usize;
            Resume::Rest(self.bytes.range(skip..).copied().collect())
        }
    }

    /// Forgets what the other end says it has, up to `upto`: it will never
    /// ask for it again.
    pub fn acknowledge(&mut self, upto: u64) {
        let upto = upto.min(self.end());
        if upto > self.start {
            self.bytes.drain(..(upto - self.start) as usize);
            self.start = upto;
        }
    }

    /// Starts the numbering again at `at`, keeping nothing: what a
    /// snapshot does, as it stands for everything before.
    pub fn restart_at(&mut self, at: u64) {
        self.start = at;
        self.bytes.clear();
    }
}

/// How far a stream has come in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Received {
    end: u64,
}

/// A chunk that starts past what came in: bytes between were lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("bytes {have} to {at} of the stream never came")]
pub struct Gap {
    pub have: u64,
    pub at: u64,
}

impl Received {
    /// Everything up to `end` came in.
    pub fn at(end: u64) -> Self {
        Self { end }
    }

    /// The number of the next byte expected: what to tell the sender on
    /// reconnect.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The new part of `bytes`, which the sender numbered from `at`: none
    /// of it if it all came before, part if some did.
    pub fn take<'a>(&mut self, at: u64, bytes: &'a [u8]) -> Result<&'a [u8], Gap> {
        if at > self.end {
            return Err(Gap { have: self.end, at });
        }
        let seen = ((self.end - at) as usize).min(bytes.len());
        let new = &bytes[seen..];
        self.end += new.len() as u64;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backlog_keeps_the_newest_bytes_numbered_from_the_start() {
        let mut b = Backlog::new(4);
        b.push(b"ab");
        b.push(b"cde");
        assert_eq!((b.start(), b.end()), (1, 5));
        assert_eq!(b.resume(3), Resume::Rest(b"de".to_vec()));
        assert_eq!(b.resume(5), Resume::Rest(Vec::new()));
        assert_eq!(b.resume(0), Resume::Lost);
        assert_eq!(b.resume(6), Resume::Ahead);
    }

    #[test]
    fn a_chunk_larger_than_the_backlog_keeps_its_tail() {
        let mut b = Backlog::new(3);
        b.push(b"x");
        b.push(b"abcdef");
        assert_eq!((b.start(), b.end()), (4, 7));
        assert_eq!(b.resume(4), Resume::Rest(b"def".to_vec()));
    }

    #[test]
    fn what_is_acknowledged_is_forgotten() {
        let mut b = Backlog::new(10);
        b.push(b"hello");
        b.acknowledge(3);
        assert_eq!(b.resume(3), Resume::Rest(b"lo".to_vec()));
        assert_eq!(b.resume(2), Resume::Lost);
        // Acknowledging more than was sent forgets only what was.
        b.acknowledge(99);
        assert_eq!((b.start(), b.end()), (5, 5));
    }

    #[test]
    fn a_receiver_takes_only_what_it_has_not_seen() {
        let mut r = Received::default();
        assert_eq!(r.take(0, b"hel").unwrap(), b"hel");
        // Sent again after a reconnect, overlapping.
        assert_eq!(r.take(1, b"ello").unwrap(), b"lo");
        assert_eq!(r.take(0, b"he").unwrap(), b"");
        assert_eq!(r.end(), 5);
        assert_eq!(r.take(7, b"!").unwrap_err(), Gap { have: 5, at: 7 });
    }
}
