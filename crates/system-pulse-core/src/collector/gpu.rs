//! GPU utilization, VRAM, temperature, and power via the [`GpuProvider`]
//! adapter (NVIDIA/NVML today; noop otherwise).
//!
//! Fixes half of the 1.0 "dead collector looks idle" defect: when no NVML
//! provider could be constructed at all (no NVIDIA driver/hardware), this
//! now reports `Availability::Unsupported` once, at `probe()`, instead of
//! every frame silently carrying an empty `Vec<GpuSnapshot>` indistinguishable
//! from "GPU present but currently idle". A transient per-tick NVML call
//! failure (e.g. `device_count()` erroring after a successful `probe()`) is
//! carried forward behaviorally unchanged from 1.0 — `GpuProvider::sample`
//! itself has no way to report *which* call failed, and reworking that is
//! out of scope for a Phase 1A port ("behaviourally unchanged").

use std::time::Duration;

use crate::gpu::{GpuProvider, NoopGpuProvider, NvidiaGpuProvider};
use crate::model::{Availability, Sampled, Source, UnsupportedReason};

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

const GPU_CADENCE: Duration = Duration::from_secs(5);

pub struct GpuCollector {
    provider: Box<dyn GpuProvider>,
    availability: Availability,
}

impl GpuCollector {
    /// Constructs against the best available provider (NVIDIA/NVML, or a
    /// noop that reports `Unsupported` if no such hardware/driver exists).
    pub fn new() -> Self {
        match NvidiaGpuProvider::new() {
            Ok(p) => Self {
                provider: Box::new(p),
                availability: Availability::Ok,
            },
            Err(_) => Self {
                provider: Box::new(NoopGpuProvider),
                availability: Availability::unsupported(UnsupportedReason::DriverAbsent),
            },
        }
    }
}

impl Default for GpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for GpuCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Gpu
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(GPU_CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            // No provider was ever available — don't bother calling into a
            // noop every tick, and don't claim a fresh reading was taken.
            return CollectorOutput::Gpu {
                devices: Sampled::unavailable(
                    self.availability.clone(),
                    Source::Nvml,
                    ctx.wall_now,
                ),
                process_mem: Default::default(),
            };
        }
        let sample = self.provider.sample();
        CollectorOutput::Gpu {
            devices: Sampled::ok(sample.devices, Source::Nvml, ctx.wall_now),
            process_mem: sample.process_mem,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UnixMillis;
    use std::time::Instant;

    fn ctx() -> CollectCtx {
        CollectCtx {
            now: Instant::now(),
            wall_now: UnixMillis(0),
        }
    }

    #[test]
    fn no_provider_reports_unsupported_not_empty_ok() {
        let mut c = GpuCollector {
            provider: Box::new(NoopGpuProvider),
            availability: Availability::unsupported(UnsupportedReason::DriverAbsent),
        };
        c.probe();
        let out = c.collect(&ctx());
        match out {
            CollectorOutput::Gpu { devices, .. } => {
                // The crucial distinction from 1.0: this must NOT be
                // `Ok` with an empty Vec (indistinguishable from "GPU
                // present but idle") — it must say why there's nothing.
                assert!(!devices.availability.is_ok());
                assert_eq!(devices.value, None);
            }
            _ => panic!("expected Gpu output"),
        }
    }
}
