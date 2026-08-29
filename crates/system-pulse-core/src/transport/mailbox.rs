//! A single-slot, latest-wins mailbox.
//!
//! Used for the hot telemetry frame and for per-topic snapshot-like data
//! (processes, disks, ...): a producer overwrites whatever is waiting to be
//! read, and `take` returns the newest value or blocks (with a timeout)
//! until one arrives. There is no way for this to grow without bound —
//! that's the whole point.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

struct Inner<T> {
    slot: Mutex<Option<T>>,
    condvar: Condvar,
    produced: AtomicU64,
    /// Incremented whenever `put` overwrites a value nobody had drained yet.
    coalesced: AtomicU64,
    /// Incremented each time `take`/`try_take` successfully returns a value.
    taken: AtomicU64,
}

/// Cheap to clone; every clone shares the same underlying slot.
pub struct Mailbox<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for Mailbox<T> {
    fn clone(&self) -> Self {
        Mailbox {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Mailbox<T> {
    pub fn new() -> Self {
        Mailbox {
            inner: Arc::new(Inner {
                slot: Mutex::new(None),
                condvar: Condvar::new(),
                produced: AtomicU64::new(0),
                coalesced: AtomicU64::new(0),
                taken: AtomicU64::new(0),
            }),
        }
    }

    /// Publishes a new value, replacing (coalescing) any value that was
    /// waiting to be drained. Never blocks.
    pub fn put(&self, value: T) {
        let mut slot = self.inner.slot.lock().unwrap();
        self.inner.produced.fetch_add(1, Ordering::Relaxed);
        if slot.is_some() {
            self.inner.coalesced.fetch_add(1, Ordering::Relaxed);
        }
        *slot = Some(value);
        self.inner.condvar.notify_all();
    }

    /// Takes the current value if one is present, without blocking.
    pub fn try_take(&self) -> Option<T> {
        let mut slot = self.inner.slot.lock().unwrap();
        let v = slot.take();
        if v.is_some() {
            self.inner.taken.fetch_add(1, Ordering::Relaxed);
        }
        v
    }

    /// Blocks (up to `timeout`) until a value is available, then takes it.
    ///
    /// Deliberately a single `wait_timeout`, not `wait_timeout_while`: the
    /// latter only wakes when its predicate actually changes, so a bare
    /// `notify()` with no accompanying `put()` (used to interrupt shutdown
    /// promptly) would be ignored and the wait would run out its full
    /// timeout regardless. A single wait returns on *any* notify — a
    /// spurious one just means the next loop iteration in the caller tries
    /// again, which every caller already does.
    pub fn take_timeout(&self, timeout: Duration) -> Option<T> {
        let mut slot = self.inner.slot.lock().unwrap();
        if slot.is_none() {
            let (guard, _) = self.inner.condvar.wait_timeout(slot, timeout).unwrap();
            slot = guard;
        }
        let v = slot.take();
        if v.is_some() {
            self.inner.taken.fetch_add(1, Ordering::Relaxed);
        }
        v
    }

    /// Wakes anyone blocked in `take_timeout` (used by shutdown so a waiting
    /// consumer thread doesn't sit out its full timeout before noticing).
    pub fn notify(&self) {
        self.inner.condvar.notify_all();
    }

    pub fn stats(&self) -> MailboxStats {
        MailboxStats {
            produced: self.inner.produced.load(Ordering::Relaxed),
            coalesced: self.inner.coalesced.load(Ordering::Relaxed),
            taken: self.inner.taken.load(Ordering::Relaxed),
        }
    }

    /// Depth is always 0 or 1 by construction — exposed for tests that want
    /// to assert the invariant directly rather than inferring it from stats.
    #[cfg(test)]
    pub(crate) fn depth(&self) -> usize {
        usize::from(self.inner.slot.lock().unwrap().is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxStats {
    pub produced: u64,
    pub coalesced: u64,
    pub taken: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn depth_never_exceeds_one() {
        let mb: Mailbox<u32> = Mailbox::new();
        assert_eq!(mb.depth(), 0);
        mb.put(1);
        assert_eq!(mb.depth(), 1);
        mb.put(2);
        assert_eq!(mb.depth(), 1); // overwritten, not queued
        assert_eq!(mb.try_take(), Some(2));
        assert_eq!(mb.depth(), 0);
    }

    #[test]
    fn a_slow_consumer_causes_coalescing_not_growth() {
        let mb: Mailbox<u32> = Mailbox::new();
        for i in 0..1000 {
            mb.put(i);
        }
        // A thousand puts with no drain in between must still leave exactly
        // one value waiting, not a queue of a thousand.
        assert_eq!(mb.depth(), 1);
        let stats = mb.stats();
        assert_eq!(stats.produced, 1000);
        assert_eq!(stats.coalesced, 999);
        assert_eq!(mb.try_take(), Some(999));
    }

    #[test]
    fn try_take_on_empty_returns_none() {
        let mb: Mailbox<u32> = Mailbox::new();
        assert_eq!(mb.try_take(), None);
    }

    #[test]
    fn take_timeout_returns_promptly_once_a_value_arrives() {
        let mb: Mailbox<u32> = Mailbox::new();
        let producer = mb.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            producer.put(42);
        });
        let start = Instant::now();
        let v = mb.take_timeout(Duration::from_secs(5));
        handle.join().unwrap();
        assert_eq!(v, Some(42));
        // Must not have waited anywhere near the 5s timeout.
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn take_timeout_gives_up_and_returns_none_when_nothing_arrives() {
        let mb: Mailbox<u32> = Mailbox::new();
        let v = mb.take_timeout(Duration::from_millis(20));
        assert_eq!(v, None);
    }

    #[test]
    fn notify_wakes_a_blocked_consumer_early() {
        let mb: Mailbox<u32> = Mailbox::new();
        let waiter = mb.clone();
        let start = Instant::now();
        let handle = thread::spawn(move || waiter.take_timeout(Duration::from_secs(30)));
        thread::sleep(Duration::from_millis(20));
        mb.notify();
        let v = handle.join().unwrap();
        assert_eq!(v, None); // woken with nothing to take, still correct
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
