//! The dedicated writer thread: owns the one `HistoryStore` connection,
//! batches inserts, and runs rollup/retention on a fixed cadence — all off
//! the hot sampling thread. `record()` never blocks: a bounded channel with
//! `try_send` means a writer that's fallen behind (or a stalled disk) can
//! only ever cause dropped history samples, never a stalled hot frame —
//! the same "coalesce or drop, never block the producer" rule the
//! telemetry transport (`crate::transport`) already applies to live frames.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::store::{HistoryError, HistoryStore};
use super::HistorySample;
use crate::model::UnixMillis;

/// How many samples accumulate before a batch is flushed early (in
/// addition to the time-based flush below) — bounds how much would be
/// lost if the process were killed mid-batch.
const BATCH_SIZE: usize = 10;
/// Upper bound on how long a buffered-but-unflushed sample can wait.
const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// How often rollup + retention run. Independent of the flush cadence —
/// rollup only ever processes buckets that have fully closed, so running
/// it less often than every tick costs nothing but a little latency on
/// data becoming visible in the rollup tables.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(30);
/// Backpressure capacity: at 1 sample/sec this is minutes of slack before
/// `record()` starts dropping, far more than the writer should ever
/// realistically fall behind by.
const CHANNEL_CAPACITY: usize = 256;

enum Msg {
    Sample(HistorySample),
    Stop,
}

pub struct HistoryWriter {
    sender: mpsc::SyncSender<Msg>,
    handle: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl HistoryWriter {
    /// Opens the database synchronously (so a failure — e.g. an
    /// unwritable data directory — surfaces to the caller immediately)
    /// then hands it to the writer thread.
    pub fn spawn(path: PathBuf) -> Result<Self, HistoryError> {
        let store = HistoryStore::open(&path)?;
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let handle = thread::Builder::new()
            .name("history-writer".into())
            .spawn(move || run(store, receiver))
            .expect("failed to spawn history writer thread");
        Ok(Self {
            sender,
            handle: Some(handle),
            dropped,
        })
    }

    /// Never blocks. Drops (and counts) the sample if the writer thread is
    /// behind rather than stalling the caller — see the module doc.
    pub fn record(&self, sample: HistorySample) {
        if self.sender.try_send(Msg::Sample(sample)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Samples dropped because the writer thread was behind. Exposed so
    /// the status bar can show it, the same way `frames_coalesced` makes
    /// hot-path coalescing visible rather than silently lossy.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Flushes any buffered samples and joins the writer thread. Blocking
    /// is fine here — this is the shutdown path, not the hot path.
    pub fn stop(&mut self) {
        let _ = self.sender.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for HistoryWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(mut store: HistoryStore, receiver: mpsc::Receiver<Msg>) {
    let mut buffer: Vec<HistorySample> = Vec::with_capacity(BATCH_SIZE);
    let mut last_maintenance = Instant::now();

    loop {
        let should_stop = match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(Msg::Sample(s)) => {
                buffer.push(s);
                if buffer.len() >= BATCH_SIZE {
                    flush(&mut store, &mut buffer);
                }
                false
            }
            Ok(Msg::Stop) => true,
            Err(RecvTimeoutError::Timeout) => {
                flush(&mut store, &mut buffer);
                false
            }
            Err(RecvTimeoutError::Disconnected) => true,
        };

        if should_stop {
            flush(&mut store, &mut buffer);
            break;
        }

        if last_maintenance.elapsed() >= MAINTENANCE_INTERVAL {
            let now = UnixMillis::now();
            if let Err(e) = store.rollup(now) {
                eprintln!("history: rollup failed: {e}");
            }
            if let Err(e) = store.apply_retention(now) {
                eprintln!("history: retention failed: {e}");
            }
            last_maintenance = Instant::now();
        }
    }
}

fn flush(store: &mut HistoryStore, buffer: &mut Vec<HistorySample>) {
    if buffer.is_empty() {
        return;
    }
    if let Err(e) = store.insert_raw_batch(buffer) {
        // A failed batch is dropped rather than retried indefinitely — an
        // append-only history store is diagnostic evidence, not a
        // source of truth the app depends on for correctness elsewhere.
        eprintln!("history: failed to write {} sample(s): {e}", buffer.len());
    }
    buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts_ms: i64) -> HistorySample {
        HistorySample {
            ts_ms: UnixMillis(ts_ms),
            cpu_percent: Some(1.0),
            mem_used_percent: Some(2.0),
            gpu_percent: None,
            disk_read_rate: Some(0.0),
            disk_write_rate: Some(0.0),
            net_download_rate: Some(0.0),
            net_upload_rate: Some(0.0),
        }
    }

    #[test]
    fn recorded_samples_are_persisted_by_the_time_stop_returns() {
        let dir = std::env::temp_dir().join(format!(
            "system-pulse-history-writer-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.sqlite3");

        let mut writer = HistoryWriter::spawn(path.clone()).unwrap();
        for i in 0..5 {
            writer.record(sample(i * 1000));
        }
        writer.stop();

        let store = HistoryStore::open(&path).unwrap();
        assert_eq!(store.row_count("samples_raw"), 5);
        assert_eq!(writer.dropped_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_never_blocks_the_caller_even_under_sustained_load() {
        // The plan's own acceptance test shape ("writer never blocks the
        // hot loop... under sustained write load"): hammer `record()` far
        // faster than the writer thread could possibly flush and commit,
        // and assert the *caller* (this test, standing in for the hot
        // loop) never stalls waiting for it — `try_send` either enqueues
        // or drops, but never blocks.
        let dir = std::env::temp_dir().join(format!(
            "system-pulse-history-writer-load-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.sqlite3");

        let mut writer = HistoryWriter::spawn(path).unwrap();
        let start = Instant::now();
        for i in 0..20_000 {
            writer.record(sample(i));
        }
        let elapsed = start.elapsed();
        writer.stop();

        // Generous bound: 20,000 non-blocking channel sends completing in
        // under a second catches a real regression (an accidentally
        // blocking `send`) without being flaky under CI/sandbox jitter.
        assert!(
            elapsed < Duration::from_secs(1),
            "record() took {elapsed:?} for 20,000 calls — it must never block"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
