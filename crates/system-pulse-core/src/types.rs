//! Serialized data-contract types shared with the frontend.
//!
//! These structs are the single source of truth for what crosses the IPC
//! boundary. The TypeScript mirror lives in `src/lib/contracts.ts` and must be
//! kept in sync. `#[serde(rename_all = "camelCase")]` keeps the JSON keys
//! idiomatic for the web frontend.
//!
//! Every type here derives both `Serialize` and `Deserialize` (Phase 1A):
//! this is what makes a captured probe NDJSON frame replayable back into a
//! `TelemetrySnapshot` in tests, instead of being a write-only wire format.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Sampled, UnixMillis};

/// One full telemetry frame, emitted at the cheap (hot) interval (default
/// 1 s). Each section is a `Sampled<T>` — see `crate::model` — so a
/// collector failure or an unsupported/needs-elevation state is always
/// distinguishable from a genuine reading, never a silent zero.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TelemetrySnapshot {
    /// Wall-clock time the frame was assembled.
    pub timestamp_ms: UnixMillis,
    pub uptime_secs: u64,
    pub cpu: Sampled<CpuSnapshot>,
    pub memory: Sampled<MemorySnapshot>,
    pub disk_io: Sampled<DiskIoSnapshot>,
    pub disks: Sampled<Vec<DiskSnapshot>>,
    pub networks: Sampled<Vec<NetworkSnapshot>>,
    pub gpu: Sampled<Vec<GpuSnapshot>>,
    pub processes: Sampled<Vec<ProcessSnapshot>>,
    /// Derived/computed, not collected from hardware — provenance doesn't
    /// apply the same way, so this stays a plain list. Reshaped into a
    /// scored `HealthScore` in Phase 2; untouched here.
    pub health: Vec<HealthAlert>,
}

/// Raw, cumulative CPU tick counters used to derive utilization.
/// `total` must include `idle`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CpuSnapshot {
    /// Total CPU utilization across all cores, 0..=100.
    pub total_percent: f32,
    /// Per-core utilization, 0..=100, ordered by core index.
    pub per_core: Vec<f32>,
    /// Representative current frequency in MHz (best-effort).
    pub frequency_mhz: Option<u64>,
    pub core_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemorySnapshot {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub used_percent: f32,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskIoSnapshot {
    /// Aggregate read throughput in bytes/second.
    pub read_rate: f64,
    /// Aggregate write throughput in bytes/second.
    pub write_rate: f64,
    /// Cumulative bytes read since sampling started.
    pub total_read: u64,
    /// Cumulative bytes written since sampling started.
    pub total_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskSnapshot {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub used_percent: f32,
    pub read_rate: f64,
    pub write_rate: f64,
    pub is_removable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NetworkSnapshot {
    pub name: String,
    /// Receive throughput in bytes/second.
    pub download_rate: f64,
    /// Transmit throughput in bytes/second.
    pub upload_rate: f64,
    pub total_rx: u64,
    pub total_tx: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GpuSnapshot {
    pub name: String,
    pub utilization_percent: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub temperature_c: Option<u32>,
    pub power_w: Option<f32>,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory: u64,
    pub gpu_mem: Option<u64>,
    pub exe: Option<String>,
    pub user: Option<String>,
    /// Process creation time, backing `ProcessIdentity` — required so
    /// terminating a process can be revalidated against the exact process
    /// the UI showed, not just a PID Windows may have already recycled.
    pub started_at: Option<UnixMillis>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HealthAlert {
    pub severity: Severity,
    /// Stable machine-readable category: cpu | memory | disk | gpu | process.
    pub category: String,
    pub title: String,
    pub detail: String,
    /// Associated process id when the alert concerns a single process.
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub hostname: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub total_memory: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Availability, Source};

    /// Every contract type must survive serialize -> deserialize -> equal,
    /// so a probe NDJSON capture can be replayed into tests and so no field
    /// silently regresses to serialize-only (which would re-block replay).
    #[test]
    fn telemetry_snapshot_round_trips_through_json() {
        let snapshot = TelemetrySnapshot {
            timestamp_ms: UnixMillis(1_700_000_000_000),
            uptime_secs: 12345,
            cpu: Sampled::ok(
                CpuSnapshot {
                    total_percent: 12.5,
                    per_core: vec![1.0, 2.0],
                    frequency_mhz: Some(3400),
                    core_count: 2,
                },
                Source::Sysinfo,
                UnixMillis(1),
            ),
            memory: Sampled::ok(MemorySnapshot::default(), Source::Sysinfo, UnixMillis(1)),
            disk_io: Sampled::ok(DiskIoSnapshot::default(), Source::Sysinfo, UnixMillis(1)),
            disks: Sampled::ok(vec![], Source::Sysinfo, UnixMillis(1)),
            networks: Sampled::ok(vec![], Source::Sysinfo, UnixMillis(1)),
            gpu: Sampled::unavailable(
                Availability::unsupported(crate::model::UnsupportedReason::DriverAbsent),
                Source::Nvml,
                UnixMillis(1),
            ),
            processes: Sampled::ok(
                vec![ProcessSnapshot {
                    pid: 1,
                    name: "init".to_string(),
                    cpu_percent: 0.0,
                    memory: 1024,
                    gpu_mem: None,
                    exe: None,
                    user: None,
                    started_at: Some(UnixMillis(0)),
                }],
                Source::Sysinfo,
                UnixMillis(1),
            ),
            health: vec![HealthAlert {
                severity: Severity::Warning,
                category: "memory".to_string(),
                title: "High memory".to_string(),
                detail: "detail".to_string(),
                pid: None,
            }],
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let back: TelemetrySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timestamp_ms, snapshot.timestamp_ms);
        assert_eq!(back.cpu, snapshot.cpu);
        assert_eq!(back.gpu, snapshot.gpu);
        assert_eq!(back.processes, snapshot.processes);
    }

    #[test]
    fn settings_and_system_info_round_trip() {
        let info = SystemInfo {
            os_name: "Test OS".to_string(),
            ..SystemInfo::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: SystemInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
    }
}
