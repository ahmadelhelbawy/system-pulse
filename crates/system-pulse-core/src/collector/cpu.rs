//! Headline + per-core CPU utilization.
//!
//! Ported from the 1.0 sampler behaviorally unchanged, with one fix: a
//! `CpuTimesSource` read failure (`GetSystemTimes`/`/proc/stat` failing) now
//! reports `Availability::Failed` instead of silently emitting a `0.0`
//! utilization — a dead collector must never look like an idle CPU.

use std::sync::Arc;

use parking_lot::Mutex;
use sysinfo::{CpuRefreshKind, System};

use crate::calc::compute_cpu_percent;
use crate::model::{Availability, FailureCode, Sampled, Source};
use crate::platform::{self, CpuTimesSource};
use crate::types::{CpuSnapshot, CpuTimes};

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

#[cfg(target_os = "windows")]
const SOURCE: Source = Source::GetSystemTimes;
#[cfg(target_os = "linux")]
const SOURCE: Source = Source::ProcStat;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const SOURCE: Source = Source::Sysinfo;

pub struct CpuCollector {
    sys: Arc<Mutex<System>>,
    cpu_source: Box<dyn CpuTimesSource>,
    prev_cpu: Option<CpuTimes>,
}

impl CpuCollector {
    pub fn new(sys: Arc<Mutex<System>>) -> Self {
        Self {
            sys,
            cpu_source: platform::default_source(),
            prev_cpu: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source(sys: Arc<Mutex<System>>, source: Box<dyn CpuTimesSource>) -> Self {
        Self {
            sys,
            cpu_source: source,
            prev_cpu: None,
        }
    }
}

impl Collector for CpuCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Cpu
    }

    fn cadence(&self) -> Cadence {
        Cadence::Hot
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.sys
            .lock()
            .refresh_cpu_specifics(CpuRefreshKind::everything());
        match self.cpu_source.read() {
            Some(t) => {
                self.prev_cpu = Some(t);
                Availability::Ok
            }
            None => Availability::failed(FailureCode::ApiError),
        }
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        let curr = self.cpu_source.read();

        let (total_percent, availability) = match (self.prev_cpu, curr) {
            (Some(prev), Some(curr)) => {
                self.prev_cpu = Some(curr);
                (compute_cpu_percent(&prev, &curr), Availability::Ok)
            }
            (None, Some(curr)) => {
                // First successful read since construction/reset: no prior
                // sample to diff against yet, so no rate — but this is a
                // real state, not a failure.
                self.prev_cpu = Some(curr);
                (0.0, Availability::Ok)
            }
            (_, None) => (0.0, Availability::failed(FailureCode::ApiError)),
        };

        let mut sys = self.sys.lock();
        sys.refresh_cpu_usage();
        let cpus = sys.cpus();
        let per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
        let frequency_mhz = cpus.first().map(|c| c.frequency()).filter(|f| *f > 0);
        let core_count = cpus.len();
        drop(sys);

        let snapshot = CpuSnapshot {
            total_percent,
            per_core,
            frequency_mhz,
            core_count,
        };

        let sampled = if availability.is_ok() {
            Sampled::ok(snapshot, SOURCE, ctx.wall_now)
        } else {
            Sampled::unavailable(availability, SOURCE, ctx.wall_now)
        };
        CollectorOutput::Cpu(sampled)
    }

    /// See the 1.0 defect this fixes: nothing used to reset `prev_cpu` when
    /// `visible` flipped back to true, so the first post-resume frame
    /// averaged CPU over the entire hidden window.
    fn reset_baseline(&mut self) {
        self.prev_cpu = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::test_support::{ctx_at, ScriptedCpuTimes};
    use std::time::Instant;

    fn times(idle: u64, total: u64) -> CpuTimes {
        CpuTimes { idle, total }
    }

    #[test]
    fn failed_read_reports_failed_not_zero_percent() {
        let sys = Arc::new(Mutex::new(System::new()));
        let mut c = CpuCollector::with_source(
            sys,
            Box::new(ScriptedCpuTimes::new(vec![Some(times(0, 100)), None])),
        );
        c.probe();
        let out = c.collect(&ctx_at(Instant::now()));
        match out {
            CollectorOutput::Cpu(sampled) => {
                assert!(!sampled.availability.is_ok());
                assert_eq!(sampled.value, None);
            }
            _ => panic!("expected Cpu output"),
        }
    }

    #[test]
    fn successful_reads_compute_a_real_percentage() {
        let sys = Arc::new(Mutex::new(System::new()));
        let mut c = CpuCollector::with_source(
            sys,
            Box::new(ScriptedCpuTimes::new(vec![
                Some(times(0, 100)),
                Some(times(50, 200)),
            ])),
        );
        c.probe();
        let out = c.collect(&ctx_at(Instant::now()));
        match out {
            CollectorOutput::Cpu(sampled) => {
                assert!(sampled.availability.is_ok());
                let v = sampled.value.unwrap();
                assert!((v.total_percent - 50.0).abs() < f32::EPSILON);
            }
            _ => panic!("expected Cpu output"),
        }
    }

    #[test]
    fn reset_baseline_prevents_averaging_over_a_pause() {
        let sys = Arc::new(Mutex::new(System::new()));
        let mut c = CpuCollector::with_source(
            sys,
            Box::new(ScriptedCpuTimes::new(vec![
                Some(times(0, 100)),
                Some(times(50, 200)),
            ])),
        );
        c.probe();
        c.reset_baseline();
        // With the baseline reset, the very next read has no prior sample —
        // it must report 0.0/Ok (a real "no rate yet" state), not compute a
        // percentage against a stale pre-pause reading.
        let out = c.collect(&ctx_at(Instant::now()));
        match out {
            CollectorOutput::Cpu(sampled) => {
                assert!(sampled.availability.is_ok());
                assert_eq!(sampled.value.unwrap().total_percent, 0.0);
            }
            _ => panic!("expected Cpu output"),
        }
    }
}
