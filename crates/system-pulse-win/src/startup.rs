//! Autostart enumeration (Phase 3): Run/RunOnce registry keys (HKCU +
//! HKLM) and Startup-folder shortcuts, cross-referenced against
//! `StartupApproved` for the enabled/disabled state Task Manager's
//! Startup tab shows. No COM.
//!
//! Uses `winreg` (already a proven dependency, via `src-tauri`'s
//! autostart code) rather than hand-rolled `RegEnumValueW` FFI — the
//! registry access here has no requirement for anything `winreg` doesn't
//! already provide safely.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, Sampled, Source};
use system_pulse_core::types::StartupItem;
#[cfg(target_os = "windows")]
use system_pulse_core::types::StartupLocation;

const CADENCE: Duration = Duration::from_secs(3600);

/// True if a `StartupApproved` binary value marks the entry disabled —
/// the reverse-engineered but stable convention Explorer/Task Manager
/// both rely on: byte 0 of the value is `0x02` or `0x03` when the user
/// has disabled the entry from Task Manager's Startup tab; any other
/// value, or no `StartupApproved` entry at all (never configured through
/// that UI), means enabled. Pure and unit-testable without the registry.
pub fn is_disabled_by_approved(value: Option<&[u8]>) -> bool {
    matches!(value.and_then(|b| b.first()), Some(0x02) | Some(0x03))
}

#[cfg(target_os = "windows")]
mod raw {
    use super::{is_disabled_by_approved, StartupItem, StartupLocation};
    use std::path::Path;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_ONCE_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    const APPROVED_RUN_PATH: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn read_run_values(
        hive: winreg::HKEY,
        path: &str,
        location: StartupLocation,
        approved: Option<&RegKey>,
    ) -> Vec<StartupItem> {
        let Ok(key) = RegKey::predef(hive).open_subkey(path) else {
            return Vec::new();
        };
        key.enum_values()
            .filter_map(Result::ok)
            .map(|(name, value)| {
                let disabled = approved
                    .and_then(|a| a.get_raw_value(&name).ok())
                    .map(|v| is_disabled_by_approved(Some(&v.bytes)))
                    .unwrap_or(false);
                StartupItem {
                    command: value.to_string(),
                    name,
                    location,
                    enabled: !disabled,
                }
            })
            .collect()
    }

    fn read_startup_folder(dir: &Path, location: StartupLocation) -> Vec<StartupItem> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| {
                let path = e.path();
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                StartupItem {
                    name,
                    // Shortcut targets aren't resolved (would need
                    // `IShellLink`, COM) — the file's own path is shown
                    // instead, which is still enough to identify the
                    // entry and matches Task Manager's own display for
                    // items it can't further decompose.
                    command: path.to_string_lossy().into_owned(),
                    location,
                    enabled: true,
                }
            })
            .collect()
    }

    pub fn read() -> Vec<StartupItem> {
        let approved_hkcu = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(APPROVED_RUN_PATH)
            .ok();
        // HKLM's StartupApproved\Run lives in the same relative path under
        // HKLM; a per-machine Run entry's approval state is tracked there.
        let approved_hklm = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(APPROVED_RUN_PATH)
            .ok();

        let mut out = Vec::new();
        out.extend(read_run_values(
            HKEY_CURRENT_USER,
            RUN_PATH,
            StartupLocation::HkcuRun,
            approved_hkcu.as_ref(),
        ));
        out.extend(read_run_values(
            HKEY_LOCAL_MACHINE,
            RUN_PATH,
            StartupLocation::HklmRun,
            approved_hklm.as_ref(),
        ));
        // RunOnce entries have no `StartupApproved` concept (they aren't
        // shown/toggleable in Task Manager's Startup tab) — always enabled.
        out.extend(read_run_values(
            HKEY_CURRENT_USER,
            RUN_ONCE_PATH,
            StartupLocation::HkcuRunOnce,
            None,
        ));
        out.extend(read_run_values(
            HKEY_LOCAL_MACHINE,
            RUN_ONCE_PATH,
            StartupLocation::HklmRunOnce,
            None,
        ));

        if let Ok(appdata) = std::env::var("APPDATA") {
            out.extend(read_startup_folder(
                Path::new(&appdata)
                    .join(r"Microsoft\Windows\Start Menu\Programs\Startup")
                    .as_path(),
                StartupLocation::UserStartupFolder,
            ));
        }
        if let Ok(program_data) = std::env::var("ProgramData") {
            out.extend(read_startup_folder(
                Path::new(&program_data)
                    .join(r"Microsoft\Windows\Start Menu\Programs\Startup")
                    .as_path(),
                StartupLocation::CommonStartupFolder,
            ));
        }
        out
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::StartupItem;

    pub fn read() -> Vec<StartupItem> {
        Vec::new()
    }
}

pub struct StartupCollector {
    availability: Availability,
}

impl StartupCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for StartupCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for StartupCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Startup
    }

    fn cadence(&self) -> Cadence {
        Cadence::Cold(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        #[cfg(target_os = "windows")]
        {
            self.availability = Availability::Ok;
        }
        #[cfg(not(target_os = "windows"))]
        {
            self.availability = Availability::unsupported(
                system_pulse_core::model::UnsupportedReason::NotImplementedOnPlatform,
            );
        }
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            return CollectorOutput::Startup(Sampled::unavailable(
                self.availability.clone(),
                Source::Registry,
                ctx.wall_now,
            ));
        }
        // Enumeration itself is infallible here (missing keys/folders
        // just contribute no entries), unlike the SCM/driver collectors
        // where the whole underlying API call can fail outright.
        CollectorOutput::Startup(Sampled::ok(raw::read(), Source::Registry, ctx.wall_now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_byte_0x02_and_0x03_mean_disabled() {
        assert!(is_disabled_by_approved(Some(&[0x02, 0, 0, 0])));
        assert!(is_disabled_by_approved(Some(&[0x03, 0, 0, 0])));
    }

    #[test]
    fn any_other_byte_means_enabled() {
        assert!(!is_disabled_by_approved(Some(&[0x06, 0, 0, 0])));
        assert!(!is_disabled_by_approved(Some(&[0x00])));
    }

    #[test]
    fn missing_approved_entry_means_enabled() {
        assert!(!is_disabled_by_approved(None));
    }

    #[test]
    fn empty_value_means_enabled_not_a_panic() {
        assert!(!is_disabled_by_approved(Some(&[])));
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = StartupCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = StartupCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::Startup(_) => {}
            _ => panic!("expected Startup output"),
        }
    }
}
