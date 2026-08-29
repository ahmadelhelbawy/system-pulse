//! Backpressure primitives: **no queue in this system is unbounded**.
//!
//! Two distinct disciplines, matching the master plan's transport section:
//! - [`Mailbox`]: single-slot, latest-wins. A frame that hasn't been drained
//!   before the next one is produced is replaced, not queued — stale
//!   telemetry frames have no value, so this is coalescing by design, not a
//!   bug to fix later.
//! - [`BoundedRing`]: capacity-N ring for ordered, gap-sensitive streams
//!   (event log, alerts). On overflow the oldest entry is dropped and a
//!   counter increments, so the consumer can render "N dropped" instead of
//!   silently missing history.

mod mailbox;
mod ring;

pub use mailbox::Mailbox;
pub use ring::BoundedRing;
