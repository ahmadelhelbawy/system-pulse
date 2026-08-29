//! Provenance model: every collected value carries where it came from and,
//! when unavailable, why — see [`availability`] for the rationale.

mod availability;
mod time;

pub use availability::{Availability, FailureCode, Sampled, Source, UnsupportedReason};
pub use time::UnixMillis;
