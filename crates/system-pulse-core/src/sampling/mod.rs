//! The telemetry sampling engine: a central owner of system state that
//! refreshes at tiered cadences and emits frames to a sink.

mod service;
mod system;

pub use service::{TelemetryService, TelemetrySink};
pub use system::{static_system_info, SystemSampler};
