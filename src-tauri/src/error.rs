//! Typed, structured error returned across the IPC boundary.
//!
//! Serializes as `{ "kind": "...", ...fields }` so the frontend can switch
//! on `kind` — e.g. to offer a "relaunch elevated" affordance only on
//! `needsElevation`, or "that process already exited" on
//! `identityMismatch` — instead of substring-matching a human sentence
//! (1.0's `AppError` serialized to a bare display string with no
//! machine-readable discriminant at all).

use serde::Serialize;
use system_pulse_core::model::UnixMillis;
use system_pulse_core::process::KillError;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AppError {
    /// Catch-all for the Windows-integration layer (hotkey registration,
    /// autostart, settings persistence), which itself returns `Result<_,
    /// String>` — see `From<String>` below.
    Message {
        message: String,
    },
    InvalidSettings {
        message: String,
    },
    NotFound {
        pid: u32,
    },
    AccessDenied {
        pid: u32,
    },
    /// The pid exists but is no longer the process the caller expected —
    /// see `system_pulse_core::process::ProcessIdentity`.
    IdentityMismatch {
        pid: u32,
        expected: UnixMillis,
        actual: Option<UnixMillis>,
    },
    // `NeedsElevation { capability }` lands in Phase 4 alongside the first
    // command that can actually produce it (an error variant nothing
    // constructs is dead code, not a useful contract).
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Message { message } => write!(f, "{message}"),
            AppError::InvalidSettings { message } => write!(f, "invalid settings: {message}"),
            AppError::NotFound { pid } => write!(f, "no process with pid {pid}"),
            AppError::AccessDenied { pid } => write!(
                f,
                "access denied terminating pid {pid} (the process may require elevation)"
            ),
            AppError::IdentityMismatch { pid, .. } => write!(
                f,
                "pid {pid} no longer matches the process that was shown (it likely already exited)"
            ),
        }
    }
}

impl std::error::Error for AppError {}

impl From<KillError> for AppError {
    fn from(e: KillError) -> Self {
        match e {
            KillError::NotFound(pid) => AppError::NotFound { pid },
            KillError::AccessDenied(pid) => AppError::AccessDenied { pid },
            KillError::IdentityMismatch {
                pid,
                expected,
                actual,
            } => AppError::IdentityMismatch {
                pid,
                expected,
                actual,
            },
        }
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        AppError::Message { message }
    }
}

impl From<system_pulse_core::history::HistoryError> for AppError {
    fn from(e: system_pulse_core::history::HistoryError) -> Self {
        AppError::Message {
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shape_is_tagged_by_kind() {
        let json = serde_json::to_string(&AppError::NotFound { pid: 42 }).unwrap();
        assert_eq!(json, r#"{"kind":"notFound","pid":42}"#);
    }

    #[test]
    fn identity_mismatch_carries_both_timestamps() {
        let err = AppError::from(KillError::IdentityMismatch {
            pid: 7,
            expected: UnixMillis(1),
            actual: Some(UnixMillis(2)),
        });
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""kind":"identityMismatch""#));
        assert!(json.contains(r#""pid":7"#));
    }
}
