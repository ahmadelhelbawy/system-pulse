//! Public facade over the telemetry engine: `TelemetryService`/`TelemetrySink`
//! for live sampling (backed by `crate::scheduler` + `crate::collector`),
//! plus a one-shot static system-info lookup.

mod service;
mod system;

pub use service::{Backpressure, TelemetryService, TelemetrySink};
pub use system::static_system_info;
