//! An interruptible sleep: `wait_until` blocks a thread up to a deadline (or
//! forever, for the hidden-idle case) but returns immediately if anyone
//! calls `notify` — used so a settings change, a visibility flip, or
//! shutdown takes effect right away instead of waiting out whatever sleep
//! is already in progress (the 1.0 defect: interval/pause changes could
//! take up to `interval_ms` — up to 10s — to be noticed).

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct WakeSignal {
    mutex: Mutex<()>,
    condvar: Condvar,
}

impl WakeSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sleeps until `deadline`, or returns early if `notify` is called.
    pub fn wait_until(&self, deadline: Instant) {
        let guard = self.mutex.lock().unwrap();
        let now = Instant::now();
        if deadline <= now {
            return;
        }
        let _ = self.condvar.wait_timeout(guard, deadline - now).unwrap();
    }

    /// Sleeps indefinitely (used while hidden/paused) until `notify`.
    pub fn wait_forever(&self, poll: Duration) {
        let guard = self.mutex.lock().unwrap();
        let _ = self.condvar.wait_timeout(guard, poll).unwrap();
    }

    pub fn notify(&self) {
        let _guard = self.mutex.lock().unwrap();
        self.condvar.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn wait_until_returns_at_deadline_without_notify() {
        let w = WakeSignal::new();
        let start = Instant::now();
        w.wait_until(start + Duration::from_millis(30));
        assert!(start.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn notify_wakes_a_waiter_before_its_deadline() {
        let w = Arc::new(WakeSignal::new());
        let waiter = Arc::clone(&w);
        let start = Instant::now();
        let handle = thread::spawn(move || {
            waiter.wait_until(Instant::now() + Duration::from_secs(30));
        });
        thread::sleep(Duration::from_millis(20));
        w.notify();
        handle.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
