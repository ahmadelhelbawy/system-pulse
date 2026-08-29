//! State the warm-tier worker threads publish into and the hot thread reads
//! from when assembling a frame.
//!
//! This is intentionally one small mutex guarding a handful of `Option`s
//! that are swapped in whole, not held during any actual collection work —
//! collectors run entirely outside the lock, so contention here is a memory
//! copy, never an I/O wait. That's what actually eliminates the 1.0 defect
//! (a single mutex held for the *duration of sampling*, which is what let a
//! slow collector stall the whole engine) — splitting this one further into
//! five separate mutexes would not change that property, only the code
//! shape, so it isn't done here.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::model::Sampled;
use crate::types::{
    ConnectionSnapshot, DiskIoSnapshot, DiskSnapshot, GpuSnapshot, NetworkSnapshot,
    ProcessSnapshot, SmbiosInfo, WindowsInternalState,
};

#[derive(Default)]
pub(crate) struct LatestSections {
    pub disks: Option<Sampled<Vec<DiskSnapshot>>>,
    pub disk_io: Option<Sampled<DiskIoSnapshot>>,
    pub networks: Option<Sampled<Vec<NetworkSnapshot>>>,
    pub gpu: Option<Sampled<Vec<GpuSnapshot>>>,
    pub gpu_process_mem: HashMap<u32, u64>,
    pub processes: Option<Sampled<Vec<ProcessSnapshot>>>,
    // --- Phase 1B ---
    pub windows_internal: Option<Sampled<WindowsInternalState>>,
    /// Read on demand by `get_connections`, never folded into the hot
    /// frame — see `CollectorOutput::Connections`.
    pub connections: Option<Sampled<Vec<ConnectionSnapshot>>>,
    /// Read on demand by `get_hardware_info`, never folded into the hot
    /// frame — see `CollectorOutput::Hardware`.
    pub hardware: Option<Sampled<SmbiosInfo>>,
    /// pid -> GPU engine utilization percent, from PDH. Parallel to
    /// `gpu_process_mem` (which comes from NVML); a pid may appear in
    /// either map, both, or neither.
    pub gpu_process_percent: HashMap<u32, f32>,
    /// Device-level GPU stats from PDH, used only when NVML (`gpu` above)
    /// is unavailable — see `CollectorOutput::PdhGpu`'s doc comment.
    pub gpu_device_fallback: Option<Sampled<Vec<GpuSnapshot>>>,
}

pub(crate) type SharedSections = std::sync::Arc<Mutex<LatestSections>>;

pub(crate) fn new_shared_sections() -> SharedSections {
    std::sync::Arc::new(Mutex::new(LatestSections::default()))
}
