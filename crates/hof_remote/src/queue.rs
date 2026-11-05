//! A first-in, first-out byte queue that can always be woken on shutdown,
//! and [`Lanes`]: two of them, one taken before the other.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

#[derive(Default)]
struct Inner {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}

#[derive(Default)]
pub(crate) struct Queue {
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl Queue {
    /// Adds bytes, up to `limit`. Each chunk costs at least 64 bytes, so
    /// empty and tiny frames cannot bypass the bound. Waits for room when
    /// `wait` is true, but never past `close`. One frame larger than the
    /// limit may pass through an otherwise empty queue.
    pub(crate) fn push(&self, bytes: Vec<u8>, limit: usize, wait: bool) -> bool {
        let cost = bytes.len().max(64);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while wait && inner.bytes > 0 && inner.bytes + cost > limit && !inner.closed {
            inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        if inner.closed || (inner.bytes > 0 && inner.bytes + cost > limit) {
            return false;
        }
        inner.bytes += cost;
        inner.chunks.push_back(bytes);
        self.changed.notify_one();
        true
    }

    /// Takes the next bytes, waiting for some until the queue is closed and
    /// drained. Closing never discards what was already queued.
    pub(crate) fn read(&self) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(bytes) = inner.chunks.pop_front() {
                inner.bytes = inner.bytes.saturating_sub(bytes.len().max(64));
                self.changed.notify_all();
                return Some(bytes);
            }
            if inner.closed {
                return None;
            }
            inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub(crate) fn close(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.closed = true;
        self.changed.notify_all();
    }
}

/// Which of [`Lanes`] a frame goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    /// What the panes need: keystrokes, resizes, pings, acknowledgements.
    Urgent,
    /// What the relays carry: X11, sound, files.
    Bulk,
}

#[derive(Default)]
struct LanesInner {
    urgent: VecDeque<Vec<u8>>,
    urgent_bytes: usize,
    bulk: VecDeque<Vec<u8>>,
    bulk_bytes: usize,
    closed: bool,
}

/// A connection's frames on their way out, in two lanes, each in order and
/// each with its own room. What is urgent goes first, and bulk a piece at a
/// time between: a keystroke waits for one piece of a file or an X11 image,
/// not for all that was queued before it.
#[derive(Default)]
pub(crate) struct Lanes {
    inner: Mutex<LanesInner>,
    changed: Condvar,
}

impl Lanes {
    /// As [`Queue::push`], in `lane`, with `limit` that lane's room.
    pub(crate) fn push(&self, lane: Lane, bytes: Vec<u8>, limit: usize, wait: bool) -> bool {
        let cost = bytes.len().max(64);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let used = |inner: &LanesInner| match lane {
            Lane::Urgent => inner.urgent_bytes,
            Lane::Bulk => inner.bulk_bytes,
        };
        while wait && used(&inner) > 0 && used(&inner) + cost > limit && !inner.closed {
            inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        if inner.closed || (used(&inner) > 0 && used(&inner) + cost > limit) {
            return false;
        }
        match lane {
            Lane::Urgent => {
                inner.urgent_bytes += cost;
                inner.urgent.push_back(bytes);
            }
            Lane::Bulk => {
                inner.bulk_bytes += cost;
                inner.bulk.push_back(bytes);
            }
        }
        self.changed.notify_all();
        true
    }

    /// What goes out next, in one piece: all that is urgent, then bulk up
    /// to `bulk_most` bytes (one frame at least, if there is one). Waits
    /// for some; `None` once closed and drained.
    pub(crate) fn take(&self, bulk_most: usize) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while inner.urgent.is_empty() && inner.bulk.is_empty() {
            if inner.closed {
                return None;
            }
            inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        let mut out = Vec::new();
        while let Some(bytes) = inner.urgent.pop_front() {
            inner.urgent_bytes = inner.urgent_bytes.saturating_sub(bytes.len().max(64));
            out.extend(bytes);
        }
        let mut bulk = 0;
        while bulk < bulk_most.max(1)
            && let Some(bytes) = inner.bulk.pop_front()
        {
            inner.bulk_bytes = inner.bulk_bytes.saturating_sub(bytes.len().max(64));
            bulk += bytes.len().max(1);
            out.extend(bytes);
        }
        self.changed.notify_all();
        Some(out)
    }

    pub(crate) fn close(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.closed = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn many_small_chunks_count_as_bytes_not_messages() {
        let queue = Queue::default();
        for _ in 0..100 {
            assert!(queue.push(vec![0; 4], 6400, false));
        }
        assert!(!queue.push(vec![0], 6400, false));
        for _ in 0..100 {
            assert_eq!(queue.read().unwrap().len(), 4);
        }
    }

    #[test]
    fn empty_chunks_still_have_a_cost() {
        let queue = Queue::default();
        assert!(queue.push(Vec::new(), 64, false));
        assert!(!queue.push(Vec::new(), 64, false));
        assert_eq!(queue.read().unwrap().len(), 0);
        assert!(queue.push(Vec::new(), 64, false));
    }

    #[test]
    fn closing_wakes_a_sender_waiting_for_room() {
        let queue = Arc::new(Queue::default());
        queue.push(vec![0; 4], 4, false);
        let waiting = {
            let queue = queue.clone();
            std::thread::spawn(move || queue.push(vec![0], 4, true))
        };
        std::thread::sleep(Duration::from_millis(20));
        assert!(!waiting.is_finished());
        queue.close();
        assert!(!waiting.join().unwrap());
        assert_eq!(queue.read().unwrap().len(), 4);
        assert!(queue.read().is_none());
    }

    #[test]
    fn what_is_urgent_goes_before_bulk_queued_ahead_of_it() {
        let lanes = Lanes::default();
        for n in 0..4u8 {
            assert!(lanes.push(Lane::Bulk, vec![n; 100], 1000, false));
        }
        assert!(lanes.push(Lane::Urgent, b"key".to_vec(), 1000, false));
        // The keystroke, and then one piece of bulk only.
        let mut first = b"key".to_vec();
        first.extend([0; 100]);
        assert_eq!(lanes.take(1).unwrap(), first);
        assert!(lanes.push(Lane::Urgent, b"again".to_vec(), 1000, false));
        let mut second = b"again".to_vec();
        second.extend([1; 100]);
        assert_eq!(lanes.take(1).unwrap(), second);
        assert_eq!(lanes.take(1000).unwrap(), [[2; 100], [3; 100]].concat());
    }

    #[test]
    fn a_full_bulk_lane_leaves_the_urgent_one_its_room() {
        let lanes = Lanes::default();
        assert!(lanes.push(Lane::Bulk, vec![0; 1000], 1000, false));
        assert!(!lanes.push(Lane::Bulk, vec![0; 10], 1000, false));
        assert!(lanes.push(Lane::Urgent, vec![0; 1000], 1000, false));
        lanes.close();
        assert!(lanes.take(1).is_some());
        assert!(lanes.take(1).is_none());
    }
}
