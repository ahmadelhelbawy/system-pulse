//! System Pulse core: the telemetry engine, domain data contracts, and pure
//! analysis logic shared by the Tauri desktop app and the headless probe.
//!
//! The crate is intentionally free of any GUI / Tauri dependency so that the
//! expensive logic can be unit-tested and exercised headlessly on any host OS.
//!
//! # Safety
//! Native Windows API calls (a single `GetSystemTimes` call for the headline
//! CPU metric) are confined to [`platform::cpu_times`] and wrapped in tiny,
//! commented `unsafe` blocks. Everything else is safe Rust.

#![warn(unsafe_code)]

pub mod calc;
pub mod format;
pub mod gpu;
pub mod health;
pub mod platform;
pub mod process;
pub mod sampling;
pub mod settings;
pub mod types;

pub use settings::{Hotkey, Settings};
pub use types::*;
