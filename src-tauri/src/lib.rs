//! System Pulse — Tauri desktop shell.
//!
//! This crate is intentionally thin: it wires the platform integrations
//! (global hotkey, tray, single instance, autostart, settings persistence) to
//! the telemetry engine in `system-pulse-core` and exposes a small, typed IPC
//! surface to the React frontend.

#![warn(unsafe_code)]

mod error;
mod ipc;
mod settings;
mod windows;

use std::sync::{Arc, Mutex};

use system_pulse_core::sampling::{Backpressure, TelemetryService, TelemetrySink};
use system_pulse_core::{Settings, TelemetrySnapshot};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

/// Shared application state.
pub struct AppState {
    pub settings: Arc<Mutex<Settings>>,
    pub telemetry: TelemetryService,
}

/// Bridges the telemetry engine to a Tauri IPC event.
struct TauriSink {
    app: AppHandle,
}

impl TelemetrySink for TauriSink {
    fn try_emit(&self, snapshot: TelemetrySnapshot) -> Result<(), Backpressure> {
        // Best-effort; the frontend may be hidden/closed mid-frame. Emitting
        // is cheap enough not to need its own backpressure signal here — the
        // mailbox this is drained from already coalesces upstream.
        let _ = self.app.emit("telemetry", &snapshot);
        Ok(())
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch focuses the already-running instance.
            windows::show_window(app);
        }))
        .setup(|app| {
            windows::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(windows::on_window_event)
        .invoke_handler(tauri::generate_handler![
            ipc::get_settings,
            ipc::update_settings,
            ipc::set_visibility,
            ipc::kill_process,
            ipc::is_elevated,
            ipc::request_elevation,
            ipc::get_system_info,
            ipc::get_capabilities,
            ipc::get_connections,
            ipc::get_hardware_info,
            ipc::query_history,
            ipc::get_services,
            ipc::get_drivers,
            ipc::get_startup,
            ipc::get_installed_software,
            ipc::get_scheduled_tasks,
            ipc::get_storage_health,
            ipc::get_sensor_bridge,
            ipc::get_event_log,
            ipc::get_security_posture,
            ipc::get_diagnostics,
            ipc::get_persistence_findings,
            ipc::quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building System Pulse")
        .run(|app_handle, event| {
            // 1.0 had no shutdown path at all: the telemetry thread's
            // `JoinHandle` was discarded and its loop had no stop flag, so
            // nothing here could ever have joined it anyway. Now that it
            // does, stop it cleanly on exit rather than leaving threads
            // running past the point the app handle is torn down.
            if let RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.telemetry.stop();
                }
            }
        });
}
