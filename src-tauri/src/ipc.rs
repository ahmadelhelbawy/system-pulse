//! IPC commands exposed to the frontend.
//!
//! Every command validates its inputs and returns a typed result. There is no
//! generic "run this" or shell-execution command by design: the frontend can
//! only invoke the specific, audited operations declared here.

use system_pulse_core::collector::{probe_capabilities, CollectorCapability};
use system_pulse_core::process::{kill_process as core_kill_process, ProcessIdentity};
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

#[tauri::command]
pub fn get_system_info(state: State<'_, AppState>) -> SystemInfo {
    state.telemetry.system_info()
}

/// Reports what this machine can actually measure, independent of whether
/// telemetry is currently running — "capability probing is a first-class
/// startup phase" (see the master plan's provenance model).
#[tauri::command]
pub fn get_capabilities() -> Vec<CollectorCapability> {
    probe_capabilities()
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
