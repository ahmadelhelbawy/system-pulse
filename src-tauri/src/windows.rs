//! Windows integration: global hotkey, system tray, show/hide toggle,
//! single-instance focus, autostart (HKCU Run key), and elevation detection.

use std::sync::Arc;

use system_pulse_core::sampling::TelemetryService;
use system_pulse_core::Hotkey;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Window, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::{AppState, TauriSink};

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let settings = crate::settings::load(app);

    let telemetry = TelemetryService::new(Arc::new(TauriSink { app: app.clone() }));
    telemetry.spawn();

    app.manage(AppState {
        settings: Arc::new(std::sync::Mutex::new(settings.clone())),
        telemetry,
    });

    let hotkey = settings
        .parsed_hotkey()
        .unwrap_or_else(|_| Hotkey::default());
    if let Err(e) = register_hotkey(app, &hotkey) {
        log::error!("failed to register hotkey {hotkey:?}: {e}");
    }
    build_tray(app)?;

    if settings.launch_at_startup {
        if let Err(e) = set_autostart(true) {
            log::error!("failed to sync autostart: {e}");
        }
    }

    // `--hidden` is appended to the autostart registry value so the app can
    // start quietly to the tray at login.
    if std::env::args().any(|a| a == "--hidden") {
        hide_window(app);
    } else {
        show_window(app);
    }
    Ok(())
}

/// Register (or replace) the single global hotkey.
pub fn register_hotkey(app: &AppHandle, hotkey: &Hotkey) -> Result<(), String> {
    let shortcut = hotkey.to_shortcut();
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    app.global_shortcut()
        .on_shortcut(shortcut.as_str(), move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || toggle_window(&handle));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        let hide = window
            .state::<AppState>()
            .settings
            .lock()
            .map(|s| s.hide_to_tray_on_close)
            .unwrap_or(true);
        if hide {
            api.prevent_close();
            hide_window(window.app_handle());
        }
    }
}

pub fn toggle_window(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide_window(app);
    } else {
        show_window(app);
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        set_telemetry_visible(app, true);
    }
}

pub fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
        set_telemetry_visible(app, false);
    }
}

fn set_telemetry_visible(app: &AppHandle, visible: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.telemetry.set_visible(visible);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show System Pulse", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &hide, &separator, &quit])?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip("System Pulse")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "hide" => hide_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Register/unregister the app in the per-user Run key. Never requires
/// elevation because it writes under HKCU.
#[cfg(target_os = "windows")]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| e.to_string())?;

    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().into_owned();
        let value = format!("\"{exe}\" --hidden");
        run_key
            .set_value("System Pulse", &value)
            .map_err(|e| e.to_string())
    } else {
        let _ = run_key.delete_value("System Pulse");
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    // Autostart is a Windows-only feature in v1.
    Ok(())
}

/// Whether the process holds an elevated token.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)] // Win32 token queries; handles are validated and closed.
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE(std::ptr::null_mut());
    // SAFETY: writes a valid handle into `token`; checked and closed below.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened.is_err() {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
    // SAFETY: `token` is a valid handle; buffer size matches TOKEN_ELEVATION.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
            size,
            &mut size,
        )
    };
    // SAFETY: closing the handle opened above.
    unsafe {
        let _ = CloseHandle(token);
    }
    result.is_ok() && elevation.TokenIsElevated != 0
}

#[cfg(not(target_os = "windows"))]
pub fn is_elevated() -> bool {
    false
}
