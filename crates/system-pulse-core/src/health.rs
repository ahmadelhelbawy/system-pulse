//! Deterministic, local-only health analysis.
//!
//! The analyzer is a pure function over a telemetry frame: no history state,
//! no network, no ML. "Sustained" CPU is approximated by passing a small
//! rolling window of recent total-CPU samples in `cpu_history`.

use crate::types::{DiskSnapshot, GpuSnapshot, HealthAlert, ProcessSnapshot, Severity};

/// Thresholds are plain constants so they are easy to review and tune.
pub const SUSTAINED_CPU_SAMPLES: usize = 5;
const CPU_SUSTAINED_CRITICAL: f32 = 95.0;
const CPU_SUSTAINED_WARNING: f32 = 85.0;

const MEM_CRITICAL: f32 = 95.0;
const MEM_WARNING: f32 = 85.0;

const PROCESS_MEM_CRITICAL_FRAC: f32 = 0.50;
const PROCESS_MEM_WARNING_FRAC: f32 = 0.25;
const PROCESS_CPU_FULL_CORE: f32 = 100.0;

const DISK_FULL_CRITICAL: f32 = 95.0;
const DISK_FULL_WARNING: f32 = 85.0;
const DISK_ACTIVITY_BYTES_PER_SEC: f64 = 100.0 * 1024.0 * 1024.0; // 100 MB/s

const GPU_UTIL_CRITICAL: f32 = 95.0;
const GPU_UTIL_WARNING: f32 = 85.0;
const VRAM_CRITICAL: f32 = 95.0;
const VRAM_WARNING: f32 = 85.0;
const GPU_TEMP_CRITICAL: u32 = 90;
const GPU_TEMP_WARNING: u32 = 83;

pub struct HealthInput<'a> {
    pub cpu_percent: f32,
    pub cpu_history: &'a [f32],
    pub memory_used_percent: f32,
    pub memory_total: u64,
    pub processes: &'a [ProcessSnapshot],
    pub disks: &'a [DiskSnapshot],
    pub gpu: &'a [GpuSnapshot],
}

pub fn analyze(input: &HealthInput) -> Vec<HealthAlert> {
    let mut alerts = Vec::new();
    analyze_memory(input, &mut alerts);
    analyze_cpu(input, &mut alerts);
    analyze_processes(input, &mut alerts);
    analyze_disks(input, &mut alerts);
    analyze_gpu(input, &mut alerts);
    // Most severe first, stable within a severity.
    alerts.sort_by_key(|a| severity_rank(a.severity));
    alerts
}

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

fn analyze_memory(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    if input.memory_used_percent >= MEM_CRITICAL {
        alerts.push(HealthAlert {
            severity: Severity::Critical,
            category: "memory".into(),
            title: "Memory critically low".into(),
            detail: format!(
                "{:.0}% of physical memory is in use",
                input.memory_used_percent
            ),
            pid: None,
        });
    } else if input.memory_used_percent >= MEM_WARNING {
        alerts.push(HealthAlert {
            severity: Severity::Warning,
            category: "memory".into(),
            title: "Memory usage high".into(),
            detail: format!(
                "{:.0}% of physical memory is in use",
                input.memory_used_percent
            ),
            pid: None,
        });
    }
}

fn analyze_cpu(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    let recent: Vec<f32> = input
        .cpu_history
        .iter()
        .copied()
        .rev()
        .take(SUSTAINED_CPU_SAMPLES)
        .collect();
    let mean = if recent.is_empty() {
        input.cpu_percent
    } else {
        recent.iter().sum::<f32>() / recent.len() as f32
    };

    if mean >= CPU_SUSTAINED_CRITICAL {
        alerts.push(HealthAlert {
            severity: Severity::Critical,
            category: "cpu".into(),
            title: "Sustained CPU saturation".into(),
            detail: format!("CPU has averaged {mean:.0}% over the last few samples"),
            pid: None,
        });
    } else if mean >= CPU_SUSTAINED_WARNING {
        alerts.push(HealthAlert {
            severity: Severity::Warning,
            category: "cpu".into(),
            title: "Sustained high CPU".into(),
            detail: format!("CPU has averaged {mean:.0}% over the last few samples"),
            pid: None,
        });
    }
}

fn analyze_processes(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    if input.memory_total == 0 {
        return;
    }
    for p in input.processes {
        let mem_frac = p.memory as f32 / input.memory_total as f32;
        if mem_frac >= PROCESS_MEM_CRITICAL_FRAC {
            alerts.push(HealthAlert {
                severity: Severity::Critical,
                category: "process".into(),
                title: format!("{} is using a lot of memory", p.name),
                detail: format!(
                    "{} is using {:.0}% of physical memory",
                    p.name,
                    mem_frac * 100.0
                ),
                pid: Some(p.pid),
            });
        } else if mem_frac >= PROCESS_MEM_WARNING_FRAC {
            alerts.push(HealthAlert {
                severity: Severity::Warning,
                category: "process".into(),
                title: format!("{} is using a lot of memory", p.name),
                detail: format!(
                    "{} is using {:.0}% of physical memory",
                    p.name,
                    mem_frac * 100.0
                ),
                pid: Some(p.pid),
            });
        }

        if p.cpu_percent >= PROCESS_CPU_FULL_CORE {
            alerts.push(HealthAlert {
                severity: Severity::Warning,
                category: "process".into(),
                title: format!("{} is CPU-heavy", p.name),
                detail: format!(
                    "{} is using {:.0}% CPU (≥ one full core)",
                    p.name, p.cpu_percent
                ),
                pid: Some(p.pid),
            });
        }
    }
}

fn analyze_disks(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    let mut total_io = 0.0_f64;
    for d in input.disks {
        total_io += d.read_rate + d.write_rate;
        if d.used_percent >= DISK_FULL_CRITICAL {
            alerts.push(HealthAlert {
                severity: Severity::Critical,
                category: "disk".into(),
                title: format!("Disk {} is nearly full", d.name),
                detail: format!("{:.0}% of {} is used", d.used_percent, d.name),
                pid: None,
            });
        } else if d.used_percent >= DISK_FULL_WARNING {
            alerts.push(HealthAlert {
                severity: Severity::Warning,
                category: "disk".into(),
                title: format!("Disk {} is filling up", d.name),
                detail: format!("{:.0}% of {} is used", d.used_percent, d.name),
                pid: None,
            });
        }
    }
    if total_io >= DISK_ACTIVITY_BYTES_PER_SEC {
        alerts.push(HealthAlert {
            severity: Severity::Info,
            category: "disk".into(),
            title: "High disk activity".into(),
            detail: format!(
                "Aggregate disk throughput is {:.0} MB/s",
                total_io / (1024.0 * 1024.0)
            ),
            pid: None,
        });
    }
}

fn analyze_gpu(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    for g in input.gpu {
        if let Some(util) = g.utilization_percent {
            if util >= GPU_UTIL_CRITICAL {
                alerts.push(HealthAlert {
                    severity: Severity::Critical,
                    category: "gpu".into(),
                    title: format!("GPU {} is saturated", g.name),
                    detail: format!("{:.0}% GPU utilization", util),
                    pid: None,
                });
            } else if util >= GPU_UTIL_WARNING {
                alerts.push(HealthAlert {
                    severity: Severity::Warning,
                    category: "gpu".into(),
                    title: format!("GPU {} utilization high", g.name),
                    detail: format!("{:.0}% GPU utilization", util),
                    pid: None,
                });
            }
        }

        if let (Some(used), Some(total)) = (g.vram_used, g.vram_total) {
            let frac = if total == 0 {
                0.0
            } else {
                used as f32 / total as f32 * 100.0
            };
            if frac >= VRAM_CRITICAL {
                alerts.push(HealthAlert {
                    severity: Severity::Critical,
                    category: "gpu".into(),
                    title: format!("GPU {} VRAM nearly exhausted", g.name),
                    detail: format!("{:.0}% VRAM in use", frac),
                    pid: None,
                });
            } else if frac >= VRAM_WARNING {
                alerts.push(HealthAlert {
                    severity: Severity::Warning,
                    category: "gpu".into(),
                    title: format!("GPU {} VRAM usage high", g.name),
                    detail: format!("{:.0}% VRAM in use", frac),
                    pid: None,
                });
            }
        }

        if let Some(temp) = g.temperature_c {
            if temp >= GPU_TEMP_CRITICAL {
                alerts.push(HealthAlert {
                    severity: Severity::Critical,
                    category: "gpu".into(),
                    title: format!("GPU {} is very hot", g.name),
                    detail: format!("{temp}°C"),
                    pid: None,
                });
            } else if temp >= GPU_TEMP_WARNING {
                alerts.push(HealthAlert {
                    severity: Severity::Warning,
                    category: "gpu".into(),
                    title: format!("GPU {} is warm", g.name),
                    detail: format!("{temp}°C"),
                    pid: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GpuSnapshot;

    fn disk(used_percent: f32, read: f64, write: f64) -> DiskSnapshot {
        DiskSnapshot {
            name: "C:".into(),
            mount_point: "C:\\".into(),
            file_system: "NTFS".into(),
            total: 1_000_000_000,
            available: 1,
            used_percent,
            read_rate: read,
            write_rate: write,
            is_removable: false,
        }
    }

    #[test]
    fn memory_saturation_is_critical() {
        let input = HealthInput {
            cpu_percent: 5.0,
            cpu_history: &[],
            memory_used_percent: 97.0,
            memory_total: 16 * 1024 * 1024 * 1024,
            processes: &[],
            disks: &[],
            gpu: &[],
        };
        let alerts = analyze(&input);
        assert!(alerts
            .iter()
            .any(|a| a.severity == Severity::Critical && a.category == "memory"));
    }

    #[test]
    fn sustained_cpu_uses_history_mean() {
        let history = [90.0, 92.0, 91.0, 93.0, 90.0];
        let input = HealthInput {
            cpu_percent: 50.0,
            cpu_history: &history,
            memory_used_percent: 30.0,
            memory_total: 1000,
            processes: &[],
            disks: &[],
            gpu: &[],
        };
        let alerts = analyze(&input);
        assert!(alerts
            .iter()
            .any(|a| a.category == "cpu" && a.severity == Severity::Warning));
    }

    #[test]
    fn memory_hungry_process_flagged_with_pid() {
        let processes = [ProcessSnapshot {
            pid: 4242,
            name: "bigapp".into(),
            cpu_percent: 2.0,
            memory: 600,
            gpu_mem: None,
            gpu_percent: None,
            exe: None,
            user: None,
            started_at: None,
        }];
        let input = HealthInput {
            cpu_percent: 0.0,
            cpu_history: &[],
            memory_used_percent: 10.0,
            memory_total: 1000,
            processes: &processes,
            disks: &[],
            gpu: &[],
        };
        let alerts = analyze(&input);
        assert!(alerts
            .iter()
            .any(|a| a.category == "process" && a.pid == Some(4242)));
    }

    #[test]
    fn disk_full_and_gpu_hot_are_reported() {
        let disks = [disk(96.0, 0.0, 0.0)];
        let gpu = [GpuSnapshot {
            name: "RTX 4090".into(),
            utilization_percent: Some(50.0),
            vram_used: Some(1),
            vram_total: Some(100),
            temperature_c: Some(95),
            power_w: Some(200.0),
            driver_version: None,
        }];
        let input = HealthInput {
            cpu_percent: 0.0,
            cpu_history: &[],
            memory_used_percent: 10.0,
            memory_total: 1000,
            processes: &[],
            disks: &disks,
            gpu: &gpu,
        };
        let alerts = analyze(&input);
        assert!(alerts.iter().any(|a| a.category == "disk"));
        assert!(alerts.iter().any(|a| a.category == "gpu"));
    }

    #[test]
    fn quiet_system_has_no_alerts() {
        let input = HealthInput {
            cpu_percent: 5.0,
            cpu_history: &[5.0, 6.0, 4.0, 5.0, 5.0],
            memory_used_percent: 30.0,
            memory_total: 16 * 1024 * 1024 * 1024,
            processes: &[],
            disks: &[disk(40.0, 0.0, 0.0)],
            gpu: &[],
        };
        assert!(analyze(&input).is_empty());
    }
}
