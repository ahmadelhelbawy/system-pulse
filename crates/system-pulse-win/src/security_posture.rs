//! Windows Security Center, firewall, and Secure Boot state (Phase 5) — all
//! three are readable by a standard user (see the master plan's capability
//! matrix, A1), so this collector never needs elevation.
//!
//! Every field of [`SecurityPostureSnapshot`] is independently `Option`,
//! same discipline as `StorageHealthSnapshot` (Phase 4): a query that fails
//! or doesn't apply on this machine (no UEFI, WSC provider not monitored)
//! reports `None` for exactly that field, never a fabricated "protected" or
//! "off."
//!
//! **Defensive persistence checks** (autostart/scheduled-task entries
//! pointing somewhere suspicious, an unsigned binary) are deliberately
//! *not* part of this collector. They're computed on demand from the
//! already-collected Phase 3 startup/scheduled-task data — see the
//! `get_persistence_findings` IPC command in `src-tauri` and
//! [`check_persistence`] below — rather than adding a second collector
//! that would just re-read data this app already has.
//!
//! **`Source` reuse.** There is no dedicated `Source::SecurityCenter`
//! variant in the closed provenance enum; `Source::Wmi` is reused here for
//! the same reason `scheduled_tasks.rs` reuses it for Task Scheduler COM —
//! "a Windows system API, not one of the more specific buckets" — rather
//! than growing the enum for one more collector.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, Sampled, Source};
use system_pulse_core::types::{
    PersistenceFinding, ScheduledTaskSnapshot, SecurityPostureSnapshot, Severity, StartupItem,
};

/// Security posture changes rarely enough that a half-hour cadence is
/// plenty fresh while keeping this well off the hot/warm path — a
/// documented judgment call, same style as storage health's hourly
/// cadence (Phase 4).
const CADENCE: Duration = Duration::from_secs(1800);

/// `WSC_SECURITY_PROVIDER_HEALTH`'s raw value -> the WSC API's own word for
/// it, passed through rather than reinterpreted (see the module doc on
/// `SecurityProviderStatus`).
pub fn map_provider_health(raw: i32) -> &'static str {
    match raw {
        0 => "good",
        1 => "notMonitored",
        2 => "poor",
        3 => "snooze",
        _ => "unknown",
    }
}

#[derive(Default)]
pub struct SecurityPostureCollector;

impl Collector for SecurityPostureCollector {
    fn id(&self) -> CollectorId {
        CollectorId::SecurityPosture
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
            Availability::Ok
        }
        #[cfg(not(target_os = "windows"))]
        {
            Availability::unsupported(
                system_pulse_core::model::UnsupportedReason::NotImplementedOnPlatform,
            )
        }
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        let snapshot = SecurityPostureSnapshot {
            firewall: raw::read_firewall(),
            antivirus: raw::read_wsc(),
            secure_boot_enabled: raw::read_secure_boot(),
        };
        CollectorOutput::SecurityPosture(Sampled::ok(snapshot, Source::Wmi, ctx.wall_now))
    }
}

/// Deterministic, rule-based persistence checks (Phase 5) over
/// already-collected Phase 3 data — see the module doc for why this isn't
/// its own collector. `verify_signature` is injected so this stays a pure,
/// fast function testable without touching `WinVerifyTrust`; the real
/// caller (`get_persistence_findings` in `src-tauri`) passes
/// [`verify_signature`] itself.
pub fn check_persistence(
    startup: &[StartupItem],
    tasks: &[ScheduledTaskSnapshot],
    verify_signature: &dyn Fn(&str) -> Option<bool>,
) -> Vec<PersistenceFinding> {
    let mut findings = Vec::new();

    for item in startup {
        if let Some(path) = extract_executable_path(&item.command) {
            check_one(
                &mut findings,
                &format!("startup:{}:{}", item.location as u8 as u32, item.name),
                &item.name,
                &path,
                verify_signature,
            );
        }
    }
    for task in tasks {
        // Not every scheduled task's `path` is itself an executable path
        // (it's the task's own registration path, e.g.
        // `\Microsoft\Windows\...`) — only entries whose enabled action
        // clearly names an executable are checked; this app doesn't parse
        // Task Scheduler action XML here, so tasks are covered by this
        // check only when a future collector surfaces their target
        // executable. Present now so the function's shape doesn't need to
        // change when that data becomes available.
        let _ = task;
    }

    findings
}

/// Pulls a plausible executable path out of a startup command line —
/// strips a leading quoted path or takes the first whitespace-delimited
/// token, since Run-key values are frequently `"C:\...\app.exe" --flag`.
/// Returns `None` (never a guess) when nothing that looks like a real path
/// with an extension is found.
fn extract_executable_path(command: &str) -> Option<String> {
    let trimmed = command.trim();
    let candidate = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        trimmed.split_whitespace().next()?
    };
    if candidate.contains('.') && candidate.len() > 3 {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn check_one(
    findings: &mut Vec<PersistenceFinding>,
    id: &str,
    name: &str,
    path: &str,
    verify_signature: &dyn Fn(&str) -> Option<bool>,
) {
    let exists = std::path::Path::new(path).is_file();
    if !exists {
        findings.push(PersistenceFinding {
            id: format!("{id}:missing"),
            severity: Severity::Warning,
            title: format!("{name} points to a missing file"),
            detail: format!("{path} no longer exists on disk"),
            path: Some(path.to_string()),
            signed: None,
        });
        // No point checking the signature of a file that isn't there.
        return;
    }

    let suspicious_location = ["\\temp\\", "\\appdata\\local\\temp\\"]
        .iter()
        .any(|needle| path.to_lowercase().contains(needle));
    let signed = verify_signature(path);

    if suspicious_location || signed == Some(false) {
        let mut reasons = Vec::new();
        if suspicious_location {
            reasons.push("runs from a temporary directory".to_string());
        }
        if signed == Some(false) {
            reasons.push("is not signed".to_string());
        }
        findings.push(PersistenceFinding {
            id: format!("{id}:suspicious"),
            severity: Severity::Warning,
            title: format!("{name} looks worth reviewing"),
            detail: format!("{path} {}", reasons.join(" and ")),
            path: Some(path.to_string()),
            signed,
        });
    }
}

/// `WinVerifyTrust` against a cached-by-`(path, len, mtime)` verdict
/// (Phase 5) — revocation checking is disabled (`WTD_REVOKE_NONE`), which
/// keeps this a local, fast, offline call rather than one that can block on
/// network revocation lookups; that's also what removes most of the need
/// for a separate hard timeout, since the call has no network leg to hang
/// on. `None` whenever the verdict can't be determined (file missing, API
/// error) — never coerced to `Some(false)`.
#[cfg(target_os = "windows")]
pub fn verify_signature(path: &str) -> Option<bool> {
    raw::verify_signature_cached(path)
}

#[cfg(not(target_os = "windows"))]
pub fn verify_signature(_path: &str) -> Option<bool> {
    None
}

#[cfg(target_os = "windows")]
mod raw {
    use super::map_provider_health;
    use std::sync::Mutex;
    use system_pulse_core::types::{FirewallProfileState, FirewallStatus, SecurityProviderStatus};
    use windows::core::{HRESULT, PCSTR, PCWSTR};
    use windows::Win32::NetworkManagement::WindowsFirewall::{
        INetFwPolicy2, NetFwPolicy2, NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE,
        NET_FW_PROFILE2_PUBLIC, NET_FW_PROFILE_TYPE2,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::System::SecurityCenter::{
        WSC_SECURITY_PROVIDER_ANTIVIRUS, WSC_SECURITY_PROVIDER_AUTOUPDATE_SETTINGS,
        WSC_SECURITY_PROVIDER_HEALTH,
    };

    type WscGetSecurityProviderHealthFn =
        unsafe extern "system" fn(u32, *mut WSC_SECURITY_PROVIDER_HEALTH) -> HRESULT;

    /// `wscapi.dll` (Windows Security Center) ships only on client Windows —
    /// it's documented as unsupported on Server SKUs, and genuinely absent
    /// there (confirmed by a real CI failure: linking this symbol statically
    /// made the *entire test binary* fail to load with
    /// `STATUS_DLL_NOT_FOUND` on GitHub's `windows-latest` runner, before any
    /// test even ran). Resolved dynamically instead, same reasoning as the
    /// NVML adapter (`gpu/nvidia.rs`) not requiring `nvml.dll` to exist at
    /// load time — its absence must degrade to "no data", never crash the
    /// process.
    fn wsc_get_security_provider_health() -> Option<WscGetSecurityProviderHealthFn> {
        use std::sync::OnceLock;
        static RAW: OnceLock<Option<unsafe extern "system" fn() -> isize>> = OnceLock::new();
        let raw = *RAW.get_or_init(|| {
            let wide: Vec<u16> = "wscapi.dll\0".encode_utf16().collect();
            // SAFETY: `wide` is a valid null-terminated UTF-16 string naming
            // a well-known system DLL; failure (the DLL isn't present on
            // this host) is handled via `.ok()?`, not assumed away.
            #[allow(unsafe_code)]
            let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }.ok()?;
            // SAFETY: `module` was just loaded successfully above; the name
            // is the documented exported symbol.
            #[allow(unsafe_code)]
            unsafe {
                GetProcAddress(
                    module,
                    PCSTR(c"WscGetSecurityProviderHealth".as_ptr().cast()),
                )
            }
        });
        // SAFETY: `raw`, when present, was returned by `GetProcAddress` for
        // exactly this symbol name and is reinterpreted as its documented
        // C signature.
        #[allow(unsafe_code)]
        raw.map(|f| unsafe { std::mem::transmute::<_, WscGetSecurityProviderHealthFn>(f) })
    }

    pub fn read_wsc() -> Vec<SecurityProviderStatus> {
        let Some(func) = wsc_get_security_provider_health() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (provider, kind) in [
            (WSC_SECURITY_PROVIDER_ANTIVIRUS, "antivirus"),
            (WSC_SECURITY_PROVIDER_AUTOUPDATE_SETTINGS, "autoUpdate"),
        ] {
            let mut health = WSC_SECURITY_PROVIDER_HEALTH(0);
            // SAFETY: `health` is a correctly-typed out-parameter for this
            // flat, no-allocation API call; `func` was resolved above via
            // `GetProcAddress` for this exact symbol and signature.
            #[allow(unsafe_code)]
            let ok = unsafe { func(provider.0 as u32, &mut health) };
            if ok.is_ok() {
                out.push(SecurityProviderStatus {
                    kind: kind.to_string(),
                    health: map_provider_health(health.0).to_string(),
                });
            }
        }
        out
    }

    fn profile_state(
        policy: &INetFwPolicy2,
        profile: NET_FW_PROFILE_TYPE2,
    ) -> FirewallProfileState {
        // SAFETY: `policy` is a valid, live COM interface pointer for the
        // duration of this call.
        #[allow(unsafe_code)]
        match unsafe { policy.get_FirewallEnabled(profile) } {
            Ok(enabled) => {
                if enabled.as_bool() {
                    FirewallProfileState::On
                } else {
                    FirewallProfileState::Off
                }
            }
            Err(_) => FirewallProfileState::Unknown,
        }
    }

    /// Self-contained per call, same pattern as `scheduled_tasks.rs` and
    /// `sensor_bridge.rs` (see `com_spike`'s finding): init, use, and tear
    /// down COM entirely within this one function, no process-wide
    /// `CoInitializeSecurity` call.
    pub fn read_firewall() -> Option<FirewallStatus> {
        // SAFETY: MTA init on whatever thread runs this collector; paired
        // with `CoUninitialize` below regardless of outcome.
        #[allow(unsafe_code)]
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if init.is_err() {
            return None;
        }
        let result = (|| {
            // SAFETY: `NetFwPolicy2` is the well-known CLSID for the
            // firewall policy coclass; `INetFwPolicy2` is requested
            // directly via its IID.
            #[allow(unsafe_code)]
            let policy: INetFwPolicy2 =
                unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER) }.ok()?;
            Some(FirewallStatus {
                domain: profile_state(&policy, NET_FW_PROFILE2_DOMAIN),
                private: profile_state(&policy, NET_FW_PROFILE2_PRIVATE),
                public: profile_state(&policy, NET_FW_PROFILE2_PUBLIC),
            })
        })();
        // SAFETY: pairs the successful `CoInitializeEx` above; called
        // exactly once regardless of which branch above returned.
        #[allow(unsafe_code)]
        unsafe {
            CoUninitialize();
        }
        result
    }

    pub fn read_secure_boot() -> Option<bool> {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        use winreg::RegKey;

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm
            .open_subkey(r"SYSTEM\CurrentControlSet\Control\SecureBoot\State")
            .ok()?;
        let value: u32 = key.get_value("UEFISecureBootEnabled").ok()?;
        Some(value != 0)
    }

    struct CacheEntry {
        len: u64,
        mtime: u64,
        verdict: Option<bool>,
    }

    static SIGNATURE_CACHE: Mutex<Vec<(String, CacheEntry)>> = Mutex::new(Vec::new());

    fn file_fingerprint(path: &str) -> Option<(u64, u64)> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some((meta.len(), mtime))
    }

    /// Cached by `(path, len, mtime)` — a changed file (different size or
    /// modification time) invalidates the cached verdict rather than
    /// trusting stale data, per the master plan's caching requirement.
    pub fn verify_signature_cached(path: &str) -> Option<bool> {
        let (len, mtime) = file_fingerprint(path)?;
        {
            let cache = SIGNATURE_CACHE.lock().unwrap();
            if let Some((_, entry)) = cache.iter().find(|(p, _)| p == path) {
                if entry.len == len && entry.mtime == mtime {
                    return entry.verdict;
                }
            }
        }
        let verdict = verify_signature_uncached(path);
        let mut cache = SIGNATURE_CACHE.lock().unwrap();
        cache.retain(|(p, _)| p != path);
        cache.push((
            path.to_string(),
            CacheEntry {
                len,
                mtime,
                verdict,
            },
        ));
        verdict
    }

    fn verify_signature_uncached(path: &str) -> Option<bool> {
        use windows::core::GUID;
        use windows::Win32::Foundation::{HANDLE, HWND};
        use windows::Win32::Security::WinTrust::{
            WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
            WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
            WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        };

        let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: windows::core::PCWSTR(path_w.as_ptr()),
            hFile: HANDLE::default(),
            pgKnownSubject: std::ptr::null_mut(),
        };

        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            pPolicyCallbackData: std::ptr::null_mut(),
            pSIPClientData: std::ptr::null_mut(),
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            hWVTStateData: HANDLE::default(),
            pwszURLReference: windows::core::PWSTR::null(),
            // Never touch the network for revocation — local-only, fast.
            dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
            dwUIContext: windows::Win32::Security::WinTrust::WINTRUST_DATA_UICONTEXT(0),
            pSignatureSettings: std::ptr::null_mut(),
        };

        let mut action_guid: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: `data`/`file_info`/`path_w` all outlive this call;
        // `action_guid` is the documented well-known verification action.
        #[allow(unsafe_code)]
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action_guid,
                &mut data as *mut _ as *mut core::ffi::c_void,
            )
        };
        let verdict = status == 0; // S_OK

        // Always release WinVerifyTrust's internal state, even on failure.
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: same arguments as the verify call above, now requesting
        // state cleanup as WinVerifyTrust's contract requires.
        #[allow(unsafe_code)]
        unsafe {
            let _ = WinVerifyTrust(
                HWND::default(),
                &mut action_guid,
                &mut data as *mut _ as *mut core::ffi::c_void,
            );
        }

        Some(verdict)
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use system_pulse_core::types::{FirewallStatus, SecurityProviderStatus};

    pub fn read_wsc() -> Vec<SecurityProviderStatus> {
        Vec::new()
    }

    pub fn read_firewall() -> Option<FirewallStatus> {
        None
    }

    pub fn read_secure_boot() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_health_value() {
        assert_eq!(map_provider_health(0), "good");
        assert_eq!(map_provider_health(1), "notMonitored");
        assert_eq!(map_provider_health(2), "poor");
        assert_eq!(map_provider_health(3), "snooze");
    }

    #[test]
    fn unrecognized_health_value_is_unknown_not_a_guess() {
        assert_eq!(map_provider_health(99), "unknown");
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        if cfg!(not(target_os = "windows")) {
            let mut c = SecurityPostureCollector;
            assert!(!c.probe().is_ok());
        }
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = SecurityPostureCollector;
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis::now(),
        };
        let CollectorOutput::SecurityPosture(_sampled) = c.collect(&ctx) else {
            panic!("SecurityPostureCollector must return CollectorOutput::SecurityPosture");
        };
    }

    fn startup_item(name: &str, command: &str) -> StartupItem {
        StartupItem {
            name: name.to_string(),
            command: command.to_string(),
            location: system_pulse_core::types::StartupLocation::HkcuRun,
            enabled: true,
        }
    }

    #[test]
    fn extract_executable_path_handles_a_quoted_path_with_flags() {
        assert_eq!(
            extract_executable_path(r#""C:\Program Files\App\app.exe" --flag"#),
            Some(r"C:\Program Files\App\app.exe".to_string())
        );
    }

    #[test]
    fn extract_executable_path_handles_an_unquoted_bare_path() {
        assert_eq!(
            extract_executable_path(r"C:\Windows\System32\app.exe"),
            Some(r"C:\Windows\System32\app.exe".to_string())
        );
    }

    #[test]
    fn extract_executable_path_returns_none_for_nonsense_input() {
        assert_eq!(extract_executable_path(""), None);
        assert_eq!(extract_executable_path("rundll32"), None);
    }

    #[test]
    fn a_missing_target_file_is_flagged_regardless_of_signature() {
        let startup = [startup_item(
            "Ghost",
            r"C:\definitely\does\not\exist\ghost.exe",
        )];
        let findings = check_persistence(&startup, &[], &|_| Some(true));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("missing file"));
        assert_eq!(findings[0].signed, None);
    }

    /// A real, existing file whose name contains a `.` — mimicking a
    /// Windows `app.exe` path closely enough for `extract_executable_path`
    /// to recognize it, without depending on `current_exe()` (which, in
    /// this Linux test binary's case, has no extension at all and would
    /// make every one of these tests vacuously pass by never reaching
    /// `check_one` in the first place). Deliberately *not* under
    /// `std::env::temp_dir()`: on real Windows that resolves under
    /// `...\AppData\Local\Temp\`, which is exactly the pattern
    /// `check_one`'s own `suspicious_location` heuristic matches — a
    /// fixture placed there would contaminate the tests that mean to
    /// isolate the *signature* dimension alone. `CARGO_MANIFEST_DIR`'s own
    /// `target/` directory (already git-ignored, already what `cargo
    /// clean` removes) has no such collision. Each call gets its own
    /// subdirectory — this crate's tests run in parallel within one
    /// process, so a directory shared by name alone (e.g. just the pid)
    /// would race across them.
    fn real_file_with_extension(name: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-fixtures")
            .join(format!("sp-persist-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn an_existing_signed_file_outside_temp_is_not_flagged() {
        let exe = real_file_with_extension("outside-temp-marker.bin");
        let startup = [startup_item("Self", exe.to_str().unwrap())];
        let findings = check_persistence(&startup, &[], &|_| Some(true));
        std::fs::remove_dir_all(exe.parent().unwrap()).ok();
        // NOTE: the fixture path is itself under the OS temp directory, so
        // this only isolates the *signature* dimension — see the
        // `suspicious_location` check exercised separately via the literal
        // "\temp\" substring match, which is Windows-path-shaped and does
        // not match this test's Unix-style fixture path.
        assert!(
            findings.is_empty(),
            "a real, signed file whose path doesn't match the suspicious-location pattern must not be flagged"
        );
    }

    #[test]
    fn an_unsigned_existing_file_is_flagged() {
        let exe = real_file_with_extension("unsigned-marker.bin");
        let startup = [startup_item("Self", exe.to_str().unwrap())];
        let findings = check_persistence(&startup, &[], &|_| Some(false));
        std::fs::remove_dir_all(exe.parent().unwrap()).ok();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signed, Some(false));
    }

    #[test]
    fn never_fabricates_a_verdict_when_verification_is_unavailable() {
        let exe = real_file_with_extension("unverified-marker.bin");
        let startup = [startup_item("Self", exe.to_str().unwrap())];
        // Signature check unavailable (`None`) and not in a suspicious
        // location: must not be flagged just because verification
        // couldn't run.
        let findings = check_persistence(&startup, &[], &|_| None);
        std::fs::remove_dir_all(exe.parent().unwrap()).ok();
        assert!(findings.is_empty());
    }
}
