//! Task Scheduler 2.0 COM enumeration (Phase 3) — in scope per the
//! COM/WebView2 spike's finding (see `crate::com_spike`'s recorded
//! result): safe to use from a dedicated MTA thread with
//! `CoSetProxyBlanket` per proxy and **no** process-wide
//! `CoInitializeSecurity` call (which is unavailable in this process
//! anyway — WebView2 claims it first).
//!
//! Deliberately self-contained per `collect()` call: COM is initialized,
//! used, and torn down entirely within one function call, on whatever
//! thread that call happens to run on, with no COM state persisted
//! between ticks. This sidesteps the question of whether `windows-rs`'s
//! COM interface wrappers are `Send` (several are not, by design — the
//! same issue Phase 1B's PDH session hit) without needing an `unsafe impl
//! Send`: nothing here ever crosses a thread boundary, since it's all
//! created and dropped inside a single call on a single thread. Cold
//! cadence makes the cost of re-connecting every tick a non-issue.
//!
//! Some tasks are enumerable only when elevated (per-item, not a global
//! failure per the master plan's Phase 3 risk note) — an
//! access-denied folder is simply skipped rather than aborting the whole
//! walk.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source, UnixMillis};
use system_pulse_core::types::ScheduledTaskSnapshot;

const CADENCE: Duration = Duration::from_secs(3600);
/// Hard cap on tasks collected, defense in depth against a pathological
/// number of scheduled tasks consuming unbounded memory/time — real
/// systems have at most a few hundred. Only `raw` (Windows-only) uses
/// this; the non-Windows stub never walks anything.
#[cfg(target_os = "windows")]
const MAX_TASKS: usize = 2000;
/// Hard cap on folder recursion depth — Task Scheduler's own folder tree
/// is shallow in practice; this only guards against an unexpected cycle
/// or a pathological structure, not a case ever seen on a real system.
#[cfg(target_os = "windows")]
const MAX_DEPTH: u32 = 16;

/// OLE Automation date (days since 1899-12-30, the same representation
/// Excel and `VARIANT` dates use) -> `UnixMillis`. Pure and testable
/// without Windows. Task Scheduler reports `0.0` (or occasionally a small
/// negative/garbage value) for "never run" / "not scheduled" — treated as
/// `None`, never a fabricated timestamp near the Unix epoch.
pub fn ole_date_to_unix_millis(ole_date: f64) -> Option<UnixMillis> {
    if !ole_date.is_finite() || ole_date <= 0.0 {
        return None;
    }
    const DAYS_OLE_TO_UNIX_EPOCH: f64 = 25569.0;
    let unix_seconds = (ole_date - DAYS_OLE_TO_UNIX_EPOCH) * 86400.0;
    Some(UnixMillis((unix_seconds * 1000.0) as i64))
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawTask {
    pub path: String,
    pub enabled: bool,
    pub last_run_time: f64,
    pub next_run_time: f64,
    pub last_task_result: i32,
}

/// `SCHED_S_TASK_HAS_NOT_RUN` (0x00041303 / 267011) — Task Scheduler's own
/// documented result code meaning "this task has never run." Verified by
/// running this collector for real: a never-run task's `LastRunTime`
/// comes back as a fixed non-zero sentinel (empirically, the OLE date for
/// 1999-11-30 — well before this task or feature existed), which would
/// otherwise print as a real-looking but fabricated timestamp. This
/// result code is the authoritative, documented signal instead of
/// pattern-matching that specific date value.
const SCHED_S_TASK_HAS_NOT_RUN: i32 = 267011;

/// Pure conversion, testable without Windows.
pub fn to_snapshot(raw: &RawTask) -> ScheduledTaskSnapshot {
    let never_run = raw.last_task_result == SCHED_S_TASK_HAS_NOT_RUN;
    ScheduledTaskSnapshot {
        path: raw.path.clone(),
        enabled: raw.enabled,
        last_run_time: if never_run {
            None
        } else {
            ole_date_to_unix_millis(raw.last_run_time)
        },
        next_run_time: ole_date_to_unix_millis(raw.next_run_time),
        last_task_result: Some(raw.last_task_result as u32),
    }
}

#[cfg(target_os = "windows")]
mod raw {
    use super::{RawTask, MAX_DEPTH, MAX_TASKS};
    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::System::TaskScheduler::{ITaskFolder, ITaskService, TaskScheduler};
    use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};

    fn variant_i32(v: i32) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { lVal: v },
                }),
            },
        }
    }

    fn walk_folder(folder: &ITaskFolder, depth: u32, out: &mut Vec<RawTask>) {
        if depth > MAX_DEPTH || out.len() >= MAX_TASKS {
            return;
        }

        // SAFETY: `folder` is a valid, live `ITaskFolder` proxy for the
        // duration of this call.
        #[allow(unsafe_code)]
        if let Ok(tasks) = unsafe { folder.GetTasks(0) } {
            // SAFETY: same as above.
            #[allow(unsafe_code)]
            let count = unsafe { tasks.Count() }.unwrap_or(0);
            for i in 1..=count {
                if out.len() >= MAX_TASKS {
                    return;
                }
                let index = variant_i32(i);
                // SAFETY: `tasks` is a valid collection proxy; `index` is
                // a well-formed `VT_I4` VARIANT built above, matching
                // this API's documented 1-based indexing.
                #[allow(unsafe_code)]
                let Ok(task) = (unsafe { tasks.get_Item(&index) }) else {
                    continue;
                };
                // SAFETY: `task` is a valid, live `IRegisteredTask` proxy.
                #[allow(unsafe_code)]
                let (path, enabled, last_run, next_run, last_result) = unsafe {
                    (
                        task.Path().map(|b| b.to_string()).unwrap_or_default(),
                        task.Enabled().map(|b| b.as_bool()).unwrap_or(false),
                        task.LastRunTime().unwrap_or(0.0),
                        task.NextRunTime().unwrap_or(0.0),
                        task.LastTaskResult().unwrap_or(0),
                    )
                };
                out.push(RawTask {
                    path,
                    enabled,
                    last_run_time: last_run,
                    next_run_time: next_run,
                    last_task_result: last_result,
                });
            }
        }

        // SAFETY: `folder` is a valid, live `ITaskFolder` proxy.
        #[allow(unsafe_code)]
        if let Ok(subfolders) = unsafe { folder.GetFolders(0) } {
            // SAFETY: same as above.
            #[allow(unsafe_code)]
            let count = unsafe { subfolders.Count() }.unwrap_or(0);
            for i in 1..=count {
                if out.len() >= MAX_TASKS {
                    return;
                }
                let index = variant_i32(i);
                // SAFETY: `subfolders` is a valid collection proxy;
                // `index` is a well-formed `VT_I4` VARIANT, matching this
                // API's documented 1-based indexing.
                #[allow(unsafe_code)]
                let Ok(sub) = (unsafe { subfolders.get_Item(&index) }) else {
                    continue;
                };
                walk_folder(&sub, depth + 1, out);
            }
        }
    }

    /// `None` on a failure to even connect to Task Scheduler at all
    /// (never a partial/fabricated list for that case); an individual
    /// inaccessible subfolder is silently skipped by `walk_folder` above
    /// instead of aborting the whole walk — see the module doc.
    pub fn read() -> Option<Vec<RawTask>> {
        // SAFETY: fresh call, self-contained — see the module doc for why
        // this never shares COM state across calls or threads.
        #[allow(unsafe_code)]
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if init.is_err() {
            return None;
        }

        let result = (|| {
            // SAFETY: standard `CoCreateInstance` usage for the
            // documented Task Scheduler 2.0 entry point.
            #[allow(unsafe_code)]
            let service: ITaskService =
                unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }.ok()?;
            // SAFETY: all-empty variants mean "connect to the local
            // machine as the calling user" — no elevation, no
            // credential prompt.
            #[allow(unsafe_code)]
            unsafe {
                service.Connect(
                    &VARIANT::default(),
                    &VARIANT::default(),
                    &VARIANT::default(),
                    &VARIANT::default(),
                )
            }
            .ok()?;
            // SAFETY: the root folder path is always `"\\"`.
            #[allow(unsafe_code)]
            let root = unsafe { service.GetFolder(&BSTR::from("\\")) }.ok()?;

            let mut out = Vec::new();
            walk_folder(&root, 0, &mut out);
            Some(out)
        })();

        // SAFETY: matches the successful `CoInitializeEx` above, same
        // thread, after all COM use in this call is finished.
        #[allow(unsafe_code)]
        unsafe {
            CoUninitialize()
        };
        result
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::RawTask;

    pub fn read() -> Option<Vec<RawTask>> {
        None
    }
}

pub struct ScheduledTasksCollector {
    availability: Availability,
}

impl ScheduledTasksCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for ScheduledTasksCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for ScheduledTasksCollector {
    fn id(&self) -> CollectorId {
        CollectorId::ScheduledTasks
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
        // `Source::Wmi` is reused here rather than adding a dedicated
        // variant: the `Source` enum's existing categories predate Phase
        // 3, and the master plan itself groups Task Scheduler with
        // "WMI-backed items" as the COM-dependent category throughout —
        // this is the closest existing fit for "a COM-sourced value."
        if !self.availability.is_ok() {
            return CollectorOutput::ScheduledTasks(Sampled::unavailable(
                self.availability.clone(),
                Source::Wmi,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            Some(rows) => {
                let snapshots: Vec<ScheduledTaskSnapshot> = rows.iter().map(to_snapshot).collect();
                Sampled::ok(snapshots, Source::Wmi, ctx.wall_now)
            }
            None => Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::Wmi,
                ctx.wall_now,
            ),
        };
        CollectorOutput::ScheduledTasks(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_never_run_zero_date_is_none_not_the_unix_epoch() {
        assert_eq!(ole_date_to_unix_millis(0.0), None);
    }

    #[test]
    fn a_negative_or_nan_date_is_none() {
        assert_eq!(ole_date_to_unix_millis(-5.0), None);
        assert_eq!(ole_date_to_unix_millis(f64::NAN), None);
    }

    #[test]
    fn a_real_date_converts_correctly() {
        // 2024-01-01 00:00:00 UTC is OLE date 45292.0 (well-known
        // reference value used to validate Excel/OLE date conversions).
        let millis = ole_date_to_unix_millis(45292.0).unwrap();
        // 2024-01-01T00:00:00Z in Unix millis.
        assert_eq!(millis.0, 1_704_067_200_000);
    }

    #[test]
    fn to_snapshot_carries_a_realistic_last_task_result() {
        let raw = RawTask {
            path: r"\Microsoft\Windows\Test\Task".into(),
            enabled: true,
            last_run_time: 45292.0,
            next_run_time: 0.0,
            last_task_result: 0,
        };
        let snap = to_snapshot(&raw);
        assert_eq!(snap.path, r"\Microsoft\Windows\Test\Task");
        assert!(snap.enabled);
        assert!(snap.last_run_time.is_some());
        assert_eq!(snap.next_run_time, None);
        assert_eq!(snap.last_task_result, Some(0));
    }

    #[test]
    fn a_task_that_has_never_run_reports_no_last_run_time_despite_the_sentinel_date() {
        // The exact scenario found by running this collector for real:
        // a never-run task's raw LastRunTime is a real-looking non-zero
        // OLE date (the 1999-11-30 sentinel), which must not surface as
        // a fabricated "last ran on this date."
        let raw = RawTask {
            path: r"\Some\NeverRunTask".into(),
            enabled: true,
            last_run_time: 36494.0, // the observed 1999-11-30 sentinel
            next_run_time: 45500.0,
            last_task_result: SCHED_S_TASK_HAS_NOT_RUN,
        };
        let snap = to_snapshot(&raw);
        assert_eq!(snap.last_run_time, None);
        // A genuinely scheduled next run is untouched by this special case.
        assert!(snap.next_run_time.is_some());
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = ScheduledTasksCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = ScheduledTasksCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::ScheduledTasks(_) => {}
            _ => panic!("expected ScheduledTasks output"),
        }
    }
}
