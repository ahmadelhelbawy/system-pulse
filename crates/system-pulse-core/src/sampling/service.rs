//! The telemetry loop: schedules tiered sampling and emits frames.
//!
//! * Emits one frame per cheap interval (default 1 s).
//! * Moderate metrics every 2nd tick; expensive metrics every 5th tick.
//! * When not visible it sleeps without sampling, keeping idle CPU ~0.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::gpu::GpuProvider;
use crate::settings::{
    DEFAULT_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS, MIN_REFRESH_INTERVAL_MS,
};
use crate::types::{SystemInfo, TelemetrySnapshot};

use super::system::SystemSampler;

/// Receives telemetry frames. The Tauri app implements this to `emit` an IPC
/// event; the probe implements it to print.
pub trait TelemetrySink: Send + Sync {
    fn emit(&self, snapshot: TelemetrySnapshot);
}

pub struct TelemetryService {
    inner: Arc<ServiceInner>,
}

struct ServiceInner {
    sampler: Mutex<SystemSampler>,
    sink: Arc<dyn TelemetrySink + Send + Sync>,
    visible: AtomicBool,
    interval_ms: AtomicU64,
}

impl TelemetryService {
    pub fn new(sink: Arc<dyn TelemetrySink + Send + Sync>, gpu: Box<dyn GpuProvider>) -> Self {
        Self {
            inner: Arc::new(ServiceInner {
                sampler: Mutex::new(SystemSampler::new(gpu)),
                sink,
                visible: AtomicBool::new(false),
                interval_ms: AtomicU64::new(DEFAULT_REFRESH_INTERVAL_MS),
            }),
        }
    }

    /// Pause/resume sampling (drives near-zero CPU while hidden).
    pub fn set_visible(&self, visible: bool) {
        self.inner.visible.store(visible, Ordering::Relaxed);
    }

    pub fn set_interval_ms(&self, ms: u64) {
        let clamped = ms.clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);
        self.inner.interval_ms.store(clamped, Ordering::Relaxed);
    }

    /// Static hardware/system info (primes the sampler if necessary).
    pub fn system_info(&self) -> SystemInfo {
        let mut sampler = self.inner.sampler.lock().unwrap();
        sampler.sample_cheap(); // primes on first use
        sampler.system_info().clone()
    }

    /// Spawn the background telemetry thread.
    pub fn spawn(&self) -> JoinHandle<()> {
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("system-pulse-telemetry".to_string())
            .spawn(move || run(inner))
            .expect("failed to spawn telemetry thread")
    }
}

fn run(inner: Arc<ServiceInner>) {
    let mut tick: u64 = 0;
    loop {
        if !inner.visible.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }

        {
            let mut sampler = inner.sampler.lock().unwrap();
            if tick.is_multiple_of(2) {
                sampler.sample_moderate();
            }
            if tick.is_multiple_of(5) {
                sampler.sample_expensive();
            }
            sampler.sample_cheap();
            let snapshot = sampler.snapshot();
            drop(sampler);
            inner.sink.emit(snapshot);
        }

        tick = tick.wrapping_add(1);
        let interval = inner
            .interval_ms
            .load(Ordering::Relaxed)
            .clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);
        std::thread::sleep(Duration::from_millis(interval));
    }
}
