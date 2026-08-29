//! Process list: pid, name, CPU/memory, exe, user, and identity for safe
//! termination. GPU memory attribution is filled in later by the assembly
//! stage (see `crate::collector::CollectorOutput::Gpu`) — this collector
//! doesn't know about GPU state, deliberately keeping collectors
//! independent of one another.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use sysinfo::{ProcessesToUpdate, System};

use crate::model::{Availability, Sampled, Source, UnixMillis};
use crate::process::{transform_processes, ProcessRow, ProcessSortKey, SortDir};
use crate::types::ProcessSnapshot;

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

const PROCESS_CADENCE: Duration = Duration::from_secs(2);
const PROCESS_LIST_LIMIT: usize = 300;

pub struct ProcessCollector {
    sys: Arc<Mutex<System>>,
}

impl ProcessCollector {
    pub fn new(sys: Arc<Mutex<System>>) -> Self {
        Self { sys }
    }
}

impl Collector for ProcessCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Process
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(PROCESS_CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.sys
            .lock()
            .refresh_processes(ProcessesToUpdate::All, true);
        Availability::Ok
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        let mut sys = self.sys.lock();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let rows: Vec<ProcessRow> = sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcessRow {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: p.cpu_usage(),
                memory: p.memory(),
                // Filled in by the assembly stage from the GPU collector's
                // per-process memory map.
                gpu_mem: None,
                exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
                user: p.user_id().map(uid_to_string),
                started_at: Some(UnixMillis((p.start_time() as i64).saturating_mul(1000))),
            })
            .collect();
        drop(sys);

        let processes: Vec<ProcessSnapshot> = transform_processes(
            rows,
            ProcessSortKey::Cpu,
            SortDir::Desc,
            None,
            PROCESS_LIST_LIMIT,
        );
        CollectorOutput::Process(Sampled::ok(processes, Source::Sysinfo, ctx.wall_now))
    }
}

fn uid_to_string(uid: &sysinfo::Uid) -> String {
    // `Uid` has no Display; use Debug and strip the wrapper: `Uid(1000)` -> `1000`.
    let s = format!("{uid:?}");
    s.strip_prefix("Uid(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(&s)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UnixMillis;
    use std::time::Instant;

    #[test]
    fn collects_at_least_the_current_process() {
        let sys = Arc::new(Mutex::new(System::new()));
        let mut c = ProcessCollector::new(sys);
        c.probe();
        let out = c.collect(&CollectCtx {
            now: Instant::now(),
            wall_now: UnixMillis(0),
        });
        match out {
            CollectorOutput::Process(sampled) => {
                assert!(sampled.availability.is_ok());
                let procs = sampled.value.unwrap();
                assert!(!procs.is_empty());
                assert!(procs.iter().all(|p| p.gpu_mem.is_none()));
            }
            _ => panic!("expected Process output"),
        }
    }
}
