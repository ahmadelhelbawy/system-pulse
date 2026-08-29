//! Settings persistence (JSON in the platform config directory).

use std::fs;
use std::path::PathBuf;

use system_pulse_core::Settings;
use tauri::{AppHandle, Manager};

fn config_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("settings.json")
}

pub fn load(app: &AppHandle) -> Settings {
    let path = config_path(app);
    match fs::read_to_string(&path) {
        Ok(json) => {
            let mut settings: Settings = serde_json::from_str(&json).unwrap_or_default();
            settings.sanitize();
            settings
        }
        Err(_) => Settings::default(),
    }
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = config_path(app);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}
