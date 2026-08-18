//! Typed error returned across the IPC boundary.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    #[error(transparent)]
    Kill(#[from] system_pulse_core::process::KillError),
}

/// Commands return `Result<T, AppError>`; Tauri requires the error type to be
/// serializable. We serialize the display message only.
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
