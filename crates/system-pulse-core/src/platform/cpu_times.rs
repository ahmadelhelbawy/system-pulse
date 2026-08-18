//! Raw, cumulative CPU tick counters for the headline CPU metric.
//!
//! Windows reads `GetSystemTimes`; Linux reads `/proc/stat`. Both return
//! counters where `total` includes `idle`, which lets [`crate::calc::compute_cpu_percent`]
//! derive utilization without trusting a single vendor implementation.

use crate::types::CpuTimes;

/// A source of raw cumulative CPU tick counters.
pub trait CpuTimesSource: Send {
    fn read(&self) -> CpuTimes;
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
    fn read(&self) -> CpuTimes {
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
            CpuTimes {
                idle: idle_ticks,
                total: total_ticks,
            }
        } else {
            CpuTimes::default()
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
    fn read(&self) -> CpuTimes {
        let Ok(contents) = std::fs::read_to_string("/proc/stat") else {
            return CpuTimes::default();
        };
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
                    return CpuTimes { idle, total };
                }
            }
        }
        CpuTimes::default()
    }
}

/// Fallback for unsupported platforms (yields a stable zero).
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub struct ZeroCpuTimes;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl CpuTimesSource for ZeroCpuTimes {
    fn read(&self) -> CpuTimes {
        CpuTimes::default()
    }
}
