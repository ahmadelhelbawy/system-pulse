//! The hot thread: runs `Hot` collectors (CPU, memory) inline every tick,
//! reads the latest warm-tier results, joins GPU per-process memory into
//! the process list, runs health analysis, and publishes the assembled
//! frame. Never blocks on I/O — everything it touches is either in-memory
//! or a collector explicitly documented as sub-millisecond.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::alerts::AlertEngine;
use crate::collector::{CollectCtx, Collector, CollectorOutput, CpuCollector, MemoryCollector};
use crate::health::{analyze, HealthInput};
use crate::history::{HistorySample, HistoryWriter};
use crate::model::{Sampled, UnixMillis};
use crate::settings::{MAX_REFRESH_INTERVAL_MS, MIN_REFRESH_INTERVAL_MS};
use crate::transport::Mailbox;
use crate::types::TelemetrySnapshot;

use super::shared::SharedSections;
use super::wake::WakeSignal;

/// Rolling window of recent total-CPU percentages, feeding `health`'s
/// "sustained CPU" check — ported unchanged from the 1.0 sampler.
const CPU_HISTORY_LEN: usize = 30;

pub(crate) struct HotLoop {
    cpu: CpuCollector,
    memory: MemoryCollector,
    cpu_history: Vec<f32>,
    alert_engine: AlertEngine,
    history: Option<Arc<HistoryWriter>>,
    sections: SharedSections,
    frame_out: Mailbox<TelemetrySnapshot>,
    shutdown: Arc<AtomicBool>,
    visible: Arc<AtomicBool>,
    resume_pending: Arc<AtomicBool>,
    interval_ms: Arc<AtomicU64>,
    wake: Arc<WakeSignal>,
}

impl HotLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cpu: CpuCollector,
        memory: MemoryCollector,
        sections: SharedSections,
        frame_out: Mailbox<TelemetrySnapshot>,
        shutdown: Arc<AtomicBool>,
        visible: Arc<AtomicBool>,
        resume_pending: Arc<AtomicBool>,
        interval_ms: Arc<AtomicU64>,
        wake: Arc<WakeSignal>,
        history: Option<Arc<HistoryWriter>>,
    ) -> Self {
        Self {
            cpu,
            memory,
            cpu_history: Vec::with_capacity(CPU_HISTORY_LEN),
            alert_engine: AlertEngine::new(),
            history,
            sections,
            frame_out,
            shutdown,
            visible,
            resume_pending,
            interval_ms,
            wake,
        }
    }

    pub fn run(mut self) {
        const HIDDEN_POLL: Duration = Duration::from_millis(250);

        self.cpu.probe();
        self.memory.probe();

        while !self.shutdown.load(Ordering::Relaxed) {
            if !self.visible.load(Ordering::Relaxed) {
                self.wake.wait_forever(HIDDEN_POLL);
                continue;
            }

            if self.resume_pending.swap(false, Ordering::AcqRel) {
                self.cpu.reset_baseline();
                self.memory.reset_baseline();
            }

            let tick_start = Instant::now();
            let ctx = CollectCtx {
                now: tick_start,
                wall_now: UnixMillis::now(),
            };

            let cpu = match self.cpu.collect(&ctx) {
                CollectorOutput::Cpu(s) => s,
                _ => unreachable!("CpuCollector always returns CollectorOutput::Cpu"),
            };
            let memory = match self.memory.collect(&ctx) {
                CollectorOutput::Memory(s) => s,
                _ => unreachable!("MemoryCollector always returns CollectorOutput::Memory"),
            };

            self.cpu_history
                .push(cpu.value.as_ref().map(|c| c.total_percent).unwrap_or(0.0));
            if self.cpu_history.len() > CPU_HISTORY_LEN {
                let excess = self.cpu_history.len() - CPU_HISTORY_LEN;
                self.cpu_history.drain(..excess);
            }

            let snapshot = self.assemble(ctx.wall_now, cpu, memory);
            self.record_history(&snapshot);
            self.frame_out.put(snapshot);

            let interval = Duration::from_millis(
                self.interval_ms
                    .load(Ordering::Relaxed)
                    .clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS),
            );
            self.wake.wait_until(tick_start + interval);
        }
    }

    /// Best-effort: absent when no history path was configured, or the
    /// writer's channel is full (in which case `record` itself already
    /// counts the drop — see `history::HistoryWriter`). Never blocks the
    /// hot loop either way.
    fn record_history(&self, snapshot: &TelemetrySnapshot) {
        let Some(writer) = &self.history else {
            return;
        };
        let gpu_percent = snapshot.gpu.value.as_ref().and_then(|gpus| {
            let readings: Vec<f32> = gpus.iter().filter_map(|g| g.utilization_percent).collect();
            if readings.is_empty() {
                None
            } else {
                Some(readings.iter().sum::<f32>() as f64 / readings.len() as f64)
            }
        });
        let net_download_rate = snapshot
            .networks
            .value
            .as_ref()
            .map(|nets| nets.iter().map(|n| n.download_rate).sum());
        let net_upload_rate = snapshot
            .networks
            .value
            .as_ref()
            .map(|nets| nets.iter().map(|n| n.upload_rate).sum());
        writer.record(HistorySample {
            ts_ms: snapshot.timestamp_ms,
            cpu_percent: snapshot.cpu.value.as_ref().map(|c| c.total_percent as f64),
            mem_used_percent: snapshot
                .memory
                .value
                .as_ref()
                .map(|m| m.used_percent as f64),
            gpu_percent,
            disk_read_rate: snapshot.disk_io.value.as_ref().map(|d| d.read_rate),
            disk_write_rate: snapshot.disk_io.value.as_ref().map(|d| d.write_rate),
            net_download_rate,
            net_upload_rate,
        });
    }

    fn assemble(
        &mut self,
        as_of: UnixMillis,
        cpu: Sampled<crate::types::CpuSnapshot>,
        memory: Sampled<crate::types::MemorySnapshot>,
    ) -> TelemetrySnapshot {
        let sections = self.sections.lock();
        let disks = sections.disks.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::Sysinfo,
                as_of,
            )
        });
        let disk_io = sections.disk_io.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::Sysinfo,
                as_of,
            )
        });
        let networks = sections.networks.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::Sysinfo,
                as_of,
            )
        });
        let nvml_gpu = sections.gpu.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::Nvml,
                as_of,
            )
        });
        // Fallback ladder (Phase 1B): NVML is richer (temp/power/VRAM) and
        // stays authoritative whenever it has data; PDH's vendor-neutral
        // device-level utilization only fills in when NVML has none at all
        // (no NVIDIA hardware/driver) — never overwrites a working NVML
        // reading.
        let gpu = if nvml_gpu.availability.is_ok() {
            nvml_gpu
        } else {
            sections.gpu_device_fallback.clone().unwrap_or(nvml_gpu)
        };
        let windows_internal = sections.windows_internal.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::PerfInfo,
                as_of,
            )
        });
        let mut processes = sections.processes.clone().unwrap_or_else(|| {
            Sampled::unavailable(
                crate::model::Availability::failed(crate::model::FailureCode::Timeout),
                crate::model::Source::Sysinfo,
                as_of,
            )
        });
        join_gpu_attribution(
            &mut processes,
            &sections.gpu_process_mem,
            &sections.gpu_process_percent,
        );
        drop(sections);

        let empty_disks = Vec::new();
        let empty_gpu = Vec::new();
        let empty_procs = Vec::new();
        let candidates = analyze(&HealthInput {
            cpu_percent: cpu.value.as_ref().map(|c| c.total_percent).unwrap_or(0.0),
            cpu_history: &self.cpu_history,
            memory_used_percent: memory.value.as_ref().map(|m| m.used_percent).unwrap_or(0.0),
            memory_total: memory.value.as_ref().map(|m| m.total).unwrap_or(0),
            processes: processes.value.as_deref().unwrap_or(&empty_procs),
            disks: disks.value.as_deref().unwrap_or(&empty_disks),
            gpu: gpu.value.as_deref().unwrap_or(&empty_gpu),
        });
        // Score on the debounced alerts, not the raw per-tick candidates —
        // scoring on undebounced input would make the score exactly as
        // flappy as the alerts it's built from claim not to be.
        let stabilized = self.alert_engine.evaluate(candidates);
        let health = crate::analysis::score(&stabilized);

        TelemetrySnapshot {
            timestamp_ms: as_of,
            uptime_secs: sysinfo::System::uptime(),
            cpu,
            memory,
            disk_io,
            disks,
            networks,
            gpu,
            processes,
            windows_internal,
            health,
        }
    }
}

/// Joins the GPU collectors' per-process maps into the process list. Done
/// here, not inside either collector, so they stay independent of one
/// another — see `ProcessCollector`'s module doc. `gpu_mem` (NVML) and
/// `gpu_percent` (PDH) are independent per pid: a process may be in either
/// map, both, or neither. A pid absent from a map is left `None`, never
/// coerced to `Some(0)`.
fn join_gpu_attribution(
    processes: &mut Sampled<Vec<crate::types::ProcessSnapshot>>,
    gpu_process_mem: &std::collections::HashMap<u32, u64>,
    gpu_process_percent: &std::collections::HashMap<u32, f32>,
) {
    if let Some(rows) = processes.value.as_mut() {
        for row in rows.iter_mut() {
            row.gpu_mem = gpu_process_mem.get(&row.pid).copied();
            row.gpu_percent = gpu_process_percent.get(&row.pid).copied();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;
    use crate::types::ProcessSnapshot;
    use std::collections::HashMap;

    fn process(pid: u32) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            name: format!("proc-{pid}"),
            cpu_percent: 0.0,
            memory: 0,
            gpu_mem: None,
            gpu_percent: None,
            exe: None,
            user: None,
            started_at: None,
        }
    }

    #[test]
    fn join_attaches_gpu_memory_and_percent_independently_by_pid() {
        let mut processes = Sampled::ok(
            vec![process(1), process(2), process(3)],
            Source::Sysinfo,
            UnixMillis(0),
        );
        let mut gpu_mem = HashMap::new();
        gpu_mem.insert(2u32, 512_000u64);
        let mut gpu_percent = HashMap::new();
        gpu_percent.insert(3u32, 42.0f32); // different pid than gpu_mem, deliberately

        join_gpu_attribution(&mut processes, &gpu_mem, &gpu_percent);

        let rows = processes.value.unwrap();
        assert_eq!(rows[0].gpu_mem, None);
        assert_eq!(rows[0].gpu_percent, None);
        assert_eq!(rows[1].gpu_mem, Some(512_000));
        assert_eq!(rows[1].gpu_percent, None);
        assert_eq!(rows[2].gpu_mem, None);
        assert_eq!(rows[2].gpu_percent, Some(42.0));
    }

    #[test]
    fn join_is_a_no_op_when_process_data_is_unavailable() {
        let mut processes: Sampled<Vec<ProcessSnapshot>> = Sampled::unavailable(
            crate::model::Availability::failed(crate::model::FailureCode::Timeout),
            Source::Sysinfo,
            UnixMillis(0),
        );
        let mut gpu_mem = HashMap::new();
        gpu_mem.insert(1u32, 1u64);

        join_gpu_attribution(&mut processes, &gpu_mem, &HashMap::new());

        assert_eq!(processes.value, None);
    }

    #[test]
    fn join_leaves_unmatched_pids_as_none_not_zero() {
        let mut processes = Sampled::ok(vec![process(99)], Source::Sysinfo, UnixMillis(0));
        let gpu_mem = HashMap::new(); // no GPU data at all this tick
        join_gpu_attribution(&mut processes, &gpu_mem, &HashMap::new());
        let row = &processes.value.unwrap()[0];
        assert_eq!(row.gpu_mem, None);
        assert_eq!(row.gpu_percent, None);
    }
}
