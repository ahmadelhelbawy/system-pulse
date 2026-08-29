//! Inert stand-in for `store`/`writer`, compiled only when the `history`
//! Cargo feature is off. Presents the exact same public API as the real
//! implementation — a `path`-independent `spawn`, a no-op `record`, an
//! always-empty `query` — so nothing outside the `history` module needs
//! its own `#[cfg(feature = "history")]` to call it. See `mod.rs`'s doc
//! for why this exists: it's what lets `cargo check --target
//! x86_64-pc-windows-msvc --no-default-features` verify the rest of this
//! crate in this repo's sandbox, which has no way to compile the real
//! implementation's bundled-SQLite C dependency for that target.

use std::path::Path;

use super::{HistoryPoint, HistorySample, SeriesId, TimeRange};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {}

pub struct HistoryStore;

impl HistoryStore {
    pub fn open(_path: &Path) -> Result<Self, HistoryError> {
        Ok(Self)
    }

    pub fn query(
        &self,
        _range: TimeRange,
        _series: SeriesId,
    ) -> Result<Vec<HistoryPoint>, HistoryError> {
        Ok(Vec::new())
    }
}

pub struct HistoryWriter;

impl HistoryWriter {
    pub fn spawn(_path: std::path::PathBuf) -> Result<Self, HistoryError> {
        Ok(Self)
    }

    pub fn record(&self, _sample: HistorySample) {}

    pub fn dropped_count(&self) -> u64 {
        0
    }

    pub fn stop(&mut self) {}
}
