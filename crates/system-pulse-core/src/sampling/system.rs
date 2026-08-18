//! Central sampler that owns the `sysinfo` state and derives every metric.
//!
//! Cadence tiers (see [`super::service`]):
//! * cheap (default 1 s): CPU total + per-core, memory
//! * moderate (default 2 s): processes, network, disk I/O
//! * expensive (default 5 s): GPU via the adapter
//! * static: hardware/system info, collected once and cached

use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sysinfo::{CpuRefreshKind, DiskRefreshKind, Disks, Networks, ProcessesToUpdate, System};

use crate::calc::{compute_cpu_percent, compute_rate, percent};
use crate::gpu::{GpuProvider, GpuSample, NoopGpuProvider};
use crate::health::{analyze, HealthInput};
use crate::platform::CpuTimesSource;
use crate::process::{transform_processes, ProcessRow, ProcessSortKey, SortDir};
use crate::types::*;

const PROCESS_LIST_LIMIT: usize = 300;
const CPU_HISTORY_LEN: usize = 30;

pub struct SystemSampler {
    sys: System,
    disks: Disks,
    networks: Networks,
    gpu: Box<dyn GpuProvider>,
    cpu_source: Box<dyn CpuTimesSource>,

    prev_cpu: Option<CpuTimes>,
    prev_net: HashMap<String, (u64, u64)>,
    prev_disk: HashMap<String, (u64, u64)>,
    last_net_ts: Option<Instant>,
    last_disk_ts: Option<Instant>,

    gpu_cache: GpuSample,
    cpu_history: Vec<f32>,
    system_info: SystemInfo,
    primed: bool,
}

impl SystemSampler {
    pub fn new(gpu: Box<dyn GpuProvider>) -> Self {
        Self {
            sys: System::new(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            gpu,
            cpu_source: crate::platform::default_source(),
            prev_cpu: None,
            prev_net: HashMap::new(),
            prev_disk: HashMap::new(),
            last_net_ts: None,
            last_disk_ts: None,
            gpu_cache: GpuSample::default(),
            cpu_history: Vec::with_capacity(CPU_HISTORY_LEN),
            system_info: SystemInfo::default(),
            primed: false,
        }
    }

    /// Cached static hardware/system info (valid after any sample call).
    pub fn system_info(&self) -> &SystemInfo {
        &self.system_info
    }

    fn prime(&mut self) {
        if self.primed {
            return;
        }
        self.sys.refresh_cpu_specifics(CpuRefreshKind::everything());
        self.sys.refresh_memory();
        self.disks
            .refresh_specifics(true, DiskRefreshKind::everything());
        self.networks.refresh(true);

        self.prev_cpu = Some(self.cpu_source.read());
        self.prev_net = self.read_net_totals();
        self.prev_disk = self.read_disk_totals();
        let now = Instant::now();
        self.last_net_ts = Some(now);
        self.last_disk_ts = Some(now);
        self.system_info = self.collect_system_info();
        self.primed = true;
    }

    /// Cheap tier: refresh CPU usage and memory.
    pub fn sample_cheap(&mut self) {
        self.prime();
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
    }

    /// Moderate tier: refresh processes, network, and disk I/O.
    pub fn sample_moderate(&mut self) {
        self.prime();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.networks.refresh(true);
        self.disks
            .refresh_specifics(true, DiskRefreshKind::everything());
        let now = Instant::now();
        self.last_net_ts = Some(now);
        self.last_disk_ts = Some(now);
    }

    /// Expensive tier: refresh GPU metrics.
    pub fn sample_expensive(&mut self) {
        self.prime();
        self.gpu_cache = self.gpu.sample();
    }

    /// Assemble the current frame from the most recent samples.
    pub fn snapshot(&mut self) -> TelemetrySnapshot {
        self.prime();
        let now = Instant::now();

        let curr_cpu = self.cpu_source.read();
        let total_percent = self
            .prev_cpu
            .map(|prev| compute_cpu_percent(&prev, &curr_cpu))
            .unwrap_or(0.0);
        self.prev_cpu = Some(curr_cpu);

        let cpus = self.sys.cpus();
        let per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
        let frequency_mhz = cpus.first().map(|c| c.frequency()).filter(|f| *f > 0);
        let cpu = CpuSnapshot {
            total_percent,
            per_core,
            frequency_mhz,
            core_count: cpus.len(),
        };

        self.cpu_history.push(total_percent);
        if self.cpu_history.len() > CPU_HISTORY_LEN {
            let excess = self.cpu_history.len() - CPU_HISTORY_LEN;
            self.cpu_history.drain(..excess);
        }

        let memory = MemorySnapshot {
            total: self.sys.total_memory(),
            used: self.sys.used_memory(),
            available: self.sys.available_memory(),
            used_percent: percent(self.sys.used_memory(), self.sys.total_memory()),
            swap_total: self.sys.total_swap(),
            swap_used: self.sys.used_swap(),
        };

        let (disks, disk_io) = self.build_disks(now);
        let networks = self.build_networks(now);
        let processes = self.build_processes();
        let gpu = self.gpu_cache.devices.clone();

        let health = analyze(&HealthInput {
            cpu_percent: total_percent,
            cpu_history: &self.cpu_history,
            memory_used_percent: memory.used_percent,
            memory_total: memory.total,
            processes: &processes,
            disks: &disks,
            gpu: &gpu,
        });

        TelemetrySnapshot {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            uptime_secs: System::uptime(),
            cpu,
            memory,
            disk_io,
            disks,
            networks,
            gpu,
            processes,
            health,
        }
    }

    fn read_net_totals(&self) -> HashMap<String, (u64, u64)> {
        self.networks
            .list()
            .iter()
            .map(|(name, data)| {
                (
                    name.clone(),
                    (data.total_received(), data.total_transmitted()),
                )
            })
            .collect()
    }

    fn read_disk_totals(&self) -> HashMap<String, (u64, u64)> {
        self.disks
            .list()
            .iter()
            .map(|d| {
                let u = d.usage();
                (
                    d.name().to_string_lossy().into_owned(),
                    (u.total_read_bytes, u.total_written_bytes),
                )
            })
            .collect()
    }

    fn build_disks(&mut self, now: Instant) -> (Vec<DiskSnapshot>, DiskIoSnapshot) {
        let dt = self
            .last_disk_ts
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let curr = self.read_disk_totals();
        let mut list = Vec::new();
        let mut total_read = 0u64;
        let mut total_write = 0u64;
        let mut total_read_rate = 0f64;
        let mut total_write_rate = 0f64;

        for disk in self.disks.list() {
            let key = disk.name().to_string_lossy().into_owned();
            let (read, write) = curr.get(&key).copied().unwrap_or((0, 0));
            let (prev_read, prev_write) =
                self.prev_disk.get(&key).copied().unwrap_or((read, write));
            let read_rate = compute_rate(prev_read, read, dt);
            let write_rate = compute_rate(prev_write, write, dt);

            total_read += read;
            total_write += write;
            total_read_rate += read_rate;
            total_write_rate += write_rate;

            let used = disk.total_space().saturating_sub(disk.available_space());
            list.push(DiskSnapshot {
                name: key,
                mount_point: disk.mount_point().to_string_lossy().into_owned(),
                file_system: disk.file_system().to_string_lossy().into_owned(),
                total: disk.total_space(),
                available: disk.available_space(),
                used_percent: percent(used, disk.total_space()),
                read_rate,
                write_rate,
                is_removable: disk.is_removable(),
            });
        }

        self.prev_disk = curr;
        (
            list,
            DiskIoSnapshot {
                read_rate: total_read_rate,
                write_rate: total_write_rate,
                total_read,
                total_write,
            },
        )
    }

    fn build_networks(&mut self, now: Instant) -> Vec<NetworkSnapshot> {
        let dt = self
            .last_net_ts
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let curr = self.read_net_totals();
        let mut out = Vec::new();

        for (name, data) in self.networks.list() {
            let (rx, tx) = (data.total_received(), data.total_transmitted());
            let (prev_rx, prev_tx) = self.prev_net.get(name).copied().unwrap_or((rx, tx));
            out.push(NetworkSnapshot {
                name: name.clone(),
                download_rate: compute_rate(prev_rx, rx, dt),
                upload_rate: compute_rate(prev_tx, tx, dt),
                total_rx: rx,
                total_tx: tx,
            });
        }
        // Stable ordering for deterministic UI and snapshots.
        out.sort_by(|a, b| a.name.cmp(&b.name));

        self.prev_net = curr;
        out
    }

    fn build_processes(&mut self) -> Vec<ProcessSnapshot> {
        let rows: Vec<ProcessRow> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcessRow {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: p.cpu_usage(),
                memory: p.memory(),
                gpu_mem: self.gpu_cache.process_mem.get(&pid.as_u32()).copied(),
                exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
                user: p.user_id().map(uid_to_string),
            })
            .collect();
        transform_processes(
            rows,
            ProcessSortKey::Cpu,
            SortDir::Desc,
            None,
            PROCESS_LIST_LIMIT,
        )
    }

    fn collect_system_info(&self) -> SystemInfo {
        let cpu_model = self
            .sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        SystemInfo {
            os_name: System::name().unwrap_or_else(|| "Unknown OS".to_string()),
            os_version: System::long_os_version().unwrap_or_else(|| "Unknown".to_string()),
            kernel_version: System::kernel_version().unwrap_or_default(),
            hostname: System::host_name().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_model,
            cpu_cores: self.sys.cpus().len(),
            total_memory: self.sys.total_memory(),
        }
    }
}

/// One-shot static system info (used by the probe and by tests).
pub fn static_system_info() -> SystemInfo {
    let mut sampler = SystemSampler::new(Box::new(NoopGpuProvider));
    sampler.prime();
    sampler.system_info.clone()
}

fn uid_to_string(uid: &sysinfo::Uid) -> String {
    // `Uid` has no Display; use Debug and strip the wrapper: `Uid(1000)` -> `1000`.
    let s = format!("{uid:?}");
    s.strip_prefix("Uid(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(&s)
        .to_string()
}
