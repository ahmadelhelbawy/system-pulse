//! IPC commands exposed to the frontend.
//!
//! Every command validates its inputs and returns a typed result. There is no
//! generic "run this" or shell-execution command by design: the frontend can
//! only invoke the specific, audited operations declared here.

use system_pulse_core::analysis::diagnostics::correlate;
use system_pulse_core::collector::{probe_capabilities, CollectorCapability};
use system_pulse_core::history::{HistoryPoint, SeriesId, TimeRange};
use system_pulse_core::model::{Sampled, UnixMillis};
use system_pulse_core::process::{kill_process as core_kill_process, ProcessIdentity};
use system_pulse_core::types::{
    ConnectionSnapshot, DiagnosticFinding, DriverSnapshot, EventLogSnapshot, HealthAlert,
    InstalledSoftware, PersistenceFinding, ScheduledTaskSnapshot, SecurityPostureSnapshot,
    SensorBridgeSnapshot, ServiceSnapshot, SmbiosInfo, StartupItem, StorageHealthSnapshot,
};
use system_pulse_core::{Settings, SystemInfo};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::AppState;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, AppError> {
    let mut next = settings;
    next.sanitize();

    let previous_hotkey = state.settings.lock().unwrap().hotkey.clone();
    if next.hotkey != previous_hotkey {
        let hotkey = next
            .parsed_hotkey()
            .map_err(|e| AppError::InvalidSettings {
                message: e.to_string(),
            })?;
        crate::windows::register_hotkey(&app, &hotkey).map_err(AppError::from)?;
    }

    state.telemetry.set_interval_ms(next.refresh_interval_ms);
    crate::windows::set_autostart(next.launch_at_startup).map_err(AppError::from)?;
    crate::settings::save(&app, &next).map_err(AppError::from)?;

    *state.settings.lock().unwrap() = next.clone();
    Ok(next)
}

#[tauri::command]
pub fn set_visibility(state: State<'_, AppState>, visible: bool) {
    state.telemetry.set_visible(visible);
}

/// Terminates a process, but only the exact process `identity` names —
/// see `system_pulse_core::process::ProcessIdentity`. The frontend sends
/// back the full identity it received in the process list, not a bare pid,
/// so a PID Windows has already recycled can never be killed by mistake.
#[tauri::command]
pub fn kill_process(identity: ProcessIdentity) -> Result<(), AppError> {
    core_kill_process(identity).map_err(AppError::from)
}

#[tauri::command]
pub fn is_elevated() -> bool {
    crate::windows::is_elevated()
}

/// Relaunches the app elevated (UAC), then exits this instance — see
/// `crate::windows::request_elevation`. Always user-initiated; nothing
/// calls this automatically.
#[tauri::command]
pub fn request_elevation(app: AppHandle) -> Result<(), AppError> {
    crate::windows::request_elevation(&app).map_err(AppError::from)
}

#[tauri::command]
pub fn get_system_info(state: State<'_, AppState>) -> SystemInfo {
    state.telemetry.system_info()
}

/// Reports what this machine can actually measure, independent of whether
/// telemetry is currently running — "capability probing is a first-class
/// startup phase" (see the master plan's provenance model). Concatenates
/// `system-pulse-core`'s own collectors with `system-pulse-win`'s, since
/// core cannot depend on the Windows crate to enumerate them itself.
#[tauri::command]
pub fn get_capabilities() -> Vec<CollectorCapability> {
    let mut caps = probe_capabilities();
    caps.extend(system_pulse_win::probe_capabilities());
    caps
}

/// Network connections + owning PID (Warm 2s cadence) — an on-demand read
/// of whatever the background collector last published, not a live push;
/// the Network panel is expected to poll this while it's the active tab
/// rather than receiving every 2s tick unconditionally.
#[tauri::command]
pub fn get_connections(state: State<'_, AppState>) -> Option<Sampled<Vec<ConnectionSnapshot>>> {
    state.telemetry.latest_connections()
}

/// Motherboard/BIOS/DIMM inventory (Cold cadence, cached forever after the
/// first successful read) — same on-demand-read shape as `get_connections`.
#[tauri::command]
pub fn get_hardware_info(state: State<'_, AppState>) -> Option<Sampled<SmbiosInfo>> {
    state.telemetry.latest_hardware()
}

/// Queries recorded telemetry history for the Trends panel. `range`/`series`
/// select what to fetch; the backend picks the coarsest rollup granularity
/// that still covers the range (see `system_pulse_core::history::HistoryStore`)
/// so a wide range stays fast without the frontend needing to know about
/// rollups at all.
#[tauri::command]
pub fn query_history(
    state: State<'_, AppState>,
    range: TimeRange,
    series: SeriesId,
) -> Result<Vec<HistoryPoint>, AppError> {
    state
        .telemetry
        .query_history(range, series)
        .map_err(AppError::from)
}

/// Service Control Manager list (Phase 3, Cold cadence) — same on-demand
/// shape as `get_connections`/`get_hardware_info`.
#[tauri::command]
pub fn get_services(state: State<'_, AppState>) -> Option<Sampled<Vec<ServiceSnapshot>>> {
    state.telemetry.latest_services()
}

/// Loaded kernel driver list (Phase 3, Cold cadence).
#[tauri::command]
pub fn get_drivers(state: State<'_, AppState>) -> Option<Sampled<Vec<DriverSnapshot>>> {
    state.telemetry.latest_drivers()
}

/// Autostart entries — Run/RunOnce keys and Startup folders (Phase 3,
/// Cold cadence).
#[tauri::command]
pub fn get_startup(state: State<'_, AppState>) -> Option<Sampled<Vec<StartupItem>>> {
    state.telemetry.latest_startup()
}

/// Installed software from the Uninstall registry keys (Phase 3, Cold
/// cadence).
#[tauri::command]
pub fn get_installed_software(
    state: State<'_, AppState>,
) -> Option<Sampled<Vec<InstalledSoftware>>> {
    state.telemetry.latest_installed_software()
}

/// Task Scheduler entries (Phase 3, Cold cadence) — see
/// `system_pulse_win::com_spike` for why this is safe to use in-process.
#[tauri::command]
pub fn get_scheduled_tasks(
    state: State<'_, AppState>,
) -> Option<Sampled<Vec<ScheduledTaskSnapshot>>> {
    state.telemetry.latest_scheduled_tasks()
}

/// Physical drive health via `DeviceIoControl` (Phase 4, Cold cadence) —
/// `NeedsElevation` unless the app is running elevated, since opening a
/// `\\.\PhysicalDriveN` handle at all requires admin.
#[tauri::command]
pub fn get_storage_health(
    state: State<'_, AppState>,
) -> Option<Sampled<Vec<StorageHealthSnapshot>>> {
    state.telemetry.latest_storage_health()
}

/// Optional LibreHardwareMonitor WMI sensor bridge (Phase 4, Cold cadence) —
/// `Unsupported { DriverAbsent }` when LibreHardwareMonitor isn't running,
/// never a fabricated reading.
#[tauri::command]
pub fn get_sensor_bridge(state: State<'_, AppState>) -> Option<Sampled<SensorBridgeSnapshot>> {
    state.telemetry.latest_sensor_bridge()
}

/// Windows Event Log (Phase 5, Cold cadence) — bounded, bookmarked
/// incremental reads; `securityIncluded` on the snapshot tells the
/// frontend whether the Security channel is actually being read right now
/// (it's gated on elevation inside the collector itself, not by this
/// command).
#[tauri::command]
pub fn get_event_log(state: State<'_, AppState>) -> Option<Sampled<EventLogSnapshot>> {
    state.telemetry.latest_event_log()
}

/// Security Center + firewall + Secure Boot (Phase 5, Cold cadence).
/// Persistence findings are a separate, on-demand command (below) rather
/// than part of this snapshot — see `system_pulse_win::security_posture`'s
/// module doc.
#[tauri::command]
pub fn get_security_posture(
    state: State<'_, AppState>,
) -> Option<Sampled<SecurityPostureSnapshot>> {
    state.telemetry.latest_security_posture()
}

/// Correlates the frontend's currently-active alerts (health + anomaly —
/// the caller concatenates `health.alerts` and `anomalies` from its latest
/// telemetry frame) against recorded history, producing evidence-bearing
/// diagnostic findings (Phase 5). Pure correlation logic lives in
/// `system_pulse_core::analysis::diagnostics`; this command only supplies
/// the `query_history` closure that logic needs.
#[tauri::command]
pub fn get_diagnostics(
    state: State<'_, AppState>,
    alerts: Vec<HealthAlert>,
) -> Vec<DiagnosticFinding> {
    let history = |series: SeriesId, range: TimeRange| -> Vec<HistoryPoint> {
        state
            .telemetry
            .query_history(range, series)
            .unwrap_or_default()
    };
    correlate(&alerts, &history, UnixMillis::now())
}

/// Deterministic, rule-based persistence checks (Phase 5) over the
/// already-collected Phase 3 startup/scheduled-task data, enriched with a
/// cached `WinVerifyTrust` signature verdict per flagged executable — see
/// `system_pulse_win::security_posture::check_persistence`. On-demand
/// (Cold-cadence source data, deliberately not folded into a collector's
/// own cadence) so a user opening the Security panel always sees a check
/// run against the freshest available startup/task lists.
#[tauri::command]
pub fn get_persistence_findings(state: State<'_, AppState>) -> Vec<PersistenceFinding> {
    let startup = state
        .telemetry
        .latest_startup()
        .and_then(|s| s.value)
        .unwrap_or_default();
    let tasks = state
        .telemetry
        .latest_scheduled_tasks()
        .and_then(|s| s.value)
        .unwrap_or_default();
    system_pulse_win::security_posture::check_persistence(
        &startup,
        &tasks,
        &system_pulse_win::security_posture::verify_signature,
    )
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
