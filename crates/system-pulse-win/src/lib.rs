//! Windows-only telemetry collectors: Phase 1B (`GetPerformanceInfo`,
//! TCP/UDP connection tables, PDH per-process GPU utilization, SMBIOS
//! hardware inventory), Phase 3 (services, drivers, startup entries,
//! installed software, Task Scheduler), and Phase 4 (storage health,
//! optional sensor bridge).
//!
//! Depends on `system-pulse-core` (for the `Collector` trait, provenance
//! model, and contract types) but is never depended on by it — the
//! reverse would be circular, and it's also *why* the contract types these
//! collectors produce (`WindowsInternalState`, `ConnectionSnapshot`,
//! `SmbiosInfo`, the GPU-attribution fields on `ProcessSnapshot`,
//! `ServiceSnapshot`, `StorageHealthSnapshot`, etc.) live in
//! `system-pulse-core::types` rather than here: only that crate's own
//! `cargo test` can run `ts-rs`'s export tests natively in this repo's
//! WSL2 dev environment, where this crate can only ever be
//! `cargo check --target x86_64-pc-windows-msvc`-verified, never executed.
//!
//! Every collector here has a real `cfg(windows)` implementation and a
//! `cfg(not(windows))` stub that reports `Unsupported` — so the workspace
//! still builds and tests headlessly on Linux, and so a collector that
//! turns out to be unreliable on some Windows configuration degrades to
//! the same honest "unavailable" state non-Windows hosts always see, never
//! a panic or a fabricated value.
//!
//! `com_spike` (Phase 3's prerequisite COM/WebView2 investigation) is not
//! a collector and isn't wired into `all_collectors()` — see its own doc
//! for the recorded finding that makes the Task Scheduler and sensor
//! bridge collectors safe to ship.

#![warn(unsafe_code)]

#[cfg(target_os = "windows")]
pub mod com_spike;
pub mod drivers;
pub mod installed_software;
pub mod pdh_gpu;
pub mod perf_info;
pub mod scheduled_tasks;
pub mod sensor_bridge;
pub mod services;
pub mod smbios;
pub mod startup;
pub mod storage_health;
pub mod tcp_table;

pub use drivers::DriversCollector;
pub use installed_software::InstalledSoftwareCollector;
pub use pdh_gpu::PdhGpuCollector;
pub use perf_info::PerfInfoCollector;
pub use scheduled_tasks::ScheduledTasksCollector;
pub use sensor_bridge::SensorBridgeCollector;
pub use services::ServicesCollector;
pub use smbios::SmbiosCollector;
pub use startup::StartupCollector;
pub use storage_health::StorageHealthCollector;
pub use tcp_table::TcpTableCollector;

use system_pulse_core::collector::{Collector, CollectorCapability};

/// Probes a fresh instance of every collector in this crate, mirroring
/// `system_pulse_core::collector::probe_capabilities` — kept as a sibling
/// function here (not merged into that one) because core cannot depend on
/// this crate. Callers (the Tauri shell's `get_capabilities` command)
/// concatenate both.
pub fn probe_capabilities() -> Vec<CollectorCapability> {
    let probe_one = |mut c: Box<dyn Collector>| CollectorCapability {
        id: c.id(),
        required_privilege: c.required_privilege(),
        availability: c.probe(),
    };
    vec![
        probe_one(Box::<PerfInfoCollector>::default()),
        probe_one(Box::<TcpTableCollector>::default()),
        probe_one(Box::<SmbiosCollector>::default()),
        probe_one(Box::<PdhGpuCollector>::default()),
        probe_one(Box::<ServicesCollector>::default()),
        probe_one(Box::<DriversCollector>::default()),
        probe_one(Box::<StartupCollector>::default()),
        probe_one(Box::<InstalledSoftwareCollector>::default()),
        probe_one(Box::<ScheduledTasksCollector>::default()),
        probe_one(Box::<StorageHealthCollector>::default()),
        probe_one(Box::<SensorBridgeCollector>::default()),
    ]
}

/// Constructs one instance of every collector in this crate, ready to hand
/// to `Scheduler::spawn`/`TelemetryService::spawn`.
pub fn all_collectors() -> Vec<Box<dyn Collector>> {
    vec![
        Box::<PerfInfoCollector>::default(),
        Box::<TcpTableCollector>::default(),
        Box::<SmbiosCollector>::default(),
        Box::<PdhGpuCollector>::default(),
        Box::<ServicesCollector>::default(),
        Box::<DriversCollector>::default(),
        Box::<StartupCollector>::default(),
        Box::<InstalledSoftwareCollector>::default(),
        Box::<ScheduledTasksCollector>::default(),
        Box::<StorageHealthCollector>::default(),
        Box::<SensorBridgeCollector>::default(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_all_eleven_collectors() {
        let caps = probe_capabilities();
        assert_eq!(caps.len(), 11);
    }

    #[test]
    fn all_collectors_constructs_eleven() {
        assert_eq!(all_collectors().len(), 11);
    }
}
