//! Windows integration: global hotkey, system tray, show/hide toggle,
//! single-instance focus, autostart (HKCU Run key), elevation detection,
//! and elevation on request (Phase 4).

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
    // `app_data_dir()` (not `app_config_dir()`, used for settings.json) —
    // this is a continuously-growing data file, not user configuration.
    // `None` (no history) rather than failing setup if the data dir can't
    // be resolved: telemetry must keep working live either way, matching
    // `Scheduler::spawn`'s own "history is diagnostic evidence, not
    // load-bearing" stance on a failed-to-open database.
    let history_db_path = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join("history.sqlite3"))
        .ok();
    telemetry.spawn(system_pulse_win::all_collectors(), history_db_path);

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

    // `--elevated` is appended by `request_elevation`'s own relaunch
    // (Phase 4) purely for diagnostics — the app never trusts its own
    // argv for elevation *state*, which always comes from the real token
    // check (`is_elevated`) regardless of how the process was started.
    if std::env::args().any(|a| a == "--elevated") {
        log::info!(
            "started as an elevated relaunch (is_elevated={})",
            is_elevated()
        );
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

/// Relaunches this executable elevated (UAC `"runas"`), then exits this
/// (unelevated) process so the relaunch can become the sole running
/// instance — reconciling with `tauri-plugin-single-instance`, whose
/// existing-instance callback would otherwise make the elevated relaunch
/// exit itself immediately on the (still-running) unelevated original
/// holding the single-instance lock (see the master plan's Phase 4
/// sequence note on exactly this problem).
///
/// **Called only in direct response to an explicit user action** (a
/// Settings toggle or an in-place "this needs elevation" prompt) — there
/// is no automatic or implicit elevation anywhere in this app, and this
/// is the *only* elevation mechanism: the master plan has no helper
/// service and no per-call escalation, so a privileged read (SMART data,
/// full driver enumeration, some Task Scheduler folders) can only ever
/// come from the whole process's token being elevated. A user who wants
/// none of that stays on the unelevated path indefinitely; nothing here
/// forces or hints otherwise.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)] // ShellExecuteExW; all buffers are validated and outlive the call.
pub fn request_elevation(app: &AppHandle) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if is_elevated() {
        return Ok(()); // already elevated; nothing to do
    }

    let exe =
        std::env::current_exe().map_err(|e| format!("could not resolve own exe path: {e}"))?;
    let exe_wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb_wide: Vec<u16> = "runas\0".encode_utf16().collect();
    let params_wide: Vec<u16> = "--elevated\0".encode_utf16().collect();

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb_wide.as_ptr()),
        lpFile: PCWSTR(exe_wide.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    // SAFETY: `info` is fully initialized with the correct `cbSize` and
    // null-terminated UTF-16 buffers that all outlive this call.
    let result = unsafe { ShellExecuteExW(&mut info) };
    match result {
        Ok(()) => {
            // `SEE_MASK_NOCLOSEPROCESS` handed us a process handle we
            // don't need to track further — close it, not the child
            // itself (that has no effect on an already-launched process).
            if !info.hProcess.is_invalid() {
                // SAFETY: `info.hProcess` was populated by the successful
                // call above.
                unsafe {
                    let _ = CloseHandle(info.hProcess);
                }
            }
            // Release the single-instance lock so the elevated relaunch
            // (once the user approves the UAC prompt, which can take any
            // amount of time) can acquire it as the new sole instance.
            app.exit(0);
            Ok(())
        }
        // `HRESULT_FROM_WIN32(ERROR_CANCELLED)` — the user dismissed the
        // UAC prompt. Distinguished from other failures so the frontend
        // can show "you declined elevation" rather than a generic error;
        // deliberately does *not* exit the current (still perfectly
        // usable, unelevated) instance in this case.
        Err(e)
            if e.code().0
                == ((ERROR_CANCELLED.0 & 0x0000_ffff) | (7 << 16) | 0x8000_0000) as i32 =>
        {
            Err("elevation was cancelled".to_string())
        }
        Err(e) => Err(format!("failed to relaunch elevated: {e}")),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn request_elevation(_app: &AppHandle) -> Result<(), String> {
    Err("elevation is only supported on Windows".to_string())
}
