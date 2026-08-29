//! `GetPerformanceInfo` (psapi): handles, threads, process count, and the
//! commit/pool/cache figures Task Manager's Performance tab derives its
//! "Committed" and "Cached" numbers from.
//!
//! **Cadence caveat — deliberately conservative.** The master plan's
//! capability matrix assumes this call is sub-millisecond and starts it on
//! the `Hot` tier, but explicitly requires that to be measured before
//! shipping ("assumed sub-millisecond, not proven"). This dev environment
//! is WSL2 with no Windows target — it cannot execute this call at all, so
//! that measurement cannot be made here. Rather than gamble the hot
//! thread's budget on an unverified assumption, this collector ships on
//! `Warm(1s)` — as fresh as `Hot` would be in practice, without the risk.
//! Promote to `Hot` (moving its construction into
//! `system_pulse_core::scheduler::hot::HotLoop` alongside Cpu/Memory) only
//! after a real Windows host confirms sub-millisecond timing under load.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::WindowsInternalState;

const CADENCE: Duration = Duration::from_secs(1);

/// The fields `GetPerformanceInfo` actually fills in, independent of the
/// FFI call itself — kept separate so the byte-to-struct conversion (which
/// involves a pages-to-bytes multiplication) is unit-testable without
/// calling into Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawPerformanceInfo {
    pub commit_total_pages: u64,
    pub commit_limit_pages: u64,
    pub physical_total_pages: u64,
    pub system_cache_pages: u64,
    pub kernel_paged_pages: u64,
    pub kernel_non_paged_pages: u64,
    pub page_size: u64,
    pub handle_count: u32,
    pub process_count: u32,
    pub thread_count: u32,
}

/// Converts the raw page-counted struct into the byte-counted contract
/// type. Pure and platform-independent — see the module doc for why the
/// FFI call itself is kept out of this function.
pub fn to_windows_internal_state(raw: &RawPerformanceInfo) -> WindowsInternalState {
    let page = raw.page_size.max(1); // a zero page size would be a bogus read; never divide/multiply by 0
    WindowsInternalState {
        handle_count: raw.handle_count,
        process_count: raw.process_count,
        thread_count: raw.thread_count,
        commit_total: raw.commit_total_pages.saturating_mul(page),
        commit_limit: raw.commit_limit_pages.saturating_mul(page),
        kernel_paged_pool: raw.kernel_paged_pages.saturating_mul(page),
        kernel_non_paged_pool: raw.kernel_non_paged_pages.saturating_mul(page),
        system_cache: raw.system_cache_pages.saturating_mul(page),
    }
}

#[cfg(target_os = "windows")]
mod raw {
    use super::RawPerformanceInfo;
    use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

    /// `None` on API failure — a failed call must never be reported as a
    /// zeroed-out reading (the 1.0 "dead collector looks idle" defect).
    pub fn read() -> Option<RawPerformanceInfo> {
        let mut info = PERFORMANCE_INFORMATION {
            cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is a valid, correctly-sized stack local; `cb` is
        // set to its exact size as the API requires.
        #[allow(unsafe_code)]
        let ok = unsafe { GetPerformanceInfo(&mut info, info.cb) };
        if ok.is_err() {
            return None;
        }
        Some(RawPerformanceInfo {
            commit_total_pages: info.CommitTotal as u64,
            commit_limit_pages: info.CommitLimit as u64,
            physical_total_pages: info.PhysicalTotal as u64,
            system_cache_pages: info.SystemCache as u64,
            kernel_paged_pages: info.KernelPaged as u64,
            kernel_non_paged_pages: info.KernelNonpaged as u64,
            page_size: info.PageSize as u64,
            handle_count: info.HandleCount,
            process_count: info.ProcessCount,
            thread_count: info.ThreadCount,
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::RawPerformanceInfo;

    pub fn read() -> Option<RawPerformanceInfo> {
        None
    }
}

pub struct PerfInfoCollector {
    availability: Availability,
}

impl PerfInfoCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for PerfInfoCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for PerfInfoCollector {
    fn id(&self) -> CollectorId {
        CollectorId::WindowsInternal
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(CADENCE)
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
            return CollectorOutput::WindowsInternal(Sampled::unavailable(
                self.availability.clone(),
                Source::PerfInfo,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            Some(r) => Sampled::ok(
                to_windows_internal_state(&r),
                Source::PerfInfo,
                ctx.wall_now,
            ),
            None => Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::PerfInfo,
                ctx.wall_now,
            ),
        };
        CollectorOutput::WindowsInternal(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_converted_to_bytes() {
        let raw = RawPerformanceInfo {
            commit_total_pages: 1000,
            commit_limit_pages: 2000,
            physical_total_pages: 4_000_000,
            system_cache_pages: 500,
            kernel_paged_pages: 100,
            kernel_non_paged_pages: 50,
            page_size: 4096,
            handle_count: 12345,
            process_count: 200,
            thread_count: 3000,
        };
        let state = to_windows_internal_state(&raw);
        assert_eq!(state.commit_total, 1000 * 4096);
        assert_eq!(state.commit_limit, 2000 * 4096);
        assert_eq!(state.kernel_paged_pool, 100 * 4096);
        assert_eq!(state.kernel_non_paged_pool, 50 * 4096);
        assert_eq!(state.system_cache, 500 * 4096);
        assert_eq!(state.handle_count, 12345);
        assert_eq!(state.process_count, 200);
        assert_eq!(state.thread_count, 3000);
    }

    #[test]
    fn a_zero_page_size_does_not_zero_out_or_panic() {
        // A malformed/zeroed read must not silently report 0 bytes (which
        // would look like a real, tiny commit total) or divide-by-zero
        // panic; treated as page_size=1 so the raw counts pass through.
        let raw = RawPerformanceInfo {
            commit_total_pages: 42,
            page_size: 0,
            ..Default::default()
        };
        let state = to_windows_internal_state(&raw);
        assert_eq!(state.commit_total, 42);
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = PerfInfoCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_reports_a_well_formed_unavailable_state() {
        // On this (non-Windows) host, collect() must never panic and must
        // never fabricate a value — exactly the "no mock data" contract.
        let mut c = PerfInfoCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::WindowsInternal(_s) => {
                #[cfg(not(target_os = "windows"))]
                {
                    assert_eq!(_s.value, None);
                    assert!(!_s.availability.is_ok());
                }
            }
            _ => panic!("expected WindowsInternal output"),
        }
    }
}
