//! Loaded kernel driver enumeration (Phase 3): `EnumDeviceDrivers` +
//! `GetDeviceDriverBaseNameW`/`GetDeviceDriverFileNameW` (psapi), no COM.
//!
//! **Deliberate deviation from the plan's literal wording.** The plan
//! names SetupAPI as the source for a driver's human-readable description
//! and version. Correlating a *loaded kernel module* (what
//! `EnumDeviceDrivers` reports) to a *device instance* in SetupAPI's
//! device tree is not a direct lookup — it requires walking every device
//! node (`SetupDiGetClassDevs(DIGCF_ALLCLASSES)`), reading each one's
//! driver service name, and matching that back to the module list, with
//! no guaranteed 1:1 correspondence (one driver file can back several
//! device instances, and some loaded modules — the kernel itself, HAL —
//! have no device node at all). Every `.sys` file already carries a
//! standard Win32 version resource (`FileDescription`/`FileVersion`),
//! readable with the well-documented `GetFileVersionInfoW`/
//! `VerQueryValueW` pair used for any PE file. That gives the same
//! practical result — a human-readable name and a version string — for
//! every loaded driver uniformly, without the device-tree correlation
//! problem or its edge cases.
//!
//! **Elevation finding, from running this for real.** The master plan's
//! capability matrix lists this as unprivileged. Verified false on a real
//! Windows 11 host: an unelevated process gets a successful
//! `EnumDeviceDrivers` call back reporting **zero** drivers — a
//! kernel-ASLR-related mitigation (hiding kernel addresses from
//! standard-user processes), not a documented error return. Since a real
//! Windows host always has dozens of drivers loaded, `collect()` treats
//! an empty (not merely short) result as the signature of this
//! restriction and reports `NeedsElevation` rather than a fabricated
//! "zero drivers installed."

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::DriverSnapshot;

const CADENCE: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawDriver {
    pub base_address: u64,
    pub base_name: String,
    /// A regular filesystem path (already translated from the `\SystemRoot\`
    /// -style device path psapi reports), when resolvable.
    pub file_path: Option<String>,
}

/// Pure, testable without Windows: turns `\SystemRoot\...` into a real
/// path using the given Windows directory (`%SystemRoot%`/`%windir%` in
/// production; injected here so the substitution logic doesn't need a
/// live environment variable to test).
pub fn resolve_system_root_path(raw_path: &str, windows_dir: &str) -> String {
    const PREFIX: &str = r"\SystemRoot\";
    if let Some(rest) = raw_path.strip_prefix(PREFIX) {
        format!("{}\\{}", windows_dir.trim_end_matches('\\'), rest)
    } else {
        raw_path.to_string()
    }
}

#[cfg(target_os = "windows")]
mod raw {
    use super::{resolve_system_root_path, RawDriver};
    use std::ffi::c_void;
    use windows::Win32::System::ProcessStatus::{
        EnumDeviceDrivers, GetDeviceDriverBaseNameW, GetDeviceDriverFileNameW,
    };

    fn windows_dir() -> String {
        std::env::var("SystemRoot")
            .or_else(|_| std::env::var("windir"))
            .unwrap_or_else(|_| r"C:\Windows".to_string())
    }

    /// `None` on any enumeration failure — never a partial/fabricated list.
    ///
    /// Sizing here does *not* use the "probe with a null buffer and
    /// `cb = 0`" idiom other Windows APIs support: verified by running
    /// this collector for real, `EnumDeviceDrivers` reports `0` needed
    /// bytes for a null/zero-length buffer instead of the true required
    /// size, which silently produced an empty driver list on every real
    /// host. Growing a real buffer until it's confirmed large enough is
    /// the idiom this specific API actually needs.
    pub fn read() -> Option<Vec<RawDriver>> {
        let mut capacity: usize = 1024;
        let addrs: Vec<*mut c_void> = loop {
            let mut buf: Vec<*mut c_void> = vec![std::ptr::null_mut(); capacity];
            let mut needed = 0u32;
            // SAFETY: `buf` has room for `capacity` pointers, matching the
            // byte count passed as `cb`; `needed` is a valid out-pointer.
            #[allow(unsafe_code)]
            unsafe {
                EnumDeviceDrivers(
                    buf.as_mut_ptr(),
                    (capacity * std::mem::size_of::<*mut c_void>()) as u32,
                    &mut needed,
                )
            }
            .ok()?;
            let returned_count = needed as usize / std::mem::size_of::<*mut c_void>();
            if returned_count <= capacity {
                buf.truncate(returned_count);
                break buf;
            }
            // The buffer was too small; `needed` reports the true size
            // required — grow to fit and try again.
            capacity = returned_count;
        };
        let win_dir = windows_dir();

        let mut out = Vec::with_capacity(addrs.len());
        for addr in addrs {
            if addr.is_null() {
                continue;
            }
            let mut name_buf = [0u16; 260];
            // SAFETY: `name_buf` is a valid, sufficiently large UTF-16
            // buffer; `addr` came from a successful `EnumDeviceDrivers`
            // call above.
            #[allow(unsafe_code)]
            let name_len = unsafe { GetDeviceDriverBaseNameW(addr, &mut name_buf) };
            let base_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);

            let mut path_buf = [0u16; 260];
            // SAFETY: same as above.
            #[allow(unsafe_code)]
            let path_len = unsafe { GetDeviceDriverFileNameW(addr, &mut path_buf) };
            let file_path = if path_len > 0 {
                let raw_path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
                Some(resolve_system_root_path(&raw_path, &win_dir))
            } else {
                None
            };

            out.push(RawDriver {
                base_address: addr as u64,
                base_name,
                file_path,
            });
        }
        Some(out)
    }

    /// Reads `FileDescription`/`FileVersion` from a PE file's version
    /// resource — the same standard resource every signed driver ships,
    /// used here instead of SetupAPI device-tree correlation (see the
    /// module doc). `None` fields, never fabricated, when the file has no
    /// version resource or a field is absent from it.
    pub fn read_version_info(path: &str) -> (Option<String>, Option<String>) {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW};

        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a valid null-terminated UTF-16 path; `None`
        // for the reserved handle out-param, per the documented usage.
        #[allow(unsafe_code)]
        let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
        if size == 0 {
            return (None, None);
        }

        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is exactly `size` bytes as reported above.
        #[allow(unsafe_code)]
        let ok = unsafe {
            GetFileVersionInfoW(
                PCWSTR(wide.as_ptr()),
                None,
                size,
                buf.as_mut_ptr() as *mut _,
            )
        };
        if ok.is_err() {
            return (None, None);
        }

        let description = query_string(&buf, "FileDescription");
        let version = query_string(&buf, "FileVersion");
        (description, version)
    }

    /// `VerQueryValueW` against the default (US English, Unicode)
    /// `StringFileInfo` sub-block — the overwhelmingly common case for a
    /// Windows driver; a driver localized only into another code page is
    /// reported without a description/version rather than guessed.
    fn query_string(version_info: &[u8], field: &str) -> Option<String> {
        use windows::Win32::Storage::FileSystem::VerQueryValueW;

        let sub_block: Vec<u16> = format!(r"\StringFileInfo\040904B0\{field}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        // SAFETY: `version_info` is a fully-populated buffer from a
        // successful `GetFileVersionInfoW` call; `sub_block` is a valid
        // null-terminated UTF-16 string; `ptr`/`len` are valid out-params
        // that `VerQueryValueW` documents as pointing *into*
        // `version_info` itself on success (never freed separately).
        #[allow(unsafe_code)]
        let found = unsafe {
            VerQueryValueW(
                version_info.as_ptr() as *const _,
                windows::core::PCWSTR(sub_block.as_ptr()),
                &mut ptr,
                &mut len,
            )
        };
        if !found.as_bool() || ptr.is_null() || len == 0 {
            return None;
        }
        // SAFETY: `ptr`/`len` (in UTF-16 code units) were just validated
        // as pointing into `version_info`'s own buffer by the successful
        // call above.
        #[allow(unsafe_code)]
        let slice = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize) };
        let s = String::from_utf16_lossy(slice);
        let trimmed = s.trim_end_matches('\0').trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::RawDriver;

    pub fn read() -> Option<Vec<RawDriver>> {
        None
    }

    pub fn read_version_info(_path: &str) -> (Option<String>, Option<String>) {
        (None, None)
    }
}

pub struct DriversCollector {
    availability: Availability,
}

impl DriversCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for DriversCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for DriversCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Drivers
    }

    fn cadence(&self) -> Cadence {
        Cadence::Cold(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        // The plan's capability matrix assumed this was unprivileged
        // (matching `EnumDeviceDrivers`'s own docs). Verified false by
        // running this collector for real: on this Windows 11 host, an
        // unelevated process gets a *successful* call back with **zero**
        // drivers — a well-known kernel-ASLR-related mitigation, not a
        // documented return-code distinction, so `collect()` below infers
        // it from the impossible "zero drivers loaded" result rather than
        // from an error. Reported here as `Admin` since that's what a
        // non-empty, meaningful result actually needs in practice.
        Privilege::Admin
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
            return CollectorOutput::Drivers(Sampled::unavailable(
                self.availability.clone(),
                Source::Registry,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            // A real Windows host always has dozens of drivers loaded
            // (ntoskrnl, hal, bus/filesystem drivers, ...) — an empty
            // list is never a genuine reading, only the signature of the
            // unelevated kernel-address restriction described above.
            Some(rows) if rows.is_empty() => {
                Sampled::unavailable(Availability::NeedsElevation, Source::Registry, ctx.wall_now)
            }
            Some(rows) => {
                let snapshots: Vec<DriverSnapshot> = rows
                    .into_iter()
                    .map(|r| {
                        let (description, version) = r
                            .file_path
                            .as_deref()
                            .map(raw::read_version_info)
                            .unwrap_or((None, None));
                        DriverSnapshot {
                            name: r.base_name,
                            description,
                            version,
                            base_address: r.base_address,
                        }
                    })
                    .collect();
                Sampled::ok(snapshots, Source::Registry, ctx.wall_now)
            }
            None => Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::Registry,
                ctx.wall_now,
            ),
        };
        CollectorOutput::Drivers(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_system_root_prefixed_paths() {
        assert_eq!(
            resolve_system_root_path(r"\SystemRoot\System32\drivers\Wdf01000.sys", r"C:\Windows"),
            r"C:\Windows\System32\drivers\Wdf01000.sys"
        );
    }

    #[test]
    fn leaves_already_normal_paths_untouched() {
        assert_eq!(
            resolve_system_root_path(r"C:\Windows\System32\ntoskrnl.exe", r"C:\Windows"),
            r"C:\Windows\System32\ntoskrnl.exe"
        );
    }

    #[test]
    fn handles_a_trailing_slash_on_the_windows_dir() {
        assert_eq!(
            resolve_system_root_path(r"\SystemRoot\System32\drivers\a.sys", r"C:\Windows\"),
            r"C:\Windows\System32\drivers\a.sys"
        );
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = DriversCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = DriversCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::Drivers(_) => {}
            _ => panic!("expected Drivers output"),
        }
    }
}
