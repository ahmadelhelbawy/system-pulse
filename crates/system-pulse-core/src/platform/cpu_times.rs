//! Raw, cumulative CPU tick counters for the headline CPU metric.
//!
//! Windows reads `GetSystemTimes`; Linux reads `/proc/stat`. Both return
//! counters where `total` includes `idle`, which lets [`crate::calc::compute_cpu_percent`]
//! derive utilization without trusting a single vendor implementation.

use crate::types::CpuTimes;

/// A source of raw cumulative CPU tick counters. Returns `None` when the
/// underlying read fails, so a collector failure is never silently mistaken
/// for an idle system (see `Availability` in `crate::model`).
pub trait CpuTimesSource: Send {
    fn read(&self) -> Option<CpuTimes>;
}

/// Return the platform-appropriate source.
pub fn default_source() -> Box<dyn CpuTimesSource> {
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsCpuTimes)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(LinuxCpuTimes)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Box::new(ZeroCpuTimes)
    }
}

#[cfg(target_os = "windows")]
pub struct WindowsCpuTimes;

#[cfg(target_os = "windows")]
impl CpuTimesSource for WindowsCpuTimes {
    #[allow(unsafe_code)] // single FFI call; arguments are valid out-pointers.
    fn read(&self) -> Option<CpuTimes> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetSystemTimes;

        let mut idle = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };

        // SAFETY: all three out-pointers are valid stack locals.
        let ok = unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) };
        if ok.is_ok() {
            let idle_ticks = ft_to_u64(idle);
            let total_ticks = ft_to_u64(kernel) + ft_to_u64(user);
            Some(CpuTimes {
                idle: idle_ticks,
                total: total_ticks,
            })
        } else {
            None
        }
    }
}

#[cfg(target_os = "windows")]
fn ft_to_u64(ft: windows::Win32::Foundation::FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

#[cfg(target_os = "linux")]
pub struct LinuxCpuTimes;

#[cfg(target_os = "linux")]
impl CpuTimesSource for LinuxCpuTimes {
    fn read(&self) -> Option<CpuTimes> {
        let contents = std::fs::read_to_string("/proc/stat").ok()?;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("cpu ") {
                let fields: Vec<u64> = rest
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                // user nice system idle iowait irq softirq steal ...
                if fields.len() >= 5 {
                    let idle = fields[3] + fields[4];
                    let total: u64 = fields.iter().sum();
                    return Some(CpuTimes { idle, total });
                }
            }
        }
        None
    }
}

/// Fallback for unsupported platforms: no CPU-time source exists.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub struct ZeroCpuTimes;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl CpuTimesSource for ZeroCpuTimes {
    fn read(&self) -> Option<CpuTimes> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingSource;
    impl CpuTimesSource for FailingSource {
        fn read(&self) -> Option<CpuTimes> {
            None
        }
    }

    struct WorkingSource(CpuTimes);
    impl CpuTimesSource for WorkingSource {
        fn read(&self) -> Option<CpuTimes> {
            Some(self.0)
        }
    }

    #[test]
    fn failing_source_reports_none_not_zero_times() {
        // A failed read must be distinguishable from a genuine zero-delta
        // reading — `None`, not `Some(CpuTimes::default())` — so callers can
        // surface `Availability::Failed` instead of a fabricated 0%.
        let source = FailingSource;
        assert_eq!(source.read(), None);
    }

    #[test]
    fn working_source_reports_some() {
        let times = CpuTimes {
            idle: 10,
            total: 20,
        };
        let source = WorkingSource(times);
        assert_eq!(source.read(), Some(times));
    }
}
