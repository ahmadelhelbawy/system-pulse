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

use system_pulse_core::sampling::{TelemetryService, TelemetrySink};
use system_pulse_core::{Settings, TelemetrySnapshot};
use tauri::{AppHandle, Emitter};

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
    fn emit(&self, snapshot: TelemetrySnapshot) {
        // Best-effort; the frontend may be hidden/closed mid-frame.
        let _ = self.app.emit("telemetry", &snapshot);
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
            ipc::get_system_info,
            ipc::quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while building System Pulse")
        .run(|_app_handle, _event| {
            // Run loop is intentionally minimal; all lifecycle logic lives in
            // `windows` and the telemetry engine.
        });
}
