//! SQLite-backed history: a bounded high-resolution raw window plus
//! progressively longer-retention rollups (Phase 2 of the master plan).
//!
//! The actual SQLite-backed implementation (`store`/`writer`) is behind the
//! `history` Cargo feature (see `Cargo.toml`) — not a runtime flag.
//! `rusqlite`'s `bundled` feature compiles SQLite's C amalgamation as part
//! of its build script, which this repo's WSL2 sandbox cannot do for the
//! `x86_64-pc-windows-msvc` cross-check target (no MSVC-ABI C toolchain —
//! no `cl.exe`/`lib.exe`/`clang-cl`, only `rust-lld` and a resource-compiler
//! stub for pure-Rust FFI). That means `cargo check --target
//! x86_64-pc-windows-msvc` cannot verify the real implementation in this
//! environment; it *can* verify everything else in this crate via
//! `--no-default-features`, which swaps in `stub`'s inert no-op
//! implementation of the exact same public API (so nothing outside this
//! module — `Scheduler`, `HotLoop`, the `query_history` IPC command — ever
//! needs its own `#[cfg(feature = "history")]`). The real implementation is
//! verified here by `cargo test -p system-pulse-core` on Linux instead,
//! and — like the plan's other real-hardware-only unknowns (PDH GPU
//! counters, COM/WebView2 interaction) — must be confirmed by an actual
//! build on a real Windows host before this ships.
//!
//! Storage model: `samples_raw` is written every tick and is a bounded,
//! ring-deleted evidence window (default 30 minutes) — its purpose is
//! evidence, not long-term storage: a diagnostic finding can cite the
//! actual per-second data behind it (Phase 5). `samples_10s`/`samples_1m`/
//! `samples_5m` are rollups of rollups (each aggregates the table below
//! it, never re-scanning raw data), with progressively longer retention,
//! and are what `query_history` actually serves for any range wider than
//! the raw window.
//!
//! Threading: a single dedicated writer thread owns the one
//! `rusqlite::Connection` (SQLite connections aren't meant to be shared
//! across threads without a serializing wrapper, and a single owner needs
//! none) and drains a bounded channel fed by the hot loop. The hot loop
//! never blocks on it — see `writer::HistoryWriter`.

mod rollup;
#[cfg(feature = "history")]
mod store;
#[cfg(not(feature = "history"))]
mod stub;
#[cfg(feature = "history")]
mod writer;

#[cfg(feature = "history")]
pub use store::{HistoryError, HistoryStore};
#[cfg(feature = "history")]
pub use writer::HistoryWriter;

#[cfg(not(feature = "history"))]
pub use stub::{HistoryError, HistoryStore, HistoryWriter};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::UnixMillis;

/// One tick's worth of headline metrics, as handed to the history writer.
/// A fixed, closed set of columns — not an arbitrary series bag — mirrors
/// the master plan's "deliberately typed, not dynamic" rule for
/// `CollectorOutput`: this is the finite list of metrics a Trends chart
/// actually needs, which keeps the schema, rollup SQL and query API all
/// plainly typed instead of stringly keyed. `None` means "no reading this
/// tick" (the source `Sampled<T>` was not `Ok`) and is stored as SQL
/// `NULL`, never a fabricated `0.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistorySample {
    pub ts_ms: UnixMillis,
    pub cpu_percent: Option<f64>,
    pub mem_used_percent: Option<f64>,
    pub gpu_percent: Option<f64>,
    pub disk_read_rate: Option<f64>,
    pub disk_write_rate: Option<f64>,
    pub net_download_rate: Option<f64>,
    pub net_upload_rate: Option<f64>,
}

/// Which recorded metric a history query wants. Closed enum for the same
/// reason `HistorySample`'s fields are fixed columns rather than a keyed
/// map — see that type's doc.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SeriesId {
    CpuPercent,
    MemUsedPercent,
    GpuPercent,
    DiskReadRate,
    DiskWriteRate,
    NetDownloadRate,
    NetUploadRate,
}

impl SeriesId {
    /// The backing column name. Selecting a dynamic SQL column/table by
    /// interpolating a string is only safe because it is drawn from this
    /// fixed, closed set of compile-time-known identifiers — never from
    /// caller input — which is also why it can't be done with a `?`
    /// bind parameter (SQL doesn't allow binding identifiers, only values).
    /// Only `store` (feature `history`) calls this; `stub` doesn't need a
    /// real column name for its always-empty query.
    #[cfg_attr(not(feature = "history"), allow(dead_code))]
    fn column(self) -> &'static str {
        match self {
            SeriesId::CpuPercent => "cpu_percent",
            SeriesId::MemUsedPercent => "mem_used_percent",
            SeriesId::GpuPercent => "gpu_percent",
            SeriesId::DiskReadRate => "disk_read_rate",
            SeriesId::DiskWriteRate => "disk_write_rate",
            SeriesId::NetDownloadRate => "net_download_rate",
            SeriesId::NetUploadRate => "net_upload_rate",
        }
    }
}

/// An inclusive wall-clock query range. `UnixMillis` on both ends — see
/// `crate::model::time` for why history never mixes this with `Instant`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimeRange {
    pub from_ms: UnixMillis,
    pub to_ms: UnixMillis,
}

/// One point of a queried series.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryPoint {
    pub ts_ms: UnixMillis,
    pub value: f64,
}

/// Retention/rollup constants. Plain constants, not settings — tuning them
/// is a code change, matching how the sampling tiers in `scheduler` are
/// also fixed constants rather than user-configurable knobs.
pub mod retention {
    /// The bounded high-resolution evidence window. Chosen from the
    /// plan's stated 15–60 minute target range.
    pub const RAW_MS: i64 = 30 * 60 * 1000;
    /// Hard row-count backstop on top of the time-based cap above, so an
    /// unusual sampling rate (a much shorter refresh interval) still can't
    /// make the raw table grow unbounded between retention passes.
    pub const RAW_ROW_CAP: i64 = 4_000;

    pub const BUCKET_10S_MS: i64 = 10_000;
    pub const BUCKET_1M_MS: i64 = 60_000;
    pub const BUCKET_5M_MS: i64 = 300_000;

    pub const RETENTION_10S_MS: i64 = 24 * 60 * 60 * 1000; // 24h
    pub const RETENTION_1M_MS: i64 = 7 * 24 * 60 * 60 * 1000; // 7d
    pub const RETENTION_5M_MS: i64 = 30 * 24 * 60 * 60 * 1000; // 30d
}
