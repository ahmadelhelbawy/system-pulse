//! Service Control Manager enumeration (Phase 3): `OpenSCManagerW` +
//! `EnumServicesStatusExW`, no COM. Read-only — starting/stopping a
//! service needs admin and is out of scope (see the plan's capability
//! matrix). Cold cadence: this is a "load once per session, refresh
//! rarely" list, not something that needs 1 Hz freshness.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::{ServiceSnapshot, ServiceStartType, ServiceStatus};

const CADENCE: Duration = Duration::from_secs(3600);

/// The fields actually read from the SCM, independent of the FFI call
/// itself — same separation `perf_info`/`smbios` use, so the raw-to-typed
/// mapping is unit-testable without calling into Windows.
#[derive(Debug, Clone, PartialEq)]
pub struct RawService {
    pub name: String,
    pub display_name: String,
    pub current_state: u32,
    pub process_id: u32,
    /// `None` when the per-service `QueryServiceConfigW` follow-up call
    /// failed (e.g. a service that vanished between enumeration and the
    /// query, or transient access denial) — never guessed.
    pub start_type: Option<u32>,
}

/// `SERVICE_STATUS_PROCESS.dwCurrentState` -> the contract enum. Returns
/// `None` for a value outside the documented range rather than guessing —
/// the caller skips the row (see `to_snapshot`), consistent with "no mock
/// data" for a value that shouldn't be possible on a real service.
fn map_state(raw: u32) -> Option<ServiceStatus> {
    match raw {
        1 => Some(ServiceStatus::Stopped),
        2 => Some(ServiceStatus::StartPending),
        3 => Some(ServiceStatus::StopPending),
        4 => Some(ServiceStatus::Running),
        5 => Some(ServiceStatus::ContinuePending),
        6 => Some(ServiceStatus::PausePending),
        7 => Some(ServiceStatus::Paused),
        _ => None,
    }
}

/// `QUERY_SERVICE_CONFIGW.dwStartType` -> the contract enum.
fn map_start_type(raw: u32) -> Option<ServiceStartType> {
    match raw {
        0 => Some(ServiceStartType::Boot),
        1 => Some(ServiceStartType::System),
        2 => Some(ServiceStartType::Automatic),
        3 => Some(ServiceStartType::Manual),
        4 => Some(ServiceStartType::Disabled),
        _ => None,
    }
}

/// Pure conversion, testable without Windows. A service whose *state*
/// isn't a documented value is skipped entirely (that shouldn't be
/// possible for a real service, so it's treated as a parse failure); a
/// missing or unrecognized start type just leaves that one field `None`
/// rather than dropping an otherwise-valid row.
pub fn to_snapshot(raw: &RawService) -> Option<ServiceSnapshot> {
    let status = map_state(raw.current_state)?;
    let start_type = raw.start_type.and_then(map_start_type);
    Some(ServiceSnapshot {
        name: raw.name.clone(),
        display_name: raw.display_name.clone(),
        status,
        start_type,
        pid: if raw.process_id == 0 {
            None
        } else {
            Some(raw.process_id)
        },
    })
}

#[cfg(target_os = "windows")]
mod raw {
    use super::RawService;
    use windows::core::PWSTR;
    use windows::Win32::System::Services::{
        CloseServiceHandle, EnumServicesStatusExW, OpenSCManagerW, OpenServiceW,
        QueryServiceConfigW, ENUM_SERVICE_STATUS_PROCESSW, QUERY_SERVICE_CONFIGW,
        SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_QUERY_CONFIG,
        SERVICE_STATE_ALL, SERVICE_WIN32,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn pwstr_to_string(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: `p` points at a null-terminated UTF-16 string owned by
        // the enumeration buffer this is called from, which outlives this
        // call.
        #[allow(unsafe_code)]
        unsafe {
            p.to_string().unwrap_or_default()
        }
    }

    /// The per-service `dwStartType` follow-up. `None` on any failure —
    /// never a guessed default.
    fn query_start_type(scm: SC_HANDLE, service_name: &str) -> Option<u32> {
        let name = wide(service_name);
        // SAFETY: `scm` is a valid, open SCM handle for the duration of
        // this call; `name` is a valid null-terminated UTF-16 string.
        #[allow(unsafe_code)]
        let handle = unsafe {
            OpenServiceW(
                scm,
                windows::core::PCWSTR(name.as_ptr()),
                SERVICE_QUERY_CONFIG,
            )
        }
        .ok()?;

        let mut needed = 0u32;
        // SAFETY: a `None` buffer with a zeroed size pointer is exactly
        // how this API reports the required buffer size.
        #[allow(unsafe_code)]
        let _ = unsafe { QueryServiceConfigW(handle, None, 0, &mut needed) };

        let mut buf = vec![0u8; needed as usize];
        // SAFETY: `buf` is exactly `needed` bytes as reported by the
        // sizing call above.
        #[allow(unsafe_code)]
        let ok = unsafe {
            QueryServiceConfigW(
                handle,
                Some(buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW),
                needed,
                &mut needed,
            )
        };
        // SAFETY: `handle` was successfully opened above and is closed
        // exactly once, here, regardless of the query's outcome.
        // `CloseServiceHandle`, not `CloseHandle` — an `SC_HANDLE` is a
        // distinct handle type from a general `HANDLE`.
        #[allow(unsafe_code)]
        unsafe {
            let _ = CloseServiceHandle(handle);
        }
        if ok.is_err() {
            return None;
        }
        // SAFETY: `buf` was filled by the successful call above and is
        // large enough to hold a full `QUERY_SERVICE_CONFIGW`.
        #[allow(unsafe_code)]
        let cfg = unsafe { &*(buf.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
        Some(cfg.dwStartType.0 as u32)
    }

    /// `None` on any enumeration failure — never a partial/fabricated list.
    pub fn read() -> Option<Vec<RawService>> {
        // SAFETY: no handle is held yet; opening the SCM with
        // enumerate-only access is the documented unprivileged path.
        #[allow(unsafe_code)]
        let scm = unsafe {
            OpenSCManagerW(
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                SC_MANAGER_ENUMERATE_SERVICE,
            )
        }
        .ok()?;

        let mut needed = 0u32;
        let mut count = 0u32;
        let mut resume = 0u32;
        // SAFETY: a `None` buffer with zeroed size/count pointers is
        // exactly how this API reports the required buffer size.
        //
        // The sizing call's own success/failure status is deliberately
        // *not* checked here: it always "fails" with
        // `ERROR_INSUFFICIENT_BUFFER` when there's at least one service
        // (the expected, documented outcome of this exact idiom, not a
        // real error), and comparing that failure's HRESULT against
        // `ERROR_INSUFFICIENT_BUFFER.to_hresult()` turned out not to
        // match in practice on a real host — verified by running this
        // collector for real, where the strict comparison caused every
        // enumeration to report `Failed` with zero services despite the
        // SCM being reachable. `needed` is populated regardless of the
        // return status either way, so checking it directly is both
        // simpler and correct.
        #[allow(unsafe_code)]
        let _ = unsafe {
            EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut needed,
                &mut count,
                Some(&mut resume),
                windows::core::PCWSTR::null(),
            )
        };
        if needed == 0 {
            // SAFETY: `scm` was successfully opened above.
            #[allow(unsafe_code)]
            unsafe {
                let _ = CloseServiceHandle(scm);
            }
            return Some(Vec::new());
        }

        let mut buf = vec![0u8; needed as usize];
        let mut returned = 0u32;
        resume = 0;
        // SAFETY: `buf` is exactly `needed` bytes as reported by the
        // sizing call above.
        #[allow(unsafe_code)]
        let ok = unsafe {
            EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(&mut buf),
                &mut needed,
                &mut returned,
                Some(&mut resume),
                windows::core::PCWSTR::null(),
            )
        };
        if ok.is_err() {
            // SAFETY: `scm` was successfully opened above.
            #[allow(unsafe_code)]
            unsafe {
                let _ = CloseServiceHandle(scm);
            }
            return None;
        }

        let entries = buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;
        let mut out = Vec::with_capacity(returned as usize);
        for i in 0..returned as usize {
            // SAFETY: `returned` came from the same call that filled
            // `buf`; each entry is a full `ENUM_SERVICE_STATUS_PROCESSW`.
            #[allow(unsafe_code)]
            let entry = unsafe { &*entries.add(i) };
            let name = pwstr_to_string(entry.lpServiceName);
            let display_name = pwstr_to_string(entry.lpDisplayName);
            let start_type = query_start_type(scm, &name);
            out.push(RawService {
                current_state: entry.ServiceStatusProcess.dwCurrentState.0,
                process_id: entry.ServiceStatusProcess.dwProcessId,
                name,
                display_name,
                start_type,
            });
        }

        // SAFETY: `scm` was successfully opened above and is closed
        // exactly once, here.
        #[allow(unsafe_code)]
        unsafe {
            let _ = CloseServiceHandle(scm);
        }
        Some(out)
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::RawService;

    pub fn read() -> Option<Vec<RawService>> {
        None
    }
}

pub struct ServicesCollector {
    availability: Availability,
}

impl ServicesCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for ServicesCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for ServicesCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Services
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
            return CollectorOutput::Services(Sampled::unavailable(
                self.availability.clone(),
                Source::Registry,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            Some(rows) => {
                let snapshots: Vec<ServiceSnapshot> = rows.iter().filter_map(to_snapshot).collect();
                Sampled::ok(snapshots, Source::Registry, ctx.wall_now)
            }
            None => Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::Registry,
                ctx.wall_now,
            ),
        };
        CollectorOutput::Services(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(state: u32, start: Option<u32>) -> RawService {
        RawService {
            name: "svc".into(),
            display_name: "Service".into(),
            current_state: state,
            process_id: 1234,
            start_type: start,
        }
    }

    #[test]
    fn maps_every_documented_state() {
        for (raw_val, expected) in [
            (1, ServiceStatus::Stopped),
            (2, ServiceStatus::StartPending),
            (3, ServiceStatus::StopPending),
            (4, ServiceStatus::Running),
            (5, ServiceStatus::ContinuePending),
            (6, ServiceStatus::PausePending),
            (7, ServiceStatus::Paused),
        ] {
            assert_eq!(map_state(raw_val), Some(expected));
        }
    }

    #[test]
    fn unknown_state_is_none_not_a_guess() {
        assert_eq!(map_state(99), None);
    }

    #[test]
    fn snapshot_carries_pid_only_when_nonzero() {
        let s = to_snapshot(&raw(4, Some(2))).unwrap();
        assert_eq!(s.pid, Some(1234));

        let mut r = raw(1, Some(3));
        r.process_id = 0;
        let s = to_snapshot(&r).unwrap();
        assert_eq!(s.pid, None);
    }

    #[test]
    fn a_service_with_no_start_type_still_reports_everything_else() {
        let s = to_snapshot(&raw(4, None)).unwrap();
        assert_eq!(s.status, ServiceStatus::Running);
        assert_eq!(s.start_type, None);
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = ServicesCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = ServicesCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::Services(_) => {}
            _ => panic!("expected Services output"),
        }
    }
}
