//! The collector abstraction: a pluggable unit of telemetry gathering with
//! its own cadence, privilege requirement, and availability probing.
//!
//! Deliberately typed, not dynamic: [`CollectorOutput`] is a closed enum of
//! the same sections `TelemetrySnapshot` already has, so serde, a future
//! `ts-rs` codegen pass, and NDJSON replay all keep working unmodified. A
//! `HashMap<String, serde_json::Value>` section bag was considered and
//! rejected — see the master plan's self-critique.

mod cpu;
mod disk;
mod gpu;
mod memory;
mod network;
mod process;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use gpu::GpuCollector;
pub use memory::MemoryCollector;
pub use network::NetworkCollector;
pub use process::ProcessCollector;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::UnixMillis;
use crate::model::{Availability, Sampled};
use crate::types::{
    ConnectionSnapshot, CpuSnapshot, DiskIoSnapshot, DiskSnapshot, DriverSnapshot,
    EventLogSnapshot, GpuSnapshot, InstalledSoftware, MemorySnapshot, NetworkSnapshot,
    ProcessSnapshot, ScheduledTaskSnapshot, SecurityPostureSnapshot, SensorBridgeSnapshot,
    ServiceSnapshot, SmbiosInfo, StartupItem, StorageHealthSnapshot, WindowsInternalState,
};

/// Identifies a collector for scheduling, logging, and capability reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CollectorId {
    Cpu,
    Memory,
    Disk,
    Network,
    Gpu,
    Process,
    WindowsInternal,
    Connections,
    Hardware,
    PdhGpu,
    Services,
    Drivers,
    Startup,
    InstalledSoftware,
    ScheduledTasks,
    StorageHealth,
    SensorBridge,
    EventLog,
    SecurityPosture,
}

/// How often a collector should run, and on which thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    /// Every hot-thread tick. Must complete in a couple of milliseconds.
    Hot,
    /// Wall-clock interval, dispatched on the worker pool.
    Warm(Duration),
    /// Wall-clock interval, long TTL, may block. Worker pool.
    Cold(Duration),
    /// User-initiated only; the scheduler never calls this on a timer.
    OnDemand,
}

/// The minimum privilege a collector needs to produce real data. Purely
/// descriptive in Phase 1A (no collector here needs more than `User`) — it
/// exists now so `get_capabilities` has something honest to report and so
/// later phases don't need to touch this enum to add elevated collectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Privilege {
    User,
    Admin,
    Driver,
}

/// Context handed to [`Collector::collect`] for one invocation.
pub struct CollectCtx {
    /// Monotonic clock reading for this invocation — the only thing rate
    /// math may be derived from.
    pub now: Instant,
    /// Wall-clock reading, for stamping `Sampled::as_of` only.
    pub wall_now: UnixMillis,
}

/// One collector's result for one invocation. A closed enum, not a section
/// name plus a `dyn Any` — see the module doc for why.
pub enum CollectorOutput {
    Cpu(Sampled<CpuSnapshot>),
    Memory(Sampled<MemorySnapshot>),
    Disk {
        disks: Sampled<Vec<DiskSnapshot>>,
        io: Sampled<DiskIoSnapshot>,
    },
    Network(Sampled<Vec<NetworkSnapshot>>),
    Gpu {
        devices: Sampled<Vec<GpuSnapshot>>,
        /// pid -> total GPU memory (bytes) across devices, for the process
        /// assembly join step. Not itself rendered — see `ProcessCollector`.
        process_mem: HashMap<u32, u64>,
    },
    Process(Sampled<Vec<ProcessSnapshot>>),
    /// `GetPerformanceInfo` (Phase 1B) — implemented in `system-pulse-win`.
    WindowsInternal(Sampled<WindowsInternalState>),
    /// `GetExtendedTcpTable`/`UdpTable` (Phase 1B) — implemented in
    /// `system-pulse-win`. Not part of the hot frame: read on demand by the
    /// `get_connections` IPC command so it only flows while the Network
    /// panel is open, not into every 1 Hz frame regardless of who's looking.
    Connections(Sampled<Vec<ConnectionSnapshot>>),
    /// `GetSystemFirmwareTable('RSMB')` (Phase 1B) — implemented in
    /// `system-pulse-win`. Cold/cache-forever; read on demand like
    /// `Connections`, never part of the hot frame.
    Hardware(Sampled<SmbiosInfo>),
    /// PDH `\GPU Engine(*)` (Phase 1B) — implemented in `system-pulse-win`.
    /// Per-process utilization is vendor-neutral and always attempted
    /// regardless of NVML's presence (NVML never provided per-process
    /// utilization, only VRAM); `device_fallback` is populated only when
    /// NVML is unavailable, completing the fallback ladder described in the
    /// master plan (NVML richest -> PDH vendor-neutral -> Unsupported).
    PdhGpu {
        per_process_percent: HashMap<u32, f32>,
        device_fallback: Option<Sampled<Vec<GpuSnapshot>>>,
    },
    /// SCM (`OpenSCManagerW`/`EnumServicesStatusExW`, Phase 3) — implemented
    /// in `system-pulse-win`. No COM. Cold/on-demand, like `Connections`.
    Services(Sampled<Vec<ServiceSnapshot>>),
    /// `EnumDeviceDrivers` + SetupAPI (Phase 3) — implemented in
    /// `system-pulse-win`. No COM.
    Drivers(Sampled<Vec<DriverSnapshot>>),
    /// Run/RunOnce keys + Startup folders + `StartupApproved` (Phase 3) —
    /// implemented in `system-pulse-win`. No COM.
    Startup(Sampled<Vec<StartupItem>>),
    /// Uninstall registry keys, HKLM+HKCU+WOW6432Node (Phase 3) —
    /// implemented in `system-pulse-win`. No COM; never `Win32_Product`.
    InstalledSoftware(Sampled<Vec<InstalledSoftware>>),
    /// Task Scheduler 2.0 COM (Phase 3) — implemented in
    /// `system-pulse-win`, gated on the COM/WebView2 spike's finding (see
    /// `system-pulse-win::com_spike`): safe from a dedicated MTA worker
    /// thread with `CoSetProxyBlanket` per proxy, no process-wide COM
    /// security call.
    ScheduledTasks(Sampled<Vec<ScheduledTaskSnapshot>>),
    /// `IOCTL_STORAGE_QUERY_PROPERTY`/`IOCTL_STORAGE_PREDICT_FAILURE`
    /// (Phase 4) — implemented in `system-pulse-win`. Needs elevation to
    /// open a physical drive handle at all.
    StorageHealth(Sampled<Vec<StorageHealthSnapshot>>),
    /// The optional LibreHardwareMonitor/HWiNFO sensor bridge (Phase 4) —
    /// implemented in `system-pulse-win`. Read-only, opt-in by the mere
    /// fact of the external tool already running; never installs or
    /// launches anything.
    SensorBridge(Sampled<SensorBridgeSnapshot>),
    /// `EvtQuery`/`EvtNext` with bookmarked incremental reads (Phase 5) —
    /// implemented in `system-pulse-win`. Bounded, dropped-count-visible;
    /// the Security channel is gated on elevation inside the collector
    /// itself (`EventLogSnapshot::security_included`), not by this
    /// collector's own `required_privilege()`, since Application/System
    /// stay readable either way.
    EventLog(Sampled<EventLogSnapshot>),
    /// Windows Security Center + firewall + Secure Boot (Phase 5) —
    /// implemented in `system-pulse-win`. Defensive persistence checks are
    /// computed on demand from already-collected Phase 3 data instead of
    /// living on this collector — see `system-pulse-win::security_posture`.
    SecurityPosture(Sampled<SecurityPostureSnapshot>),
}

/// A pluggable source of telemetry.
pub trait Collector: Send {
    fn id(&self) -> CollectorId;
    fn cadence(&self) -> Cadence;
    fn required_privilege(&self) -> Privilege;

    /// Called once at startup (and safe to call again to re-probe hardware
    /// that might have appeared/disappeared). Establishes the baseline
    /// availability `collect` should assume when it can't cheaply re-check.
    fn probe(&mut self) -> Availability;

    /// Called on this collector's `cadence`. Must itself decide the
    /// `Availability` of the result it returns — a probe failure earlier
    /// does not exempt `collect` from returning a well-formed, unavailable
    /// `Sampled` value rather than panicking or fabricating data.
    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput;

    /// Called when sampling resumes after being paused (window hidden).
    /// Collectors that track a rate baseline (a previous sample + when it
    /// was taken) must clear it here, so the first post-resume tick reports
    /// "no rate yet" rather than averaging over however long the pause was
    /// (the 1.0 "stale baselines after resume" defect). A no-op default:
    /// most collectors don't carry a baseline at all.
    fn reset_baseline(&mut self) {}
}

/// One collector's availability on this machine, independent of whether a
/// live `Scheduler` happens to be running — see [`probe_capabilities`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectorCapability {
    pub id: CollectorId,
    pub required_privilege: Privilege,
    pub availability: Availability,
}

/// Probes a fresh instance of every collector and reports what this machine
/// can actually do — the "capability probing is a first-class startup
/// phase" requirement. Deliberately independent of any running
/// `Scheduler`/live collector state: each collector here is newly
/// constructed and immediately dropped, so this never contends with the
/// sampling threads and can be called at any time (e.g. from an IPC
/// command) without touching shared state.
pub fn probe_capabilities() -> Vec<CollectorCapability> {
    let sys = std::sync::Arc::new(parking_lot::Mutex::new(sysinfo::System::new()));

    let probe_one = |mut c: Box<dyn Collector>| CollectorCapability {
        id: c.id(),
        required_privilege: c.required_privilege(),
        availability: c.probe(),
    };

    vec![
        probe_one(Box::new(CpuCollector::new(std::sync::Arc::clone(&sys)))),
        probe_one(Box::new(MemoryCollector::new(std::sync::Arc::clone(&sys)))),
        probe_one(Box::<DiskCollector>::default()),
        probe_one(Box::<NetworkCollector>::default()),
        probe_one(Box::<GpuCollector>::default()),
        probe_one(Box::new(ProcessCollector::new(sys))),
    ]
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[test]
    fn probes_all_six_collectors() {
        let caps = probe_capabilities();
        assert_eq!(caps.len(), 6);
        let ids: std::collections::HashSet<_> = caps.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 6, "every collector id must be represented once");
    }

    #[test]
    fn every_collector_reports_user_privilege_in_phase_1a() {
        // No Phase 1A collector needs more than standard-user privilege —
        // this pins that fact so a future collector that silently needs
        // more doesn't slip through unnoticed.
        for cap in probe_capabilities() {
            assert_eq!(cap.required_privilege, Privilege::User);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared helpers for collector unit tests: a fake `CpuTimesSource` and
    //! a `CollectCtx` builder, so each collector's own tests don't need to
    //! touch real hardware or the OS clock.

    use super::*;
    use crate::platform::CpuTimesSource;
    use crate::types::CpuTimes;
    use std::sync::Mutex;

    pub struct ScriptedCpuTimes {
        readings: Mutex<std::vec::IntoIter<Option<CpuTimes>>>,
    }

    impl ScriptedCpuTimes {
        pub fn new(readings: Vec<Option<CpuTimes>>) -> Self {
            Self {
                readings: Mutex::new(readings.into_iter()),
            }
        }
    }

    impl CpuTimesSource for ScriptedCpuTimes {
        fn read(&self) -> Option<CpuTimes> {
            self.readings.lock().unwrap().next().flatten()
        }
    }

    pub fn ctx_at(now: Instant) -> CollectCtx {
        CollectCtx {
            now,
            wall_now: UnixMillis(0),
        }
    }
}
