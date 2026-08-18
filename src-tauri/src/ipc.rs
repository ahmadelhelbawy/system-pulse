//! IPC commands exposed to the frontend.
//!
//! Every command validates its inputs and returns a typed result. There is no
//! generic "run this" or shell-execution command by design: the frontend can
//! only invoke the specific, audited operations declared here.

use system_pulse_core::process::kill_process as core_kill_process;
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
            .map_err(|e| AppError::InvalidSettings(e.to_string()))?;
        crate::windows::register_hotkey(&app, &hotkey).map_err(AppError::Message)?;
    }

    state.telemetry.set_interval_ms(next.refresh_interval_ms);
    crate::windows::set_autostart(next.launch_at_startup).map_err(AppError::Message)?;
    crate::settings::save(&app, &next).map_err(AppError::Message)?;

    *state.settings.lock().unwrap() = next.clone();
    Ok(next)
}

#[tauri::command]
pub fn set_visibility(state: State<'_, AppState>, visible: bool) {
    state.telemetry.set_visible(visible);
}

#[tauri::command]
pub fn kill_process(pid: u32) -> Result<(), AppError> {
    core_kill_process(pid).map_err(AppError::from)
}

#[tauri::command]
pub fn is_elevated() -> bool {
    crate::windows::is_elevated()
}

#[tauri::command]
pub fn get_system_info(state: State<'_, AppState>) -> SystemInfo {
    state.telemetry.system_info()
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
