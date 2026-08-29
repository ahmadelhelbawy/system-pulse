//! A warm-tier worker thread: owns a fixed subset of `Warm`/`Cold`
//! collectors and runs each on its own wall-clock cadence, publishing
//! results into [`SharedSections`] for the hot thread to pick up.
//!
//! A collector that blocks or runs long only delays *itself* — it shares no
//! lock and no thread with the hot loop, which is what actually fixes the
//! 1.0 defect ("any blocking collector stalls the 1 Hz headline metrics").

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::collector::{Cadence, CollectCtx, Collector, CollectorOutput};
use crate::model::UnixMillis;

use super::shared::SharedSections;
use super::wake::WakeSignal;

/// Small std-only jitter so multiple collectors on the same worker don't
/// stay permanently aligned to the same wall-clock tick — derived from the
/// collector's position in the worker's list, applied once at first
/// schedule only (not every cycle, so cadence stays exact after that).
fn initial_jitter(index: usize) -> Duration {
    Duration::from_millis((index as u64 * 137) % 250)
}

struct Scheduled {
    collector: Box<dyn Collector>,
    cadence: Duration,
    next_due: Instant,
}

pub(crate) struct WorkerLoop {
    scheduled: Vec<Scheduled>,
    shutdown: Arc<AtomicBool>,
    visible: Arc<AtomicBool>,
    resume_pending: Arc<AtomicBool>,
    wake: Arc<WakeSignal>,
    sections: SharedSections,
}

impl WorkerLoop {
    pub fn new(
        collectors: Vec<Box<dyn Collector>>,
        shutdown: Arc<AtomicBool>,
        visible: Arc<AtomicBool>,
        resume_pending: Arc<AtomicBool>,
        wake: Arc<WakeSignal>,
        sections: SharedSections,
    ) -> Self {
        let start = Instant::now();
        let scheduled = collectors
            .into_iter()
            .enumerate()
            .map(|(i, mut c)| {
                let cadence = match c.cadence() {
                    Cadence::Warm(d) | Cadence::Cold(d) => d,
                    Cadence::Hot => Duration::from_millis(0),
                    Cadence::OnDemand => Duration::MAX,
                };
                c.probe();
                Scheduled {
                    collector: c,
                    cadence,
                    next_due: start + initial_jitter(i),
                }
            })
            .collect();
        Self {
            scheduled,
            shutdown,
            visible,
            resume_pending,
            wake,
            sections,
        }
    }

    pub fn run(mut self) {
        const HIDDEN_POLL: Duration = Duration::from_millis(250);

        while !self.shutdown.load(Ordering::Relaxed) {
            if !self.visible.load(Ordering::Relaxed) {
                self.wake.wait_forever(HIDDEN_POLL);
                continue;
            }

            if self.resume_pending.swap(false, Ordering::AcqRel) {
                for s in &mut self.scheduled {
                    s.collector.reset_baseline();
                }
            }

            let now = Instant::now();
            let mut next_wake = now + HIDDEN_POLL;

            for s in &mut self.scheduled {
                if s.cadence == Duration::MAX {
                    continue; // OnDemand: never auto-scheduled
                }
                if now >= s.next_due {
                    let ctx = CollectCtx {
                        now,
                        wall_now: UnixMillis::now(),
                    };
                    let output = s.collector.collect(&ctx);
                    publish(&self.sections, output);
                    s.next_due = now + s.cadence;
                }
                next_wake = next_wake.min(s.next_due);
            }

            self.wake.wait_until(next_wake);
        }
    }
}

fn publish(sections: &SharedSections, output: CollectorOutput) {
    let mut sections = sections.lock();
    match output {
        CollectorOutput::Disk { disks, io } => {
            sections.disks = Some(disks);
            sections.disk_io = Some(io);
        }
        CollectorOutput::Network(net) => {
            sections.networks = Some(net);
        }
        CollectorOutput::Gpu {
            devices,
            process_mem,
        } => {
            sections.gpu = Some(devices);
            sections.gpu_process_mem = process_mem;
        }
        CollectorOutput::Process(proc) => {
            sections.processes = Some(proc);
        }
        // Hot-only outputs never originate from a worker loop.
        CollectorOutput::Cpu(_) | CollectorOutput::Memory(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{CollectorId, Privilege};
    use crate::model::{Availability, Sampled, Source};
    use crate::scheduler::shared::new_shared_sections;
    use crate::scheduler::wake::WakeSignal;
    use std::sync::atomic::AtomicU32;
    use std::thread;

    struct CountingCollector {
        calls: Arc<AtomicU32>,
        cadence: Duration,
        reset_calls: Arc<AtomicU32>,
    }

    impl Collector for CountingCollector {
        fn id(&self) -> CollectorId {
            CollectorId::Process
        }
        fn cadence(&self) -> Cadence {
            Cadence::Warm(self.cadence)
        }
        fn required_privilege(&self) -> Privilege {
            Privilege::User
        }
        fn probe(&mut self) -> Availability {
            Availability::Ok
        }
        fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
            self.calls.fetch_add(1, Ordering::Relaxed);
            CollectorOutput::Process(Sampled::ok(vec![], Source::Sysinfo, ctx.wall_now))
        }
        fn reset_baseline(&mut self) {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A collector that blocks for `stall` on every call — proves a slow
    /// warm collector delays only itself, never the hot thread (which lives
    /// entirely outside this worker and isn't exercised by this test, but a
    /// second fast collector on the SAME worker is used as the witness:
    /// its own cadence must still be honoured despite sharing a thread with
    /// the slow one).
    struct SlowCollector {
        stall: Duration,
        calls: Arc<AtomicU32>,
    }

    impl Collector for SlowCollector {
        fn id(&self) -> CollectorId {
            CollectorId::Gpu
        }
        fn cadence(&self) -> Cadence {
            Cadence::Warm(Duration::from_millis(10))
        }
        fn required_privilege(&self) -> Privilege {
            Privilege::User
        }
        fn probe(&mut self) -> Availability {
            Availability::Ok
        }
        fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
            thread::sleep(self.stall);
            self.calls.fetch_add(1, Ordering::Relaxed);
            CollectorOutput::Gpu {
                devices: Sampled::ok(vec![], Source::Nvml, ctx.wall_now),
                process_mem: Default::default(),
            }
        }
    }

    #[test]
    fn collector_runs_at_roughly_its_own_cadence() {
        let calls = Arc::new(AtomicU32::new(0));
        let reset_calls = Arc::new(AtomicU32::new(0));
        let collector = CountingCollector {
            calls: Arc::clone(&calls),
            cadence: Duration::from_millis(20),
            reset_calls,
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let visible = Arc::new(AtomicBool::new(true));
        let resume = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(WakeSignal::new());
        let sections = new_shared_sections();

        let worker = WorkerLoop::new(
            vec![Box::new(collector)],
            Arc::clone(&shutdown),
            visible,
            resume,
            wake,
            sections,
        );
        let handle = thread::spawn(move || worker.run());
        thread::sleep(Duration::from_millis(130));
        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        // ~130ms / 20ms cadence ≈ 6-7 calls; generous bounds for CI jitter.
        let n = calls.load(Ordering::Relaxed);
        assert!((4..=10).contains(&n), "expected ~6 calls, got {n}");
    }

    #[test]
    fn a_slow_collector_does_not_starve_a_fast_one_sharing_its_worker() {
        let fast_calls = Arc::new(AtomicU32::new(0));
        let slow_calls = Arc::new(AtomicU32::new(0));
        let fast = CountingCollector {
            calls: Arc::clone(&fast_calls),
            cadence: Duration::from_millis(15),
            reset_calls: Arc::new(AtomicU32::new(0)),
        };
        let slow = SlowCollector {
            stall: Duration::from_millis(60),
            calls: Arc::clone(&slow_calls),
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let visible = Arc::new(AtomicBool::new(true));
        let resume = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(WakeSignal::new());
        let sections = new_shared_sections();

        let worker = WorkerLoop::new(
            vec![Box::new(fast), Box::new(slow)],
            Arc::clone(&shutdown),
            visible,
            resume,
            wake,
            sections,
        );
        let handle = thread::spawn(move || worker.run());
        thread::sleep(Duration::from_millis(400));
        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        // Both still make meaningful progress on one worker thread — this
        // is a coarser guarantee than "never delayed" (they do share a
        // thread), but proves the fast collector isn't starved entirely.
        assert!(fast_calls.load(Ordering::Relaxed) >= 3);
        assert!(slow_calls.load(Ordering::Relaxed) >= 3);
    }

    #[test]
    fn resume_pending_triggers_reset_baseline_exactly_once() {
        let calls = Arc::new(AtomicU32::new(0));
        let reset_calls = Arc::new(AtomicU32::new(0));
        let collector = CountingCollector {
            calls,
            cadence: Duration::from_millis(500),
            reset_calls: Arc::clone(&reset_calls),
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let visible = Arc::new(AtomicBool::new(true));
        let resume = Arc::new(AtomicBool::new(true)); // simulate a pending resume
        let wake = Arc::new(WakeSignal::new());
        let sections = new_shared_sections();

        let worker = WorkerLoop::new(
            vec![Box::new(collector)],
            Arc::clone(&shutdown),
            visible,
            Arc::clone(&resume),
            wake,
            sections,
        );
        let handle = thread::spawn(move || worker.run());
        thread::sleep(Duration::from_millis(50));
        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert!(!resume.load(Ordering::Relaxed));
    }
}
