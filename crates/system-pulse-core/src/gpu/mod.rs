//! GPU telemetry behind an adapter interface so additional vendors (AMD,
//! Intel) can be added later without touching the sampler or the UI.

mod nvidia;

pub use nvidia::NvidiaGpuProvider;

use std::collections::HashMap;

use crate::types::GpuSnapshot;

/// One GPU sampling result across all adapters.
#[derive(Debug, Default, Clone)]
pub struct GpuSample {
    pub devices: Vec<GpuSnapshot>,
    /// Process id -> total GPU memory (bytes) across devices.
    pub process_mem: HashMap<u32, u64>,
}

/// A source of GPU metrics. Implementations must be cheap to construct and
/// must tolerate the hardware being absent (returning an empty sample).
pub trait GpuProvider: Send {
    fn sample(&mut self) -> GpuSample;
    fn name(&self) -> &'static str;
}

/// Provider used when no GPU adapter is available.
pub struct NoopGpuProvider;

impl GpuProvider for NoopGpuProvider {
    fn sample(&mut self) -> GpuSample {
        GpuSample::default()
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

/// Best-effort default provider: NVIDIA NVML when present, otherwise noop.
pub fn default_gpu_provider() -> Box<dyn GpuProvider> {
    match NvidiaGpuProvider::new() {
        Ok(p) => Box::new(p),
        Err(_) => Box::new(NoopGpuProvider),
    }
}
