//! NVIDIA adapter backed by NVML (dynamically loaded via `nvml-wrapper`).
//!
//! NVML is only available on systems with an NVIDIA driver installed;
//! construction fails cleanly in that case and the caller falls back to the
//! noop provider.

use std::collections::HashMap;

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::enums::device::UsedGpuMemory;
use nvml_wrapper::Nvml;

use super::{GpuProvider, GpuSample};
use crate::types::GpuSnapshot;

pub struct NvidiaGpuProvider {
    nvml: Nvml,
}

impl NvidiaGpuProvider {
    pub fn new() -> Result<Self, String> {
        let nvml = Nvml::init().map_err(|e| e.to_string())?;
        Ok(Self { nvml })
    }
}

impl GpuProvider for NvidiaGpuProvider {
    fn sample(&mut self) -> GpuSample {
        let mut devices = Vec::new();
        let mut process_mem: HashMap<u32, u64> = HashMap::new();

        let count = match self.nvml.device_count() {
            Ok(c) => c,
            Err(_) => return GpuSample::default(),
        };

        for i in 0..count {
            let device = match self.nvml.device_by_index(i) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let name = device.name().ok();
            let utilization_percent = device.utilization_rates().ok().map(|u| u.gpu as f32);
            let mem = device.memory_info().ok();
            let vram_total = mem.as_ref().map(|m| m.total);
            let vram_used = mem.as_ref().map(|m| m.used);
            let temperature_c = device.temperature(TemperatureSensor::Gpu).ok();
            let power_w = device.power_usage().ok().map(|mw| mw as f32 / 1000.0);

            // Accumulate per-process VRAM (graphics + compute processes).
            for p in [
                device.running_graphics_processes(),
                device.running_compute_processes(),
            ]
            .into_iter()
            .flatten()
            .flatten()
            {
                if let UsedGpuMemory::Used(bytes) = p.used_gpu_memory {
                    *process_mem.entry(p.pid).or_insert(0) += bytes;
                }
            }

            devices.push(GpuSnapshot {
                name: name.unwrap_or_else(|| "NVIDIA GPU".to_string()),
                utilization_percent,
                vram_used,
                vram_total,
                temperature_c,
                power_w,
                driver_version: None,
            });
        }

        // Attach the driver version to the first device (it is global).
        if let Some(first) = devices.first_mut() {
            if let Ok(driver) = self.nvml.sys_driver_version() {
                first.driver_version = Some(driver);
            }
        }

        GpuSample {
            devices,
            process_mem,
        }
    }

    fn name(&self) -> &'static str {
        "nvidia"
    }
}
