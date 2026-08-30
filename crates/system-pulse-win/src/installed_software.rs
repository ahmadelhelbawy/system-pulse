//! Installed-software enumeration (Phase 3): the Uninstall registry keys
//! (HKLM + HKCU, both the native and `WOW6432Node` views), **never**
//! `Win32_Product` (WMI) — enumerating that class silently triggers an
//! MSI reconfiguration of every installed package as a side effect. No
//! COM either way.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, Sampled, Source};
use system_pulse_core::types::InstalledSoftware;

const CADENCE: Duration = Duration::from_secs(3600);

#[cfg(target_os = "windows")]
mod raw {
    use super::InstalledSoftware;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const UNINSTALL_PATHS: &[(winreg::HKEY, &str)] = &[
        (
            HKEY_LOCAL_MACHINE,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ];

    /// One subkey -> one entry, or `None` if it's not a real user-facing
    /// product (no `DisplayName`, or explicitly marked `SystemComponent`)
    /// — filtered the same way Programs & Features itself hides these,
    /// rather than surfacing registry plumbing as if it were software.
    fn read_entry(key: &RegKey) -> Option<InstalledSoftware> {
        let name: String = key.get_value("DisplayName").ok()?;
        if name.trim().is_empty() {
            return None;
        }
        let is_system_component: u32 = key.get_value("SystemComponent").unwrap_or(0);
        if is_system_component == 1 {
            return None;
        }
        Some(InstalledSoftware {
            name,
            version: key.get_value("DisplayVersion").ok(),
            publisher: key.get_value("Publisher").ok(),
            install_date: key.get_value("InstallDate").ok(),
        })
    }

    pub fn read() -> Vec<InstalledSoftware> {
        let mut out = Vec::new();
        for (hive, path) in UNINSTALL_PATHS {
            let Ok(root) = RegKey::predef(*hive).open_subkey(path) else {
                continue;
            };
            for subkey_name in root.enum_keys().filter_map(Result::ok) {
                let Ok(subkey) = root.open_subkey(&subkey_name) else {
                    continue;
                };
                if let Some(entry) = read_entry(&subkey) {
                    out.push(entry);
                }
            }
        }
        out
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::InstalledSoftware;

    pub fn read() -> Vec<InstalledSoftware> {
        Vec::new()
    }
}

pub struct InstalledSoftwareCollector {
    availability: Availability,
}

impl InstalledSoftwareCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for InstalledSoftwareCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for InstalledSoftwareCollector {
    fn id(&self) -> CollectorId {
        CollectorId::InstalledSoftware
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
            return CollectorOutput::InstalledSoftware(Sampled::unavailable(
                self.availability.clone(),
                Source::Registry,
                ctx.wall_now,
            ));
        }
        CollectorOutput::InstalledSoftware(Sampled::ok(raw::read(), Source::Registry, ctx.wall_now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = InstalledSoftwareCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = InstalledSoftwareCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::InstalledSoftware(_) => {}
            _ => panic!("expected InstalledSoftware output"),
        }
    }
}
