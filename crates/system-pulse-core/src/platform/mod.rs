//! Platform-specific raw telemetry sources.

mod cpu_times;

pub use cpu_times::{default_source, CpuTimesSource};
