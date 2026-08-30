//! Public facade over [`crate::scheduler::Scheduler`]: the same
//! `TelemetryService`/`TelemetrySink` shape callers (the Tauri shell, the
//! headless probe) already used, now backed by the hot/warm thread split
//! instead of one thread behind one mutex.
//!
//! `TelemetrySink::try_emit` is fallible and non-blocking (previously
//! `emit`, infallible, called directly on the sampling thread) — a slow or
//! unavailable sink can no longer stall sampling: this runs on a dedicated
//! emit thread that drains the hot-frame mailbox, so a missed emit just
//! means the next (fresher) frame will coalesce over it, never a queue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::collector::Collector;
use crate::model::Sampled;
use crate::scheduler::Scheduler;
use crate::types::{
    ConnectionSnapshot, DriverSnapshot, EventLogSnapshot, InstalledSoftware, ScheduledTaskSnapshot,
    SecurityPostureSnapshot, SensorBridgeSnapshot, ServiceSnapshot, SmbiosInfo, StartupItem,
    StorageHealthSnapshot, SystemInfo, TelemetrySnapshot,
};

/// A sink cannot keep up with the current frame rate; the frame is dropped
/// (the next one will coalesce over it in the mailbox regardless).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backpressure;

/// Receives telemetry frames. The Tauri app implements this to `emit` an
/// IPC event; the probe implements it to print.
pub trait TelemetrySink: Send + Sync {
    fn try_emit(&self, snapshot: TelemetrySnapshot) -> Result<(), Backpressure>;
}

pub struct TelemetryService {
    scheduler: Arc<Scheduler>,
    emit_shutdown: Arc<AtomicBool>,
    emit_handle: Mutex<Option<JoinHandle<()>>>,
    sink: Arc<dyn TelemetrySink + Send + Sync>,
}

impl TelemetryService {
    pub fn new(sink: Arc<dyn TelemetrySink + Send + Sync>) -> Self {
        Self {
            scheduler: Arc::new(Scheduler::new()),
            emit_shutdown: Arc::new(AtomicBool::new(false)),
            emit_handle: Mutex::new(None),
            sink,
        }
    }

    /// Pause/resume sampling (drives near-zero CPU while hidden).
    pub fn set_visible(&self, visible: bool) {
        self.scheduler.set_visible(visible);
    }

    pub fn set_interval_ms(&self, ms: u64) {
        self.scheduler.set_interval_ms(ms);
    }

    /// Static hardware/system info. Independent of the live collectors —
    /// never contends with the sampling threads.
    pub fn system_info(&self) -> SystemInfo {
        Scheduler::one_shot_system_info()
    }

    /// The latest published TCP/UDP connection table — see
    /// `Scheduler::latest_connections`.
    pub fn latest_connections(&self) -> Option<Sampled<Vec<ConnectionSnapshot>>> {
        self.scheduler.latest_connections()
    }

    /// The latest published SMBIOS inventory — see
    /// `Scheduler::latest_hardware`.
    pub fn latest_hardware(&self) -> Option<Sampled<SmbiosInfo>> {
        self.scheduler.latest_hardware()
    }

    pub fn latest_services(&self) -> Option<Sampled<Vec<ServiceSnapshot>>> {
        self.scheduler.latest_services()
    }

    pub fn latest_drivers(&self) -> Option<Sampled<Vec<DriverSnapshot>>> {
        self.scheduler.latest_drivers()
    }

    pub fn latest_startup(&self) -> Option<Sampled<Vec<StartupItem>>> {
        self.scheduler.latest_startup()
    }

    pub fn latest_installed_software(&self) -> Option<Sampled<Vec<InstalledSoftware>>> {
        self.scheduler.latest_installed_software()
    }

    pub fn latest_scheduled_tasks(&self) -> Option<Sampled<Vec<ScheduledTaskSnapshot>>> {
        self.scheduler.latest_scheduled_tasks()
    }

    pub fn latest_storage_health(&self) -> Option<Sampled<Vec<StorageHealthSnapshot>>> {
        self.scheduler.latest_storage_health()
    }

    pub fn latest_sensor_bridge(&self) -> Option<Sampled<SensorBridgeSnapshot>> {
        self.scheduler.latest_sensor_bridge()
    }

    pub fn latest_event_log(&self) -> Option<Sampled<EventLogSnapshot>> {
        self.scheduler.latest_event_log()
    }

    pub fn latest_security_posture(&self) -> Option<Sampled<SecurityPostureSnapshot>> {
        self.scheduler.latest_security_posture()
    }

    /// Queries recorded history — see `Scheduler::query_history`.
    pub fn query_history(
        &self,
        range: crate::history::TimeRange,
        series: crate::history::SeriesId,
    ) -> Result<Vec<crate::history::HistoryPoint>, crate::history::HistoryError> {
        self.scheduler.query_history(range, series)
    }

    /// Spawns the hot thread, the warm-tier worker pool, and a dedicated
    /// emit thread that drains frames to the sink. `extra_collectors` are
    /// Windows-only collectors (`system-pulse-win`) the caller constructs
    /// and hands in; `history_db_path` opts into recording telemetry
    /// history — see `Scheduler::spawn`.
    pub fn spawn(
        &self,
        extra_collectors: Vec<Box<dyn Collector>>,
        history_db_path: Option<std::path::PathBuf>,
    ) {
        self.scheduler.spawn(extra_collectors, history_db_path);

        let mailbox = self.scheduler.frame_mailbox();
        let sink = Arc::clone(&self.sink);
        let shutdown = Arc::clone(&self.emit_shutdown);
        let handle = std::thread::Builder::new()
            .name("system-pulse-emit".to_string())
            .spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    if let Some(frame) = mailbox.take_timeout(Duration::from_millis(250)) {
                        // Best-effort: a slow/unavailable sink drops this
                        // frame; the next one coalesces over it regardless.
                        let _ = sink.try_emit(frame);
                    }
                }
            })
            .expect("failed to spawn telemetry emit thread");
        *self.emit_handle.lock().unwrap() = Some(handle);
    }

    /// Stops every telemetry thread (hot, both warm workers, emit) and
    /// joins them all. 1.0 had no shutdown path at all — this is what makes
    /// pause/interval changes take effect immediately instead of after up
    /// to `interval_ms`, and what a future resource-holding collector
    /// (event log, COM) will need to release cleanly.
    pub fn stop(&self) {
        self.emit_shutdown.store(true, Ordering::Relaxed);
        self.scheduler.frame_mailbox().notify();
        if let Some(h) = self.emit_handle.lock().unwrap().take() {
            let _ = h.join();
        }
        self.scheduler.stop();
    }
}
