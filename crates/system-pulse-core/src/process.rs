//! Process list transformation and process termination.
//!
//! The frontend performs its own interactive sort/filter on the received list;
//! these helpers define the canonical backend default and are unit-tested.

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};
use thiserror::Error;
use ts_rs::TS;

use crate::model::UnixMillis;
use crate::types::ProcessSnapshot;

/// A process's identity, not just its PID. Windows recycles PIDs
/// aggressively; between the frame that rendered a process row and the
/// click that kills it, the PID may already belong to something else.
/// `started_at` is the tie-breaker: `kill_process` re-reads it immediately
/// before terminating and refuses to act on a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub started_at: UnixMillis,
}

/// Creation time comes from `sysinfo::Process::start_time()`, which is
/// second-granularity on every platform it supports (internally backed by
/// `GetProcessTimes` on Windows, `/proc/<pid>/stat` on Linux) — going
/// through sysinfo rather than a raw Win32 call keeps this a zero-new-API
/// port. Comparisons therefore use a ±1s tolerance uniformly rather than
/// treating Windows as exact.
const IDENTITY_TOLERANCE_MS: i64 = 1000;

/// Reads the canonical identity of a still-running process.
pub fn identity(pid: u32) -> Option<ProcessIdentity> {
    let target = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    let process = sys.process(target)?;
    Some(ProcessIdentity {
        pid,
        started_at: start_time_to_unix_millis(process.start_time()),
    })
}

fn start_time_to_unix_millis(start_time_secs: u64) -> UnixMillis {
    UnixMillis((start_time_secs as i64).saturating_mul(1000))
}

fn identity_matches(expected: UnixMillis, actual: UnixMillis) -> bool {
    (expected.as_millis() - actual.as_millis()).abs() <= IDENTITY_TOLERANCE_MS
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum KillError {
    #[error("no process with pid {0}")]
    NotFound(u32),
    #[error("access denied terminating pid {0} (the process may require elevation)")]
    AccessDenied(u32),
    /// The pid exists, but its creation time no longer matches what the
    /// caller expected — it's a different process than the one the UI
    /// showed (almost always a recycled PID). Nothing was terminated.
    #[error("pid {pid} no longer matches the process that was shown (it likely already exited)")]
    IdentityMismatch {
        pid: u32,
        expected: UnixMillis,
        actual: Option<UnixMillis>,
    },
}

/// Terminate a process, but only if it's still the exact process `expected`
/// identifies. Uses the OS primitive (`TerminateProcess` on Windows,
/// `SIGTERM`/`SIGKILL` on Unix) via `sysinfo`; never shells out.
pub fn kill_process(expected: ProcessIdentity) -> Result<(), KillError> {
    if expected.pid == 0 {
        return Err(KillError::NotFound(expected.pid));
    }
    let target = Pid::from_u32(expected.pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    let Some(process) = sys.process(target) else {
        return Err(KillError::NotFound(expected.pid));
    };

    let actual = start_time_to_unix_millis(process.start_time());
    if !identity_matches(expected.started_at, actual) {
        return Err(KillError::IdentityMismatch {
            pid: expected.pid,
            expected: expected.started_at,
            actual: Some(actual),
        });
    }

    if process.kill() {
        Ok(())
    } else {
        Err(KillError::AccessDenied(expected.pid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSortKey {
    Cpu,
    Memory,
    Name,
    Pid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// Raw process data assembled by the sampler before normalization.
#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory: u64,
    pub gpu_mem: Option<u64>,
    pub exe: Option<String>,
    pub user: Option<String>,
    pub started_at: Option<UnixMillis>,
}

/// Normalize a single raw row into a presentable snapshot.
pub fn normalize_process(row: ProcessRow) -> ProcessSnapshot {
    ProcessSnapshot {
        pid: row.pid,
        name: if row.name.trim().is_empty() {
            "<unknown>".to_string()
        } else {
            row.name
        },
        cpu_percent: if row.cpu_percent.is_finite() {
            row.cpu_percent.max(0.0)
        } else {
            0.0
        },
        memory: row.memory,
        gpu_mem: row.gpu_mem,
        exe: row.exe,
        user: row.user,
        started_at: row.started_at,
    }
}

/// Normalize, filter, sort, and truncate a list of process rows.
pub fn transform_processes(
    rows: Vec<ProcessRow>,
    sort: ProcessSortKey,
    dir: SortDir,
    query: Option<&str>,
    limit: usize,
) -> Vec<ProcessSnapshot> {
    let q = query.map(str::to_ascii_lowercase);
    let mut out: Vec<ProcessSnapshot> = rows
        .into_iter()
        .map(normalize_process)
        .filter(|p| match &q {
            None => true,
            Some(q) => {
                q.is_empty()
                    || p.name.to_ascii_lowercase().contains(q)
                    || p.pid.to_string().contains(q)
            }
        })
        .collect();

    out.sort_by(|a, b| {
        let ord = match sort {
            ProcessSortKey::Cpu => a
                .cpu_percent
                .partial_cmp(&b.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            ProcessSortKey::Memory => a.memory.cmp(&b.memory),
            ProcessSortKey::Pid => a.pid.cmp(&b.pid),
            ProcessSortKey::Name => a
                .name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase()),
        };
        match dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    });

    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessRow {
        ProcessRow {
            pid,
            name: name.into(),
            cpu_percent: cpu,
            memory: mem,
            gpu_mem: None,
            exe: None,
            user: None,
            started_at: None,
        }
    }

    #[test]
    fn sorts_by_cpu_desc_by_default() {
        let rows = vec![
            row(1, "a", 5.0, 10),
            row(2, "b", 50.0, 10),
            row(3, "c", 20.0, 10),
        ];
        let out = transform_processes(rows, ProcessSortKey::Cpu, SortDir::Desc, None, 100);
        assert_eq!(out[0].name, "b");
        assert_eq!(out[1].name, "c");
        assert_eq!(out[2].name, "a");
    }

    #[test]
    fn filters_case_insensitively_and_by_pid() {
        let rows = vec![row(1234, "Chrome", 1.0, 1), row(99, "explorer", 2.0, 1)];
        let by_name = transform_processes(
            rows.clone(),
            ProcessSortKey::Name,
            SortDir::Asc,
            Some("chrome"),
            10,
        );
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].pid, 1234);

        let by_pid = transform_processes(rows, ProcessSortKey::Name, SortDir::Asc, Some("99"), 10);
        assert_eq!(by_pid.len(), 1);
        assert_eq!(by_pid[0].name, "explorer");
    }

    #[test]
    fn limits_results() {
        let rows: Vec<ProcessRow> = (0..50).map(|i| row(i, &format!("p{i}"), 1.0, 1)).collect();
        assert_eq!(
            transform_processes(rows, ProcessSortKey::Pid, SortDir::Asc, None, 5).len(),
            5
        );
    }

    #[test]
    fn normalizes_blank_name_and_negative_cpu() {
        let out = transform_processes(
            vec![row(1, "", -5.0, 0)],
            ProcessSortKey::Pid,
            SortDir::Asc,
            None,
            10,
        );
        assert_eq!(out[0].name, "<unknown>");
        assert_eq!(out[0].cpu_percent, 0.0);
    }

    fn identity_at(pid: u32, secs: i64) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            started_at: UnixMillis(secs * 1000),
        }
    }

    #[test]
    fn kill_missing_process_is_not_found() {
        assert_eq!(kill_process(identity_at(0, 0)), Err(KillError::NotFound(0)));
    }

    #[test]
    fn identity_matches_allows_one_second_of_slack() {
        assert!(identity_matches(UnixMillis(1000), UnixMillis(1999)));
        assert!(identity_matches(UnixMillis(1000), UnixMillis(1000)));
        assert!(!identity_matches(UnixMillis(1000), UnixMillis(2001)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_spawned_process_succeeds_then_not_found() {
        use std::process::Command;
        use std::thread::sleep;
        use std::time::Duration;

        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        sleep(Duration::from_millis(100));

        let id = identity(pid).expect("spawned process must have a readable identity");
        assert_eq!(id.pid, pid);

        assert_eq!(kill_process(id), Ok(()));
        // Reap the zombie so the pid disappears from the process table.
        let _ = child.wait();
        sleep(Duration::from_millis(50));
        assert_eq!(kill_process(id), Err(KillError::NotFound(pid)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_refuses_a_stale_identity_and_leaves_the_real_process_running() {
        use std::process::Command;
        use std::thread::sleep;
        use std::time::Duration;

        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        sleep(Duration::from_millis(100));

        let real_id = identity(pid).expect("spawned process must have a readable identity");
        // Simulate a PID recycled by a different process: same pid, a
        // creation time far enough away to exceed the tolerance.
        let stale = ProcessIdentity {
            pid,
            started_at: UnixMillis(real_id.started_at.as_millis() - 60_000),
        };

        let result = kill_process(stale);
        assert!(matches!(result, Err(KillError::IdentityMismatch { .. })));

        // The real process must be untouched — confirmed by successfully
        // killing it with its correct identity afterward.
        assert_eq!(kill_process(real_id), Ok(()));
        let _ = child.wait();
        sleep(Duration::from_millis(50));
    }
}
