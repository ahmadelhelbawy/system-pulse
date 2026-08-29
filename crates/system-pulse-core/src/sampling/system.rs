//! One-shot static system/hardware info, independent of the live collector
//! state so it never contends with the sampling threads.
//!
//! The tiered live sampling this module used to own (`SystemSampler`) moved
//! to `crate::collector` (one collector per section) and `crate::scheduler`
//! (the hot/warm thread split that replaces the old single-thread,
//! single-mutex sampler) as part of the Phase 1A rearchitecture.

use crate::scheduler::Scheduler;
use crate::types::SystemInfo;

/// One-shot static system info (used by the probe, IPC's `get_system_info`,
/// and tests).
pub fn static_system_info() -> SystemInfo {
    Scheduler::one_shot_system_info()
}
