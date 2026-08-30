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
    /// Statistically unusual readings (Phase 5) — deliberately separate
    /// from `health.alerts`: these flag a deviation from this machine's own
    /// recent pattern, not a crossed absolute threshold, and folding them
    /// into the health score would conflate two different kinds of signal.
    /// Debounced the same way health alerts are — see
    /// `crate::analysis::anomaly` and `crate::alerts::HysteresisEngine`.
    /// `#[serde(default)]` so the Phase 0-4 replay fixtures (captured
    /// before this field existed) still deserialize, as an empty list —
    /// the honest answer, since Phase 5 anomaly detection didn't exist
    /// when they were recorded.
    #[serde(default)]
    pub anomalies: Vec<HealthAlert>,
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

/// `SERVICE_STATUS.dwCurrentState` (Phase 3), from `EnumServicesStatusExW`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServiceStatus {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
}

/// A service's configured start type (`QueryServiceConfigW`'s
/// `dwStartType`) — independent of whether it's currently running.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServiceStartType {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
}

/// One row from the Service Control Manager (`OpenSCManagerW` +
/// `EnumServicesStatusExW`, Phase 3). No COM. Read-only: starting/stopping
/// a service needs admin and isn't in scope here (see the plan's A1/A2
/// capability matrix).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceSnapshot {
    /// The SCM key name (e.g. `"wuauserv"`), not the display name.
    pub name: String,
    pub display_name: String,
    pub status: ServiceStatus,
    /// `None` when the per-service config query failed (a transient
    /// handle/permission issue, distinct from the service itself being
    /// unenumerable) — the row is still shown with everything else known
    /// about it real, rather than dropped entirely for one missing field.
    pub start_type: Option<ServiceStartType>,
    /// The owning process, when running and the service isn't sharing a
    /// `svchost.exe` in a way that makes a single pid meaningless — `None`
    /// covers both "stopped" and "not resolvable."
    pub pid: Option<u32>,
}

/// One row from `EnumDeviceDrivers` (psapi) + SetupAPI (Phase 3). Kernel
/// driver names/base addresses come from the former; the human-readable
/// description and version, when available, from the latter — never
/// fabricated when SetupAPI doesn't have a matching entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DriverSnapshot {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub base_address: u64,
}

/// Where a startup entry was found (Phase 3) — Run keys, RunOnce keys, and
/// Startup folders each have distinct semantics worth keeping visible
/// rather than collapsing into one bag.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StartupLocation {
    HkcuRun,
    HklmRun,
    HkcuRunOnce,
    HklmRunOnce,
    UserStartupFolder,
    CommonStartupFolder,
}

/// One autostart entry (Phase 3): Run/RunOnce registry keys plus Startup
/// folder shortcuts, cross-referenced against `StartupApproved` for the
/// user-facing enabled/disabled state Task Manager's Startup tab shows
/// (a Run-key entry isn't removed when a user disables it there — a
/// sibling `StartupApproved` value is flipped instead).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    pub location: StartupLocation,
    pub enabled: bool,
}

/// One entry from the Uninstall registry (Phase 3: HKLM + HKCU, both the
/// native and `WOW6432Node` views) — **never** `Win32_Product` (WMI),
/// which silently triggers an MSI reconfiguration of every installed
/// package as a side effect of merely enumerating it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledSoftware {
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    /// Stored verbatim as the registry has it (commonly `YYYYMMDD`, but
    /// not universally, so this is left as an opaque display string
    /// rather than parsed into a real date and risking a fabricated one).
    pub install_date: Option<String>,
}

/// One task from Task Scheduler 2.0 COM (`ITaskService`, Phase 3) — in
/// scope per the COM/WebView2 spike's finding (see
/// `system-pulse-win::com_spike`): safe from a dedicated MTA worker
/// thread using `CoSetProxyBlanket` per proxy, no process-wide COM
/// security call. Some tasks are enumerable only when elevated; those are
/// simply absent from the list rather than causing a global failure (see
/// the plan's Phase 3 risk note).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduledTaskSnapshot {
    /// Full path, e.g. `\Microsoft\Windows\Maintenance\WinSAT`.
    pub path: String,
    pub enabled: bool,
    pub last_run_time: Option<UnixMillis>,
    pub next_run_time: Option<UnixMillis>,
    /// The last run's HRESULT/exit code, when the task has run at least
    /// once; `0` means success, matching Task Scheduler's own convention.
    pub last_task_result: Option<u32>,
}

/// `STORAGE_BUS_TYPE` (Phase 4) — which transport a physical drive is
/// attached over; not exhaustive of the Win32 enum, but every value a
/// real consumer disk realistically reports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StorageBusType {
    Ata,
    Sata,
    Scsi,
    Sas,
    Usb,
    Nvme,
    Raid,
    Virtual,
    Other,
}

/// One physical drive's health, from `IOCTL_STORAGE_QUERY_PROPERTY` +
/// `IOCTL_STORAGE_PREDICT_FAILURE` (Phase 4) — needs an elevated process
/// to even open `\\.\PhysicalDriveN`. Every field is independently
/// `Option`: a value this machine's driver/hardware doesn't report is
/// `None`, never a fabricated 0/false/"good" — see the master plan's
/// storage-health acceptance criterion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageHealthSnapshot {
    /// e.g. `\\.\PhysicalDrive0`.
    pub device: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub bus_type: Option<StorageBusType>,
    pub size_bytes: Option<u64>,
    pub temperature_c: Option<i32>,
    /// `IOCTL_STORAGE_PREDICT_FAILURE`'s own verdict — the drive's
    /// firmware/controller judged its own SMART data, not this app
    /// interpreting raw attribute thresholds itself (see
    /// `system-pulse-win::storage_health`'s module doc for why).
    pub predicted_failure: Option<bool>,
}

/// One reading from the optional sensor bridge (Phase 4) — see
/// `system-pulse-win::sensor_bridge`. Deliberately never installs or
/// launches anything; only reads from a source the user already runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SensorReading {
    pub name: String,
    /// The bridge source's own sensor-type label (e.g. `"Temperature"`,
    /// `"Fan"`, `"Voltage"`, `"Load"`, `"Power"`, `"Clock"`) passed
    /// through as-is rather than mapped into a closed enum here — the
    /// bridge's entire point is showing whatever an external tool
    /// already measured, not this app inventing a taxonomy for hardware
    /// it never queries directly.
    pub kind: String,
    pub value: f64,
}

/// The full sensor-bridge result for one tick. `source: None` (with an
/// empty `readings`) means no supported bridge was found running — never
/// distinguished from "found but had zero sensors," since both look
/// identical to the UI ("nothing to show") and the *reason* is visible
/// instead through this collector's own `Availability`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SensorBridgeSnapshot {
    pub source: Option<String>,
    pub readings: Vec<SensorReading>,
}

/// `EVT_SYSTEM_PROPERTY_ID(EvtSystemLevel)`'s value, mapped to a closed
/// enum — Windows event levels 0 (LogAlways) and 4 (Information) both
/// collapse to `Information` here since neither carries extra meaning for
/// this app; an unrecognized value also falls back to `Information` rather
/// than guessing at severity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EventLevel {
    Critical,
    Error,
    Warning,
    Information,
    Verbose,
}

/// One Windows Event Log record (Phase 5) — see
/// `system-pulse-win::event_log`. `message` is best-effort (`EvtFormatMessage`
/// against the provider's own metadata) and `None` when that lookup fails
/// for any reason (provider metadata absent, message table missing) —
/// never a fabricated or truncated guess at what the event meant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventLogEntry {
    /// e.g. `"Application"`, `"System"`, `"Security"`.
    pub channel: String,
    pub record_id: u64,
    pub event_id: u32,
    pub level: EventLevel,
    pub provider: String,
    pub time_created: UnixMillis,
    pub message: Option<String>,
}

/// The bounded, incrementally-read event log window for one collection
/// cycle (Phase 5). `dropped` is the ring's own overflow counter (see
/// `crate::transport::BoundedRing`) — visible rather than a silent gap, per
/// the master plan's backpressure rule for event-like topics.
/// `security_included` is `false` whenever the Security channel was
/// skipped because the process isn't elevated: the collector still reports
/// `Availability::Ok` for the Application/System channels it *could* read
/// rather than failing the whole snapshot over one gated channel.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventLogSnapshot {
    /// Oldest first — the order `BoundedRing` iterates and the order new
    /// records were discovered in.
    pub entries: Vec<EventLogEntry>,
    pub dropped: u64,
    pub security_included: bool,
}

/// `INetFwPolicy2`'s per-profile enabled state (Phase 5). `Unknown` is a
/// real, distinct value — reported when the profile's own COM call fails —
/// never coerced to `Off` (which would fabricate a security-relevant
/// negative) or `On` (which would hide a real problem).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FirewallProfileState {
    On,
    Off,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FirewallStatus {
    pub domain: FirewallProfileState,
    pub private: FirewallProfileState,
    pub public: FirewallProfileState,
}

/// One `WscGetSecurityProviderHealth` provider's reported health (Phase 5)
/// — `health` is the WSC API's own word (`"good"` | `"notMonitored"` |
/// `"poor"` | `"snooze"`), passed through rather than reinterpreted, since
/// WSC (not this app) is the authority on what "good" means for a given AV
/// product.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecurityProviderStatus {
    /// "antivirus" | "autoUpdate" — which WSC provider category this is;
    /// firewall health is reported via `FirewallStatus` instead, since WSC's
    /// firewall bit only says "some profile is protected," while
    /// `INetFwPolicy2` gives the real per-profile breakdown.
    pub kind: String,
    pub health: String,
}

/// One deterministic, rule-based finding from the persistence checks
/// (Phase 5) — an autostart entry or scheduled task whose target looks
/// suspicious (missing file, unusual location, an unsigned binary). `signed`
/// is `None` whenever `WinVerifyTrust` wasn't run or couldn't reach a
/// verdict — never coerced to `Some(false)`, which would fabricate "this is
/// definitely unsigned" from "this app didn't check."
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersistenceFinding {
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub path: Option<String>,
    pub signed: Option<bool>,
}

/// Security Center + firewall + Secure Boot + persistence checks for one
/// collection cycle (Phase 5). Persistence findings are computed on demand
/// from already-collected Phase 3 data (see `system-pulse-win::security_posture`'s
/// module doc) rather than by this collector itself, so they aren't part of
/// this snapshot — see the `get_persistence_findings` IPC command instead.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecurityPostureSnapshot {
    pub firewall: Option<FirewallStatus>,
    pub antivirus: Vec<SecurityProviderStatus>,
    /// `HKLM\SYSTEM\CurrentControlSet\Control\SecureBoot\State\UEFISecureBootEnabled`
    /// — `None` on non-UEFI/legacy-BIOS systems where the key doesn't exist
    /// at all, distinct from `Some(false)` (UEFI present, Secure Boot off).
    pub secure_boot_enabled: Option<bool>,
}

/// One historical sample cited as evidence for a [`DiagnosticFinding`] —
/// literally a `query_history` result point, never a synthesized or
/// interpolated value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EvidencePoint {
    pub ts_ms: UnixMillis,
    pub value: f64,
}

/// A correlated diagnostic finding (Phase 5) — an active alert enriched
/// with the actual historical evidence behind it (see
/// `crate::analysis::diagnostics::correlate`). `evidence` is empty and
/// `duration_ms` is `0` whenever history has no data for this finding's
/// series (history disabled, or too new to have any samples yet) — the
/// finding is still reported (it's real right now), but nothing about its
/// past is invented.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagnosticFinding {
    /// Same identity as the alert this was correlated from.
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Associated process id when the underlying alert concerns a single
    /// process — process-category alerts already carry their own evidence
    /// (the process name/id is the finding), so `evidence` is empty for
    /// these rather than a fabricated historical lookup this app has no
    /// per-process history to back.
    pub pid: Option<u32>,
    /// Milliseconds this condition has been continuously present in
    /// recorded history, estimated from the oldest evidence point still at
    /// or above the alert's own threshold. `0` when there's no evidence.
    pub duration_ms: i64,
    pub evidence: Vec<EvidencePoint>,
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
            anomalies: vec![],
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
