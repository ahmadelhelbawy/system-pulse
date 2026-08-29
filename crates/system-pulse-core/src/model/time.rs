//! Wall-clock timestamps that cross a serialization boundary.
//!
//! `UnixMillis` is the *only* timestamp representation allowed on the wire
//! (IPC, history, NDJSON replay, `ts-rs` codegen). Anything measuring
//! elapsed time or driving scheduling must use [`std::time::Instant`]
//! instead and never serialize it — see `crate::scheduler` for that side.
//! Mixing the two is exactly the class of bug behind the disk/network rate
//! defect this phase fixes: a rate must never be derived from wall-clock
//! deltas, because NTP steps, DST, and resume-from-sleep can move the wall
//! clock backwards without the monotonic clock moving at all.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Milliseconds since the Unix epoch. `i64` (not `u64`) so `ts-rs`/JS see an
/// unambiguous signed `number` — magnitude stays far below 2^53, so there is
/// no precision loss serializing to JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(export)]
pub struct UnixMillis(pub i64);

impl UnixMillis {
    /// The current wall-clock time. Falls back to the epoch if the system
    /// clock is set before 1970 (never observed in practice, but this keeps
    /// the constructor infallible rather than panicking).
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        UnixMillis(millis)
    }

    pub fn as_millis(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_positive_and_serializes_as_a_plain_number() {
        let t = UnixMillis::now();
        assert!(t.0 > 0);
        let json = serde_json::to_string(&t).unwrap();
        // `#[serde(transparent)]` means the wire form is a bare number, not
        // `{"0": ...}` — this is what makes it a real `number` in ts-rs/TS.
        assert!(!json.contains('{'));
        let parsed: i64 = json.parse().unwrap();
        assert_eq!(parsed, t.0);
    }

    #[test]
    fn round_trips_through_json() {
        let t = UnixMillis(1_700_000_000_000);
        let json = serde_json::to_string(&t).unwrap();
        let back: UnixMillis = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
