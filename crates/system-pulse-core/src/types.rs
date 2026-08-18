//! Serialized data-contract types shared with the frontend.
//!
//! These structs are the single source of truth for what crosses the IPC
//! boundary. The TypeScript mirror lives in `src/lib/contracts.ts` and must be
//! kept in sync. `#[serde(rename_all = "camelCase")]` keeps the JSON keys
//! idiomatic for the web frontend.

use serde::Serialize;

/// One full telemetry frame, emitted at the cheap interval (default 1 s).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    /// Wall-clock time the frame was assembled (unix ms).
    pub timestamp_ms: u64,
    pub uptime_secs: u64,
    pub cpu: CpuSnapshot,
    pub memory: MemorySnapshot,
    pub disk_io: DiskIoSnapshot,
    pub disks: Vec<DiskSnapshot>,
    pub networks: Vec<NetworkSnapshot>,
    pub gpu: Vec<GpuSnapshot>,
    pub processes: Vec<ProcessSnapshot>,
    pub health: Vec<HealthAlert>,
}

/// Raw, cumulative CPU tick counters used to derive utilization.
/// `total` must include `idle`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CpuSnapshot {
    /// Total CPU utilization across all cores, 0..=100.
    pub total_percent: f32,
    /// Per-core utilization, 0..=100, ordered by core index.
    pub per_core: Vec<f32>,
    /// Representative current frequency in MHz (best-effort).
    pub frequency_mhz: Option<u64>,
    pub core_count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub used_percent: f32,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    pub name: String,
    /// Receive throughput in bytes/second.
    pub download_rate: f64,
    /// Transmit throughput in bytes/second.
    pub upload_rate: f64,
    pub total_rx: u64,
    pub total_tx: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuSnapshot {
    pub name: String,
    pub utilization_percent: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub temperature_c: Option<u32>,
    pub power_w: Option<f32>,
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory: u64,
    pub gpu_mem: Option<u64>,
    pub exe: Option<String>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthAlert {
    pub severity: Severity,
    /// Stable machine-readable category: cpu | memory | disk | gpu | process.
    pub category: String,
    pub title: String,
    pub detail: String,
    /// Associated process id when the alert concerns a single process.
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
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
