//! Wires collectors onto two thread groups — one hot, one warm/cold worker
//! pool — with wall-clock scheduling, immediate-effect pause/interval
//! changes, and a bounded, joinable shutdown path (all absent in 1.0: the
//! telemetry thread's `JoinHandle` was discarded and the loop had no stop
//! flag at all).

mod hot;
mod shared;
mod wake;
mod worker;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use sysinfo::System;

use crate::collector::{
    Collector, CpuCollector, DiskCollector, GpuCollector, MemoryCollector, NetworkCollector,
    ProcessCollector,
};
use crate::settings::DEFAULT_REFRESH_INTERVAL_MS;
use crate::transport::Mailbox;
use crate::types::TelemetrySnapshot;

use hot::HotLoop;
use shared::{new_shared_sections, SharedSections};
use wake::WakeSignal;
use worker::WorkerLoop;

/// Owns the two thread groups and their shared control flags. `spawn` and
/// `stop` are the only lifecycle calls; everything else (`set_visible`,
/// `set_interval_ms`) takes effect immediately via [`WakeSignal`] rather
/// than waiting for whatever sleep is already in progress.
pub struct Scheduler {
    shared_sys: Arc<parking_lot::Mutex<System>>,
    sections: SharedSections,
    shutdown: Arc<AtomicBool>,
    visible: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    hot_resume: Arc<AtomicBool>,
    warm_resume: Arc<AtomicBool>,
    hot_wake: Arc<WakeSignal>,
    warm_wake: Arc<WakeSignal>,
    frame_mailbox: Mailbox<TelemetrySnapshot>,
    handles: std::sync::Mutex<Vec<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            shared_sys: Arc::new(parking_lot::Mutex::new(System::new())),
            sections: new_shared_sections(),
            shutdown: Arc::new(AtomicBool::new(false)),
            visible: Arc::new(AtomicBool::new(false)),
            interval_ms: Arc::new(AtomicU64::new(DEFAULT_REFRESH_INTERVAL_MS)),
            hot_resume: Arc::new(AtomicBool::new(false)),
            warm_resume: Arc::new(AtomicBool::new(false)),
            hot_wake: Arc::new(WakeSignal::new()),
            warm_wake: Arc::new(WakeSignal::new()),
            frame_mailbox: Mailbox::new(),
            handles: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn frame_mailbox(&self) -> Mailbox<TelemetrySnapshot> {
        self.frame_mailbox.clone()
    }

    pub fn set_visible(&self, visible: bool) {
        let was_visible = self.visible.swap(visible, Ordering::AcqRel);
        if visible && !was_visible {
            self.hot_resume.store(true, Ordering::Release);
            self.warm_resume.store(true, Ordering::Release);
        }
        self.hot_wake.notify();
        self.warm_wake.notify();
    }

    pub fn set_interval_ms(&self, ms: u64) {
        self.interval_ms.store(ms, Ordering::Relaxed);
        self.hot_wake.notify();
    }

    /// Builds the built-in collectors and spawns the hot thread plus a
    /// two-worker pool for the warm-tier ones. `extra_warm_collectors` are
    /// merged into the worker pool alongside the built-ins — this is how
    /// Windows-only collectors (`system-pulse-win`, which depends on this
    /// crate and so cannot be constructed from inside it) get scheduled:
    /// the caller (the Tauri shell) constructs them and hands them in here.
    /// Each collector is `probe()`d before its thread starts running its
    /// schedule.
    pub fn spawn(&self, extra_warm_collectors: Vec<Box<dyn Collector>>) {
        let cpu = CpuCollector::new(Arc::clone(&self.shared_sys));
        let memory = MemoryCollector::new(Arc::clone(&self.shared_sys));
        let process = ProcessCollector::new(Arc::clone(&self.shared_sys));
        let disk = DiskCollector::new();
        let network = NetworkCollector::new();
        let gpu = GpuCollector::new();

        let hot = HotLoop::new(
            cpu,
            memory,
            Arc::clone(&self.sections),
            self.frame_mailbox.clone(),
            Arc::clone(&self.shutdown),
            Arc::clone(&self.visible),
            Arc::clone(&self.hot_resume),
            Arc::clone(&self.interval_ms),
            Arc::clone(&self.hot_wake),
        );

        // Two workers, statically split: worker A gets the two collectors
        // whose cadence is more often relied on for interactivity
        // (processes, disk), worker B gets network + the more expensive
        // GPU poll. A slow collector only ever delays collectors sharing
        // its own worker, never the hot thread. Extra collectors are
        // distributed evenly across both by index.
        let mut worker_a: Vec<Box<dyn Collector>> = vec![Box::new(process), Box::new(disk)];
        let mut worker_b: Vec<Box<dyn Collector>> = vec![Box::new(network), Box::new(gpu)];
        for (i, c) in extra_warm_collectors.into_iter().enumerate() {
            if i % 2 == 0 {
                worker_a.push(c);
            } else {
                worker_b.push(c);
            }
        }

        let mut handles = self.handles.lock().unwrap();

        handles.push(
            std::thread::Builder::new()
                .name("system-pulse-hot".to_string())
                .spawn(move || hot.run())
                .expect("failed to spawn hot telemetry thread"),
        );

        for (name, collectors) in [
            ("system-pulse-warm-a", worker_a),
            ("system-pulse-warm-b", worker_b),
        ] {
            let worker = WorkerLoop::new(
                collectors,
                Arc::clone(&self.shutdown),
                Arc::clone(&self.visible),
                Arc::clone(&self.warm_resume),
                Arc::clone(&self.warm_wake),
                Arc::clone(&self.sections),
            );
            handles.push(
                std::thread::Builder::new()
                    .name(name.to_string())
                    .spawn(move || worker.run())
                    .expect("failed to spawn warm telemetry worker"),
            );
        }
    }

    /// Signals shutdown, wakes every thread that might be sleeping, and
    /// joins them all. Bounded by the threads' own poll intervals (at most
    /// ~250ms), unlike 1.0, which had no shutdown path at all.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.hot_wake.notify();
        self.warm_wake.notify();
        let mut handles = self.handles.lock().unwrap();
        for h in handles.drain(..) {
            let _ = h.join();
        }
    }

    /// The latest published TCP/UDP connection table, if a `Connections`
    /// collector has run at least once. Read on demand (see
    /// `src-tauri`'s `get_connections` command) rather than folded into
    /// every hot frame — this can be a large, Warm-cadence dataset that
    /// only the Network panel needs, and only while it's open.
    pub fn latest_connections(
        &self,
    ) -> Option<crate::model::Sampled<Vec<crate::types::ConnectionSnapshot>>> {
        self.sections.lock().connections.clone()
    }

    /// The latest published SMBIOS inventory, if a `Hardware` collector has
    /// run at least once. Same on-demand rationale as `latest_connections`;
    /// this one is Cold-cadence and effectively static besides.
    pub fn latest_hardware(&self) -> Option<crate::model::Sampled<crate::types::SmbiosInfo>> {
        self.sections.lock().hardware.clone()
    }

    /// A throwaway `System`, refreshed once, for the one-shot `SystemInfo`
    /// IPC call — deliberately independent of the live collectors' shared
    /// state so this never contends with the sampling threads.
    pub fn one_shot_system_info() -> crate::types::SystemInfo {
        let mut sys = System::new();
        sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
        sys.refresh_memory();
        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        crate::types::SystemInfo {
            os_name: System::name().unwrap_or_else(|| "Unknown OS".to_string()),
            os_version: System::long_os_version().unwrap_or_else(|| "Unknown".to_string()),
            kernel_version: System::kernel_version().unwrap_or_default(),
            hostname: System::host_name().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_model,
            cpu_cores: sys.cpus().len(),
            total_memory: sys.total_memory(),
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stop_joins_every_thread_within_a_bound() {
        let scheduler = Scheduler::new();
        scheduler.spawn(vec![]);
        scheduler.set_visible(true);
        std::thread::sleep(Duration::from_millis(50));

        let start = std::time::Instant::now();
        scheduler.stop();
        // Threads poll at most every 250ms while hidden/idle; visible
        // threads wake on notify immediately. A generous bound catches a
        // real regression (a thread that never joins) without being flaky.
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "shutdown took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn hidden_scheduler_produces_no_frames() {
        let scheduler = Scheduler::new();
        scheduler.spawn(vec![]);
        // Never call set_visible(true).
        let mailbox = scheduler.frame_mailbox();
        let frame = mailbox.take_timeout(Duration::from_millis(150));
        scheduler.stop();
        assert!(frame.is_none(), "a hidden scheduler must not sample");
    }

    #[test]
    fn visible_scheduler_produces_frames_at_roughly_the_configured_interval() {
        let scheduler = Scheduler::new();
        // MIN_REFRESH_INTERVAL_MS (250ms) is the floor the hot loop clamps
        // to regardless of what's requested — use it directly so this test
        // observes the real cadence rather than a clamped one.
        scheduler.set_interval_ms(crate::settings::MIN_REFRESH_INTERVAL_MS);
        scheduler.spawn(vec![]);
        scheduler.set_visible(true);

        let mailbox = scheduler.frame_mailbox();
        let mut count = 0;
        let deadline = std::time::Instant::now() + Duration::from_millis(900);
        while std::time::Instant::now() < deadline {
            if mailbox.take_timeout(Duration::from_millis(300)).is_some() {
                count += 1;
            }
        }
        scheduler.stop();
        // ~900ms / 250ms ≈ 3-4 frames; generous bounds for CI jitter.
        assert!(count >= 2, "expected several frames, got {count}");
    }
}
