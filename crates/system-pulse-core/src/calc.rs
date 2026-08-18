//! Pure, deterministic calculation helpers.
//!
//! Keeping these as free functions makes the trickiest math directly unit
//! testable without any I/O or platform dependency.

use crate::types::CpuTimes;

/// Compute CPU utilization (0..=100) from two consecutive cumulative tick
/// samples. `total` includes `idle`; utilization is the non-idle fraction of
/// the elapsed ticks.
pub fn compute_cpu_percent(prev: &CpuTimes, curr: &CpuTimes) -> f32 {
    let idle = curr.idle.saturating_sub(prev.idle) as f64;
    let total = curr.total.saturating_sub(prev.total) as f64;
    if total <= 0.0 {
        return 0.0;
    }
    let busy = (total - idle).max(0.0);
    clamp_percent(((busy / total) * 100.0) as f32)
}

/// Compute a throughput in bytes/second from two cumulative byte counters.
/// Returns 0.0 for a non-positive interval (avoids divide-by-zero).
pub fn compute_rate(prev: u64, curr: u64, dt_secs: f64) -> f64 {
    if dt_secs <= 0.0 {
        return 0.0;
    }
    let delta = curr.saturating_sub(prev) as f64;
    (delta / dt_secs).max(0.0)
}

/// Percentage of `part` over `whole`, clamped to 0..=100 and safe for `whole == 0`.
pub fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        return 0.0;
    }
    clamp_percent(((part as f64 / whole as f64) * 100.0) as f32)
}

/// Clamp a percentage value to the sane [0, 100] range.
pub fn clamp_percent(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_full_idle_is_zero() {
        let prev = CpuTimes {
            idle: 100,
            total: 1000,
        };
        let curr = CpuTimes {
            idle: 200,
            total: 1100,
        };
        assert_eq!(compute_cpu_percent(&prev, &curr), 0.0);
    }

    #[test]
    fn cpu_percent_full_busy_is_hundred() {
        let prev = CpuTimes {
            idle: 100,
            total: 1000,
        };
        let curr = CpuTimes {
            idle: 100,
            total: 1100,
        };
        assert_eq!(compute_cpu_percent(&prev, &curr), 100.0);
    }

    #[test]
    fn cpu_percent_half_busy() {
        let prev = CpuTimes { idle: 0, total: 0 };
        let curr = CpuTimes {
            idle: 50,
            total: 100,
        };
        assert!((compute_cpu_percent(&prev, &curr) - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cpu_percent_no_elapsed_time_is_zero() {
        let prev = CpuTimes {
            idle: 10,
            total: 100,
        };
        let curr = prev;
        assert_eq!(compute_cpu_percent(&prev, &curr), 0.0);
    }

    #[test]
    fn cpu_percent_never_negative_on_counter_reset() {
        // A counter reset (curr < prev) must not produce a negative value.
        let prev = CpuTimes {
            idle: 900,
            total: 1000,
        };
        let curr = CpuTimes {
            idle: 10,
            total: 20,
        };
        assert!(compute_cpu_percent(&prev, &curr) >= 0.0);
    }

    #[test]
    fn rate_basic() {
        assert_eq!(compute_rate(0, 1024, 1.0), 1024.0);
        assert_eq!(compute_rate(0, 512, 2.0), 256.0);
    }

    #[test]
    fn rate_zero_interval() {
        assert_eq!(compute_rate(0, 100, 0.0), 0.0);
    }

    #[test]
    fn rate_never_negative_on_reset() {
        assert_eq!(compute_rate(500, 10, 1.0), 0.0);
    }

    #[test]
    fn percent_clamps_and_handles_zero() {
        assert_eq!(percent(0, 0), 0.0);
        assert_eq!(percent(50, 100), 50.0);
        assert_eq!(percent(200, 100), 100.0);
    }

    #[test]
    fn clamp_handles_nan() {
        assert_eq!(clamp_percent(f32::NAN), 0.0);
        assert_eq!(clamp_percent(-5.0), 0.0);
        assert_eq!(clamp_percent(150.0), 100.0);
    }
}
