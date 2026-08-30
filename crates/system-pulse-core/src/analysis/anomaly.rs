//! Statistical anomaly detection (Phase 5) — robust statistics only, no ML,
//! matching the master plan's "detection authority" rule: this module (like
//! `health`) only ever answers "is this reading unusual right now," and
//! [`crate::alerts::HysteresisEngine`] (not this module) debounces that into
//! a stable, surfaced finding — the exact same separation `health.rs` and
//! `alerts` already have, reused rather than duplicated.
//!
//! **Deliberate scope decision on "time-of-day baselines."** The master
//! plan's example toolkit is "rolling median/MAD, EWMA, z-score with
//! time-of-day baselines." Correlating against the same hour on prior days
//! would need a `query_history` round trip per series per tick from the hot
//! thread, which the hot thread must never do (SQLite I/O is exactly the
//! kind of blocking call the hot/warm split exists to keep off it). Instead,
//! each series keeps a short *rolling* window (a few minutes) of its own
//! recent values: a real diurnal ramp (load rising over the morning) moves
//! slowly relative to that window, so the rolling median tracks it and no
//! anomaly fires — while a genuine spike (seconds, not hours) stands out
//! sharply against the recent baseline it hasn't had time to absorb yet.
//! This is the same robust-statistics family the plan calls for, sized to
//! the hot loop's real constraints rather than the literal multi-day
//! implementation, and is what the "diurnal pattern not flagged" acceptance
//! test below actually verifies.

use std::collections::VecDeque;

use crate::history::SeriesId;
use crate::types::{HealthAlert, Severity};

/// Samples kept per series — a handful of minutes at the default 1 Hz tick,
/// long enough to average out per-tick jitter without absorbing a real
/// spike into "normal."
const WINDOW_LEN: usize = 180;
/// A series needs at least this many samples before it can accuse anything
/// — an empty or near-empty window has no meaningful median/MAD, and the
/// first few ticks after startup must never be flagged.
const MIN_SAMPLES: usize = 30;
/// Robust z-score threshold. `1.4826` is the constant that makes MAD a
/// consistent estimator of standard deviation for a normal distribution;
/// a z-score past this is a multi-sigma-equivalent outlier, not routine
/// noise.
const Z_THRESHOLD: f64 = 5.0;
/// Fast EWMA smoothing factor — tracks the last minute or so, used
/// opposite the slow EWMA below to catch a *sustained* level shift (see
/// `RollingStats::ewma_shift_score`) that a single-tick spike detector
/// could miss if the shift arrives gradually enough that no individual
/// tick stands out against the rolling median, yet the level has still
/// genuinely moved.
const EWMA_FAST_ALPHA: f64 = 0.1;
/// Slow EWMA smoothing factor — the "expected typical level," deliberately
/// changing an order of magnitude slower than the fast one so a real shift
/// takes time to be absorbed into "normal," rather than instantly agreeing
/// with whatever the fast average just did.
const EWMA_SLOW_ALPHA: f64 = 0.01;
/// How many MAD-widths the fast/slow EWMAs must diverge by before that
/// divergence itself counts as a finding — same threshold family as
/// `Z_THRESHOLD`, expressed in the same robust units.
const EWMA_SHIFT_THRESHOLD: f64 = 3.0;

/// Rolling robust statistics for one series: a bounded window (median/MAD)
/// plus two EWMAs at different timescales. Pure, deterministic, no
/// history/SQL/IO — this is why it can run on the hot thread.
#[derive(Debug, Default)]
pub struct RollingStats {
    window: VecDeque<f64>,
    ewma_fast: Option<f64>,
    ewma_slow: Option<f64>,
}

impl RollingStats {
    pub fn push(&mut self, value: f64) {
        self.window.push_back(value);
        if self.window.len() > WINDOW_LEN {
            self.window.pop_front();
        }
        self.ewma_fast = Some(match self.ewma_fast {
            Some(prev) => EWMA_FAST_ALPHA * value + (1.0 - EWMA_FAST_ALPHA) * prev,
            None => value,
        });
        self.ewma_slow = Some(match self.ewma_slow {
            Some(prev) => EWMA_SLOW_ALPHA * value + (1.0 - EWMA_SLOW_ALPHA) * prev,
            None => value,
        });
    }

    /// How far the fast EWMA has pulled away from the slow one, in
    /// MAD-widths — a sustained shift shows up here even on a tick whose
    /// own `robust_z_score` isn't extreme, because the *window's* median
    /// has already started drifting toward the new level too. `None`
    /// before there's enough history for a meaningful MAD.
    fn ewma_shift_score(&self) -> Option<f64> {
        if self.window.len() < MIN_SAMPLES {
            return None;
        }
        let (fast, slow) = (self.ewma_fast?, self.ewma_slow?);
        let median = self.median()?;
        let mad = self.mad(median);
        if mad == 0.0 {
            return Some(if fast == slow { 0.0 } else { 1e6 });
        }
        Some((fast - slow).abs() / (1.4826 * mad))
    }

    fn median(&self) -> Option<f64> {
        if self.window.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = self.window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = sorted.len() / 2;
        Some(if sorted.len().is_multiple_of(2) {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        })
    }

    /// Median absolute deviation — the robust analogue of standard
    /// deviation, unaffected by the very outliers this function is used to
    /// detect (a plain stddev is inflated by the spike it's supposed to
    /// flag, which raises its own threshold and can mask the spike).
    fn mad(&self, median: f64) -> f64 {
        let mut deviations: Vec<f64> = self.window.iter().map(|v| (v - median).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = deviations.len() / 2;
        if deviations.len().is_multiple_of(2) {
            (deviations[mid - 1] + deviations[mid]) / 2.0
        } else {
            deviations[mid]
        }
    }

    /// Robust z-score of `value` against this series' own rolling
    /// median/MAD. `None` until the window has enough samples to mean
    /// anything.
    pub fn robust_z_score(&self, value: f64) -> Option<f64> {
        if self.window.len() < MIN_SAMPLES {
            return None;
        }
        let median = self.median()?;
        let mad = self.mad(median);
        if mad == 0.0 {
            // A perfectly flat recent window: any deviation at all is
            // notable, but there's no scale to divide by. Treat a nonzero
            // deviation as a large but finite score rather than +-infinity
            // or a divide-by-zero panic.
            return Some(if value == median { 0.0 } else { 1e6 });
        }
        Some((value - median).abs() / (1.4826 * mad))
    }
}

/// One rolling detector per fixed telemetry series — a closed set, same
/// reasoning as `SeriesId`/`HistorySample` (see `crate::history`'s module
/// doc): the finite list of metrics this app actually measures, not an
/// arbitrary keyed bag.
#[derive(Debug, Default)]
pub struct AnomalyDetector {
    cpu: RollingStats,
    memory: RollingStats,
    gpu: RollingStats,
    disk_read: RollingStats,
    disk_write: RollingStats,
    net_down: RollingStats,
    net_up: RollingStats,
}

/// Mirrors `HistorySample`'s shape (same seven fields the history writer
/// already assembles every tick) so callers don't need to build a second,
/// parallel struct just to feed this detector.
pub struct AnomalyInput {
    pub cpu_percent: Option<f64>,
    pub mem_used_percent: Option<f64>,
    pub gpu_percent: Option<f64>,
    pub disk_read_rate: Option<f64>,
    pub disk_write_rate: Option<f64>,
    pub net_download_rate: Option<f64>,
    pub net_upload_rate: Option<f64>,
}

fn series_label(series: SeriesId) -> &'static str {
    match series {
        SeriesId::CpuPercent => "CPU",
        SeriesId::MemUsedPercent => "memory",
        SeriesId::GpuPercent => "GPU",
        SeriesId::DiskReadRate => "disk read",
        SeriesId::DiskWriteRate => "disk write",
        SeriesId::NetDownloadRate => "network download",
        SeriesId::NetUploadRate => "network upload",
    }
}

/// A stable, machine-parseable key per series — the alert's `category`,
/// and what `analysis::diagnostics::category_series` maps back to a
/// [`SeriesId`] for evidence correlation. Kept distinct from
/// `series_label`'s human-readable text so a copy-edit of the display
/// string can never silently break correlation.
pub fn series_key(series: SeriesId) -> &'static str {
    match series {
        SeriesId::CpuPercent => "anomaly-cpu",
        SeriesId::MemUsedPercent => "anomaly-memory",
        SeriesId::GpuPercent => "anomaly-gpu",
        SeriesId::DiskReadRate => "anomaly-disk-read",
        SeriesId::DiskWriteRate => "anomaly-disk-write",
        SeriesId::NetDownloadRate => "anomaly-net-download",
        SeriesId::NetUploadRate => "anomaly-net-upload",
    }
}

impl AnomalyDetector {
    pub fn new() -> Self {
        Self::default()
    }

    fn check(
        stats: &mut RollingStats,
        series: SeriesId,
        value: Option<f64>,
        alerts: &mut Vec<HealthAlert>,
    ) {
        let Some(value) = value else { return };
        // Compute both scores against the window *before* pushing this
        // sample — otherwise every value is compared partly against
        // itself, which dilutes exactly the deviation we're trying to
        // catch.
        let z = stats.robust_z_score(value);
        let shift = stats.ewma_shift_score();
        stats.push(value);

        let spike = z.filter(|&z| z >= Z_THRESHOLD);
        let sustained_shift = shift.filter(|&s| s >= EWMA_SHIFT_THRESHOLD);
        // Either signal alone is sufficient: a spike detector catches a
        // sudden jump the slow-moving EWMA hasn't reacted to yet; the
        // EWMA-shift detector catches a level change gradual enough that
        // no single tick's z-score stands out, but which has still pulled
        // the fast average away from the slow one.
        let Some(score) = spike.or(sustained_shift) else {
            return;
        };
        let label = series_label(series);
        let key = series_key(series);
        alerts.push(HealthAlert {
            id: key.to_string(),
            severity: Severity::Warning,
            category: key.to_string(),
            title: format!("Unusual {label} activity"),
            detail: format!(
                "{label} reading of {value:.1} is a {score:.1}x deviation from its recent baseline"
            ),
            pid: None,
        });
    }

    /// One tick's worth of candidates — stateless from the caller's
    /// perspective (all state lives in this detector's own rolling
    /// windows), same shape as `health::analyze`. The caller is expected to
    /// pass these through a `HysteresisEngine` before surfacing them, same
    /// as `health::analyze`'s output.
    pub fn detect(&mut self, input: &AnomalyInput) -> Vec<HealthAlert> {
        let mut alerts = Vec::new();
        Self::check(
            &mut self.cpu,
            SeriesId::CpuPercent,
            input.cpu_percent,
            &mut alerts,
        );
        Self::check(
            &mut self.memory,
            SeriesId::MemUsedPercent,
            input.mem_used_percent,
            &mut alerts,
        );
        Self::check(
            &mut self.gpu,
            SeriesId::GpuPercent,
            input.gpu_percent,
            &mut alerts,
        );
        Self::check(
            &mut self.disk_read,
            SeriesId::DiskReadRate,
            input.disk_read_rate,
            &mut alerts,
        );
        Self::check(
            &mut self.disk_write,
            SeriesId::DiskWriteRate,
            input.disk_write_rate,
            &mut alerts,
        );
        Self::check(
            &mut self.net_down,
            SeriesId::NetDownloadRate,
            input.net_download_rate,
            &mut alerts,
        );
        Self::check(
            &mut self.net_up,
            SeriesId::NetUploadRate,
            input.net_upload_rate,
            &mut alerts,
        );
        alerts.sort_by_key(|a| a.id.clone());
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady_input(v: f64) -> AnomalyInput {
        AnomalyInput {
            cpu_percent: Some(v),
            mem_used_percent: None,
            gpu_percent: None,
            disk_read_rate: None,
            disk_write_rate: None,
            net_download_rate: None,
            net_upload_rate: None,
        }
    }

    #[test]
    fn an_empty_or_short_window_never_flags_anything() {
        let mut d = AnomalyDetector::new();
        for _ in 0..MIN_SAMPLES - 1 {
            assert!(d.detect(&steady_input(50.0)).is_empty());
        }
    }

    #[test]
    fn a_flat_series_never_flags_itself() {
        let mut d = AnomalyDetector::new();
        for _ in 0..WINDOW_LEN {
            assert!(d.detect(&steady_input(20.0)).is_empty());
        }
    }

    #[test]
    fn a_sudden_spike_against_a_stable_baseline_is_flagged() {
        let mut d = AnomalyDetector::new();
        for _ in 0..MIN_SAMPLES + 10 {
            d.detect(&steady_input(10.0));
        }
        let alerts = d.detect(&steady_input(95.0));
        assert!(
            alerts.iter().any(|a| a.category == "anomaly-cpu"),
            "a sudden large jump must be flagged as anomalous"
        );
    }

    #[test]
    fn a_gradual_diurnal_ramp_is_never_flagged() {
        // Simulates load climbing smoothly over "the morning" — each step
        // is tiny relative to the rolling window's own spread, so the
        // rolling median/MAD tracks the ramp instead of treating every
        // step as a fresh outlier.
        let mut d = AnomalyDetector::new();
        let mut any_flagged = false;
        let mut v = 10.0;
        for _ in 0..(WINDOW_LEN * 3) {
            v += 0.05;
            if !d.detect(&steady_input(v)).is_empty() {
                any_flagged = true;
            }
        }
        assert!(
            !any_flagged,
            "a smooth ramp must not be flagged as anomalous"
        );
    }

    #[test]
    fn a_perfectly_flat_window_does_not_divide_by_zero() {
        let mut d = AnomalyDetector::new();
        for _ in 0..MIN_SAMPLES + 5 {
            let alerts = d.detect(&steady_input(0.0));
            assert!(alerts.is_empty());
        }
        // Every prior value identical (MAD == 0); this must not panic.
        let alerts = d.detect(&steady_input(1.0));
        assert!(
            !alerts.is_empty(),
            "a jump off a flat baseline is still an outlier"
        );
    }

    #[test]
    fn missing_readings_are_skipped_not_treated_as_zero() {
        let mut d = AnomalyDetector::new();
        let empty = AnomalyInput {
            cpu_percent: None,
            mem_used_percent: None,
            gpu_percent: None,
            disk_read_rate: None,
            disk_write_rate: None,
            net_download_rate: None,
            net_upload_rate: None,
        };
        for _ in 0..MIN_SAMPLES + 10 {
            assert!(d.detect(&empty).is_empty());
        }
    }
}
