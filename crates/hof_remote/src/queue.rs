//! A first-in, first-out byte queue that can always be woken on shutdown.

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

    /// Takes the next bytes if some are waiting, without blocking.
    pub(crate) fn try_read(&self) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = inner.chunks.pop_front()?;
        inner.bytes = inner.bytes.saturating_sub(bytes.len().max(64));
        self.changed.notify_all();
        Some(bytes)
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
}
