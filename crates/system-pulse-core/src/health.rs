//! Deterministic, local-only health analysis.
//!
//! The analyzer is a pure function over a telemetry frame: no history state,
//! no network, no ML. "Sustained" CPU is approximated by passing a small
//! rolling window of recent total-CPU samples in `cpu_history`. Alert
//! *hysteresis* (debouncing raise/clear across ticks) is a separate
//! concern, layered on top by `crate::alerts::AlertEngine` — this module
//! only ever answers "is this condition true right now."

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

/// Constructs an alert and its stable `id` in one place — `category` +
/// `title` + `pid` is the identity `crate::alerts::AlertEngine` debounces
/// on, so every call site building a `HealthAlert` goes through here
/// rather than the struct literal directly, which would make it easy for a
/// new call site to forget the id (or compute it inconsistently). A
/// severity change between two thresholds of the *same* check (e.g.
/// memory warning → critical) intentionally has a different `title` and
/// therefore a different id: it resets that alert's hysteresis rather than
/// instantly reflecting the new severity, which is the conservative,
/// flap-resistant choice for a threshold crossing rather than a
/// continuously-updating measurement.
fn push_alert(
    alerts: &mut Vec<HealthAlert>,
    severity: Severity,
    category: &str,
    title: String,
    detail: String,
    pid: Option<u32>,
) {
    let id = match pid {
        Some(pid) => format!("{category}:{title}:{pid}"),
        None => format!("{category}:{title}"),
    };
    alerts.push(HealthAlert {
        id,
        severity,
        category: category.to_string(),
        title,
        detail,
        pid,
    });
}

fn analyze_memory(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    let detail = format!(
        "{:.0}% of physical memory is in use",
        input.memory_used_percent
    );
    if input.memory_used_percent >= MEM_CRITICAL {
        push_alert(
            alerts,
            Severity::Critical,
            "memory",
            "Memory critically low".into(),
            detail,
            None,
        );
    } else if input.memory_used_percent >= MEM_WARNING {
        push_alert(
            alerts,
            Severity::Warning,
            "memory",
            "Memory usage high".into(),
            detail,
            None,
        );
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
    let detail = format!("CPU has averaged {mean:.0}% over the last few samples");

    if mean >= CPU_SUSTAINED_CRITICAL {
        push_alert(
            alerts,
            Severity::Critical,
            "cpu",
            "Sustained CPU saturation".into(),
            detail,
            None,
        );
    } else if mean >= CPU_SUSTAINED_WARNING {
        push_alert(
            alerts,
            Severity::Warning,
            "cpu",
            "Sustained high CPU".into(),
            detail,
            None,
        );
    }
}

fn analyze_processes(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    if input.memory_total == 0 {
        return;
    }
    for p in input.processes {
        let mem_frac = p.memory as f32 / input.memory_total as f32;
        let mem_detail = format!(
            "{} is using {:.0}% of physical memory",
            p.name,
            mem_frac * 100.0
        );
        if mem_frac >= PROCESS_MEM_CRITICAL_FRAC {
            push_alert(
                alerts,
                Severity::Critical,
                "process",
                format!("{} is using a lot of memory", p.name),
                mem_detail,
                Some(p.pid),
            );
        } else if mem_frac >= PROCESS_MEM_WARNING_FRAC {
            push_alert(
                alerts,
                Severity::Warning,
                "process",
                format!("{} is using a lot of memory", p.name),
                mem_detail,
                Some(p.pid),
            );
        }

        if p.cpu_percent >= PROCESS_CPU_FULL_CORE {
            push_alert(
                alerts,
                Severity::Warning,
                "process",
                format!("{} is CPU-heavy", p.name),
                format!(
                    "{} is using {:.0}% CPU (\u{2265} one full core)",
                    p.name, p.cpu_percent
                ),
                Some(p.pid),
            );
        }
    }
}

fn analyze_disks(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    let mut total_io = 0.0_f64;
    for d in input.disks {
        total_io += d.read_rate + d.write_rate;
        let detail = format!("{:.0}% of {} is used", d.used_percent, d.name);
        if d.used_percent >= DISK_FULL_CRITICAL {
            push_alert(
                alerts,
                Severity::Critical,
                "disk",
                format!("Disk {} is nearly full", d.name),
                detail,
                None,
            );
        } else if d.used_percent >= DISK_FULL_WARNING {
            push_alert(
                alerts,
                Severity::Warning,
                "disk",
                format!("Disk {} is filling up", d.name),
                detail,
                None,
            );
        }
    }
    if total_io >= DISK_ACTIVITY_BYTES_PER_SEC {
        push_alert(
            alerts,
            Severity::Info,
            "disk",
            "High disk activity".into(),
            format!(
                "Aggregate disk throughput is {:.0} MB/s",
                total_io / (1024.0 * 1024.0)
            ),
            None,
        );
    }
}

fn analyze_gpu(input: &HealthInput, alerts: &mut Vec<HealthAlert>) {
    for g in input.gpu {
        if let Some(util) = g.utilization_percent {
            let detail = format!("{util:.0}% GPU utilization");
            if util >= GPU_UTIL_CRITICAL {
                push_alert(
                    alerts,
                    Severity::Critical,
                    "gpu",
                    format!("GPU {} is saturated", g.name),
                    detail,
                    None,
                );
            } else if util >= GPU_UTIL_WARNING {
                push_alert(
                    alerts,
                    Severity::Warning,
                    "gpu",
                    format!("GPU {} utilization high", g.name),
                    detail,
                    None,
                );
            }
        }

        if let (Some(used), Some(total)) = (g.vram_used, g.vram_total) {
            let frac = if total == 0 {
                0.0
            } else {
                used as f32 / total as f32 * 100.0
            };
            let detail = format!("{frac:.0}% VRAM in use");
            if frac >= VRAM_CRITICAL {
                push_alert(
                    alerts,
                    Severity::Critical,
                    "gpu",
                    format!("GPU {} VRAM nearly exhausted", g.name),
                    detail,
                    None,
                );
            } else if frac >= VRAM_WARNING {
                push_alert(
                    alerts,
                    Severity::Warning,
                    "gpu",
                    format!("GPU {} VRAM usage high", g.name),
                    detail,
                    None,
                );
            }
        }

        if let Some(temp) = g.temperature_c {
            let detail = format!("{temp}\u{b0}C");
            if temp >= GPU_TEMP_CRITICAL {
                push_alert(
                    alerts,
                    Severity::Critical,
                    "gpu",
                    format!("GPU {} is very hot", g.name),
                    detail,
                    None,
                );
            } else if temp >= GPU_TEMP_WARNING {
                push_alert(
                    alerts,
                    Severity::Warning,
                    "gpu",
                    format!("GPU {} is warm", g.name),
                    detail,
                    None,
                );
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

    #[test]
    fn two_distinct_alerts_for_the_same_process_get_distinct_ids() {
        // A process that's both memory-hungry and CPU-heavy must produce
        // two separate alerts, not one clobbering the other — this is
        // exactly the collision `push_alert`'s id scheme (category+title
        // +pid, not just category+pid) exists to avoid.
        let processes = [ProcessSnapshot {
            pid: 99,
            name: "hog".into(),
            cpu_percent: 150.0,
            memory: 900,
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
        let process_alerts: Vec<_> = alerts.iter().filter(|a| a.pid == Some(99)).collect();
        assert_eq!(process_alerts.len(), 2);
        assert_ne!(process_alerts[0].id, process_alerts[1].id);
    }
}
