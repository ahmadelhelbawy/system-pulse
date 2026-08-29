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
    /// Handles/threads/process count/commit/pool/cache (Phase 1B). Hot
    /// cadence pending real-hardware timing validation — see
    /// `system-pulse-win::perf_info`'s module doc.
    pub windows_internal: Sampled<WindowsInternalState>,
    /// Derived/computed, not collected from hardware — provenance doesn't
    /// apply the same way. A scored, hysteresis-stabilized summary
    /// (Phase 2) rather than the raw per-tick alert list `health::analyze`
    /// produces — see `crate::alerts::AlertEngine`.
    pub health: HealthScore,
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
    /// Per-process GPU engine utilization, 0..=100. Sourced from PDH's
    /// `\GPU Engine(*)\Utilization Percentage` (Phase 1B) — vendor-neutral,
    /// unlike `gpu_mem` which NVML provides only for NVIDIA. `None` when
    /// neither source has data for this process, not a fabricated `0`.
    pub gpu_percent: Option<f32>,
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
    /// Stable identity across ticks (`category:title[:pid]`) — what
    /// `crate::alerts::AlertEngine` debounces on and what the frontend
    /// should key list rendering by, instead of array index (1.0's alerts
    /// were keyed by index, so a list reorder or a cleared alert above it
    /// silently reassigned every row's identity).
    pub id: String,
    pub severity: Severity,
    /// Stable machine-readable category: cpu | memory | disk | gpu | process.
    pub category: String,
    pub title: String,
    pub detail: String,
    /// Associated process id when the alert concerns a single process.
    pub pid: Option<u32>,
}

/// One domain's contribution to the overall score — "why did this number
/// move," not just the number itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DomainHealth {
    /// cpu | memory | disk | gpu | process — matches `HealthAlert::category`.
    pub domain: String,
    /// 0..=100. 100 minus a fixed penalty per active alert in this domain
    /// (see `crate::alerts`) — deterministic and explainable by
    /// construction, never a learned or opaque model.
    pub score: u8,
    /// Human-readable reasons this domain's score is below 100, most
    /// severe first — the active alerts' titles, not a separate text.
    pub contributors: Vec<String>,
}

/// Replaces 1.0's bare `Vec<HealthAlert>`: a single number for the status
/// bar/topology hero, per-domain breakdown for "why," and the stabilized
/// alert list (see `crate::alerts::AlertEngine`) for the Health panel.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HealthScore {
    /// 0..=100. The mean of `domains`' scores — deliberately not the
    /// minimum: one saturated domain should pull the number down, not
    /// zero it out, since the other domains are still healthy evidence.
    pub overall: u8,
    pub domains: Vec<DomainHealth>,
    pub alerts: Vec<HealthAlert>,
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

/// Windows internal state from a single `GetPerformanceInfo` call
/// (Phase 1B) — handles/threads/process count and the commit/pool/cache
/// figures Task Manager's Performance tab derives its "Committed" and
/// "Cached" numbers from. All byte fields are `PageSize * <count>`; the
/// raw struct reports pages, not bytes.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WindowsInternalState {
    pub handle_count: u32,
    pub process_count: u32,
    pub thread_count: u32,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub kernel_paged_pool: u64,
    pub kernel_non_paged_pool: u64,
    pub system_cache: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum TransportProtocol {
    Tcp,
    Udp,
}

/// TCP connection states (`MIB_TCP_STATE`). Always `None` for UDP, which is
/// connectionless.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    DeleteTcb,
}

/// One row from `GetExtendedTcpTable`/`GetExtendedUdpTable`
/// (`*_OWNER_PID_ALL`, Phase 1B) — process↔network attribution and
/// listening ports, unelevated. `pid` is `Some` whenever Windows could
/// attribute the connection to a process (always, in practice, for
/// `OWNER_PID` tables; modeled as optional because the underlying API
/// contract doesn't guarantee it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionSnapshot {
    pub protocol: TransportProtocol,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub state: Option<TcpState>,
    pub pid: Option<u32>,
}

/// One DIMM entry from an SMBIOS Type 17 (Memory Device) structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DimmInfo {
    pub manufacturer: Option<String>,
    pub part_number: Option<String>,
    pub size_bytes: Option<u64>,
    pub speed_mts: Option<u32>,
}

/// Board/BIOS/DIMM inventory parsed from the SMBIOS table
/// (`GetSystemFirmwareTable('RSMB')`, Phase 1B). Cold cadence, cached
/// forever after the first successful probe — this data cannot change
/// while the machine is running.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SmbiosInfo {
    pub board_vendor: Option<String>,
    pub board_product: Option<String>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_release_date: Option<String>,
    pub dimms: Vec<DimmInfo>,
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
                    gpu_percent: None,
                    exe: None,
                    user: None,
                    started_at: Some(UnixMillis(0)),
                }],
                Source::Sysinfo,
                UnixMillis(1),
            ),
            windows_internal: Sampled::ok(
                WindowsInternalState::default(),
                Source::PerfInfo,
                UnixMillis(1),
            ),
            health: HealthScore {
                overall: 90,
                domains: vec![DomainHealth {
                    domain: "memory".to_string(),
                    score: 90,
                    contributors: vec!["High memory".to_string()],
                }],
                alerts: vec![HealthAlert {
                    id: "memory:High memory".to_string(),
                    severity: Severity::Warning,
                    category: "memory".to_string(),
                    title: "High memory".to_string(),
                    detail: "detail".to_string(),
                    pid: None,
                }],
            },
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let back: TelemetrySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.timestamp_ms, snapshot.timestamp_ms);
        assert_eq!(back.cpu, snapshot.cpu);
        assert_eq!(back.gpu, snapshot.gpu);
        assert_eq!(back.processes, snapshot.processes);
        assert_eq!(back.health, snapshot.health);
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
