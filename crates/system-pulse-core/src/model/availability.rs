//! Provenance for every collected value: where it came from and, when it
//! isn't there, *why not*.
//!
//! This is what makes the project's "no mock data" principle a property of
//! the type system instead of a matter of discipline: a collector can never
//! silently substitute a plausible-looking zero for a value it failed to
//! read. Every variant here is enum-backed and fully owned (no `&'static
//! str`) so it survives all four round trips a value can take: IPC to the
//! frontend, a future history/SQLite write, `ts-rs` codegen, and NDJSON
//! capture-and-replay in tests.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::UnixMillis;

/// Which subsystem produced a value. Closed enum so the wire form is a
/// stable string union, not an open `String` a typo can silently diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Source {
    GetSystemTimes,
    ProcStat,
    Sysinfo,
    Nvml,
    Pdh,
    Smbios,
    IpHelper,
    PerfInfo,
    Registry,
    Wmi,
    EventLog,
    StorageIoctl,
    SensorBridge,
}

/// Why a value is unsupported on this machine (as opposed to merely having
/// failed this one read — see [`FailureCode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnsupportedReason {
    NoSuchHardware,
    VendorUnsupported,
    DriverAbsent,
    OsTooOld,
    CounterMissing,
    NotImplementedOnPlatform,
}

/// Why a collector's read failed this time (transient, as opposed to
/// [`UnsupportedReason`], which is permanent for this machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FailureCode {
    Timeout,
    AccessDenied,
    ApiError,
    ParseError,
    Cancelled,
}

/// The full provenance state of a [`Sampled`] value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum Availability {
    Ok,
    #[serde(rename_all = "camelCase")]
    Unsupported {
        reason: UnsupportedReason,
    },
    NeedsElevation,
    /// The collector ran and errored. `detail` is owned, diagnostic-only
    /// text — the frontend must switch on `code`, never parse `detail`.
    #[serde(rename_all = "camelCase")]
    Failed {
        code: FailureCode,
        detail: Option<String>,
    },
    /// Previously `Ok`, now failing — the carried value (if any) is the
    /// last good reading, not a fresh one.
    ///
    /// A container-level `rename_all` on an enum only renames *variant*
    /// names, not the fields of struct-like variants (verified empirically
    /// against this crate's serde version — `last_error` serialized as
    /// exactly that, not `lastError`, until this per-variant attribute was
    /// added) — every multi-word-field variant needs its own
    /// `rename_all`, not just the enum.
    #[serde(rename_all = "camelCase")]
    Stale {
        since: UnixMillis,
        last_error: Option<FailureCode>,
    },
}

impl Availability {
    pub fn is_ok(&self) -> bool {
        matches!(self, Availability::Ok)
    }

    pub fn failed(code: FailureCode) -> Self {
        Availability::Failed { code, detail: None }
    }

    pub fn failed_with_detail(code: FailureCode, detail: impl Into<String>) -> Self {
        Availability::Failed {
            code,
            detail: Some(detail.into()),
        }
    }

    pub fn unsupported(reason: UnsupportedReason) -> Self {
        Availability::Unsupported { reason }
    }
}

/// A value together with where it came from and whether it's actually there.
///
/// `value` and `availability` are independent on purpose: `Stale` carries
/// the last good `value` (not `None`), so the UI can show "last known: 42%,
/// 12s ago" instead of blanking out on a single missed tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sampled<T> {
    pub value: Option<T>,
    pub availability: Availability,
    pub source: Source,
    pub as_of: UnixMillis,
}

impl<T> Sampled<T> {
    pub fn ok(value: T, source: Source, as_of: UnixMillis) -> Self {
        Sampled {
            value: Some(value),
            availability: Availability::Ok,
            source,
            as_of,
        }
    }

    pub fn unavailable(availability: Availability, source: Source, as_of: UnixMillis) -> Self {
        debug_assert!(
            !availability.is_ok(),
            "Sampled::unavailable called with Availability::Ok; use Sampled::ok"
        );
        Sampled {
            value: None,
            availability,
            source,
            as_of,
        }
    }

    /// Carries forward a last-known-good `value` while marking it stale —
    /// the shape `Availability::Stale` exists for.
    pub fn stale(
        value: T,
        since: UnixMillis,
        last_error: Option<FailureCode>,
        source: Source,
        as_of: UnixMillis,
    ) -> Self {
        Sampled {
            value: Some(value),
            availability: Availability::Stale { since, last_error },
            source,
            as_of,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Sampled<U> {
        Sampled {
            value: self.value.map(f),
            availability: self.availability,
            source: self.source,
            as_of: self.as_of,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_round_trips_through_json() {
        let s = Sampled::ok(42u32, Source::Sysinfo, UnixMillis(1000));
        let json = serde_json::to_string(&s).unwrap();
        let back: Sampled<u32> = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn every_availability_variant_round_trips() {
        let variants = vec![
            Availability::Ok,
            Availability::Unsupported {
                reason: UnsupportedReason::DriverAbsent,
            },
            Availability::NeedsElevation,
            Availability::Failed {
                code: FailureCode::Timeout,
                detail: Some("took too long".to_string()),
            },
            Availability::Failed {
                code: FailureCode::ApiError,
                detail: None,
            },
            Availability::Stale {
                since: UnixMillis(500),
                last_error: Some(FailureCode::AccessDenied),
            },
        ];
        for a in variants {
            let json = serde_json::to_string(&a).unwrap();
            let back: Availability = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back, "round trip failed for {json}");
        }
    }

    #[test]
    fn unavailable_never_carries_a_value() {
        let s: Sampled<u32> = Sampled::unavailable(
            Availability::unsupported(UnsupportedReason::NoSuchHardware),
            Source::Nvml,
            UnixMillis(0),
        );
        assert_eq!(s.value, None);
        assert!(!s.availability.is_ok());
    }

    #[test]
    fn stale_carries_the_last_good_value() {
        let s = Sampled::stale(
            99u32,
            UnixMillis(100),
            Some(FailureCode::ApiError),
            Source::Nvml,
            UnixMillis(200),
        );
        assert_eq!(s.value, Some(99));
        assert!(matches!(s.availability, Availability::Stale { .. }));
    }

    #[test]
    fn map_preserves_provenance() {
        let s = Sampled::ok(10u32, Source::Pdh, UnixMillis(1));
        let mapped = s.map(|v| v * 2);
        assert_eq!(mapped.value, Some(20));
        assert_eq!(mapped.source, Source::Pdh);
    }

    #[test]
    fn wire_shape_is_tagged_by_state() {
        // Locks in the `#[serde(tag = "state")]` shape the frontend depends
        // on to discriminate without a separate `type` field.
        let json = serde_json::to_string(&Availability::NeedsElevation).unwrap();
        assert_eq!(json, r#"{"state":"needsElevation"}"#);
    }

    #[test]
    fn stale_variant_field_is_camel_case_on_the_wire() {
        // Regression test: a container-level `#[serde(rename_all =
        // "camelCase")]` on an enum only renames variant *names*, not the
        // fields inside struct-like variants — `last_error` serialized as
        // exactly that (not `lastError`) until `Stale` got its own
        // `#[serde(rename_all = "camelCase")]`. Every field-bearing variant
        // needs the attribute individually; this pins the one field wide
        // enough (two words) for the bug to actually be visible.
        let json = serde_json::to_string(&Availability::Stale {
            since: UnixMillis(1),
            last_error: Some(FailureCode::Timeout),
        })
        .unwrap();
        assert!(json.contains(r#""lastError":"timeout""#), "got: {json}");
        assert!(!json.contains("last_error"), "got: {json}");
    }
}
