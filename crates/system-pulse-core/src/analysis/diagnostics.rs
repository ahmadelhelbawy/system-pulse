//! Diagnostics correlation (Phase 5): enriches an already-stabilized alert
//! (health or anomaly) with the actual recorded history behind it, so the
//! Diagnostics screen can cite real evidence instead of restating the
//! alert's own text.
//!
//! Pure and injected: `correlate` takes a `history` closure instead of
//! calling `crate::history::HistoryStore`/`Scheduler::query_history`
//! directly, so it never touches SQLite or a `Scheduler` and stays testable
//! with a synthetic series. The real caller (the `get_diagnostics` IPC
//! command in `src-tauri`) supplies `TelemetryService::query_history` as
//! that closure.

use crate::analysis::anomaly::series_key;
use crate::history::{HistoryPoint, SeriesId, TimeRange};
use crate::model::UnixMillis;
use crate::types::{DiagnosticFinding, EvidencePoint, HealthAlert};

/// How far back to look for evidence behind an active finding — matches
/// the Phase 2 raw evidence window (`history::retention::RAW_MS`), the
/// highest-resolution data actually available to cite.
const EVIDENCE_WINDOW_MS: i64 = crate::history::retention::RAW_MS;

/// Maps a `health::analyze` alert's `category` (`"cpu"`, `"memory"`, ...)
/// or an `analysis::anomaly` alert's `category` (`"anomaly-cpu"`, ...) to
/// the recorded series it concerns. `None` for `"process"` (see
/// `correlate`'s doc) and any other unrecognized category.
fn category_series(category: &str) -> Option<SeriesId> {
    match category {
        "cpu" => Some(SeriesId::CpuPercent),
        "memory" => Some(SeriesId::MemUsedPercent),
        "gpu" => Some(SeriesId::GpuPercent),
        // "disk" health alerts fire on used-space or aggregate read+write
        // I/O, not read vs write specifically; read is the representative
        // series so this stays one history query per finding rather than
        // fetching and merging both directions.
        "disk" => Some(SeriesId::DiskReadRate),
        _ => [
            SeriesId::CpuPercent,
            SeriesId::MemUsedPercent,
            SeriesId::GpuPercent,
            SeriesId::DiskReadRate,
            SeriesId::DiskWriteRate,
            SeriesId::NetDownloadRate,
            SeriesId::NetUploadRate,
        ]
        .into_iter()
        .find(|&s| series_key(s) == category),
    }
}

/// Correlates a stabilized alert list against recorded history, producing
/// evidence-bearing findings — one per input alert, same order.
///
/// Process-category alerts (`category == "process"`) pass through with
/// empty evidence and `duration_ms: 0`: the alert itself (a specific
/// process's current CPU/memory reading) already *is* the evidence, and
/// there is no per-process history table to look further into — see
/// `crate::history`'s module doc: the raw window records seven
/// system-wide series, never per-process ones. Reporting empty evidence
/// here is the honest answer, not a gap papered over with a live lookup
/// this function has no access to (and process alerts don't need one).
pub fn correlate(
    alerts: &[HealthAlert],
    history: &dyn Fn(SeriesId, TimeRange) -> Vec<HistoryPoint>,
    now: UnixMillis,
) -> Vec<DiagnosticFinding> {
    alerts
        .iter()
        .map(|alert| {
            let Some(series) = category_series(&alert.category) else {
                return DiagnosticFinding {
                    id: alert.id.clone(),
                    severity: alert.severity,
                    title: alert.title.clone(),
                    detail: alert.detail.clone(),
                    pid: alert.pid,
                    duration_ms: 0,
                    evidence: Vec::new(),
                };
            };

            let range = TimeRange {
                from_ms: UnixMillis(now.0 - EVIDENCE_WINDOW_MS),
                to_ms: now,
            };
            let evidence: Vec<EvidencePoint> = history(series, range)
                .iter()
                .map(|p| EvidencePoint {
                    ts_ms: p.ts_ms,
                    value: p.value,
                })
                .collect();
            // A conservative lower bound on "how long has this been going
            // on" (the age of the oldest evidence point in the window),
            // not a precise onset time — this never tries to locate the
            // exact threshold-crossing instant from what may be
            // rollup-resolution data.
            let duration_ms = evidence
                .first()
                .map(|p| (now.0 - p.ts_ms.0).max(0))
                .unwrap_or(0);

            DiagnosticFinding {
                id: alert.id.clone(),
                severity: alert.severity,
                title: alert.title.clone(),
                detail: alert.detail.clone(),
                pid: alert.pid,
                duration_ms,
                evidence,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Severity;

    fn alert(category: &str, pid: Option<u32>) -> HealthAlert {
        HealthAlert {
            id: format!("{category}:test"),
            severity: Severity::Warning,
            category: category.to_string(),
            title: "test alert".to_string(),
            detail: "detail".to_string(),
            pid,
        }
    }

    #[test]
    fn a_cpu_alert_is_enriched_with_matching_history() {
        let alerts = vec![alert("cpu", None)];
        let now = UnixMillis(10_000);
        let history = |series: SeriesId, _range: TimeRange| -> Vec<HistoryPoint> {
            assert_eq!(series, SeriesId::CpuPercent);
            vec![
                HistoryPoint {
                    ts_ms: UnixMillis(1_000),
                    value: 90.0,
                },
                HistoryPoint {
                    ts_ms: UnixMillis(5_000),
                    value: 95.0,
                },
            ]
        };
        let findings = correlate(&alerts, &history, now);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.len(), 2);
        assert_eq!(findings[0].duration_ms, 9_000); // 10_000 - 1_000
    }

    #[test]
    fn a_process_alert_never_queries_history_and_has_no_evidence() {
        let alerts = vec![alert("process", Some(42))];
        let history = |_series: SeriesId, _range: TimeRange| -> Vec<HistoryPoint> {
            panic!("process-category alerts must never query history");
        };
        let findings = correlate(&alerts, &history, UnixMillis(1_000));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].evidence.is_empty());
        assert_eq!(findings[0].duration_ms, 0);
        assert_eq!(findings[0].pid, Some(42));
    }

    #[test]
    fn an_anomaly_alert_maps_back_to_its_series() {
        let alerts = vec![alert("anomaly-memory", None)];
        let history = |series: SeriesId, _range: TimeRange| -> Vec<HistoryPoint> {
            assert_eq!(series, SeriesId::MemUsedPercent);
            vec![]
        };
        let findings = correlate(&alerts, &history, UnixMillis(1_000));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].evidence.is_empty());
    }

    #[test]
    fn no_history_data_yields_empty_evidence_not_a_fabricated_duration() {
        let alerts = vec![alert("gpu", None)];
        let history = |_: SeriesId, _: TimeRange| -> Vec<HistoryPoint> { vec![] };
        let findings = correlate(&alerts, &history, UnixMillis(1_000));
        assert!(findings[0].evidence.is_empty());
        assert_eq!(findings[0].duration_ms, 0);
    }

    #[test]
    fn an_unrecognized_category_is_passed_through_with_no_evidence() {
        let alerts = vec![alert("mystery-category", None)];
        let history = |_: SeriesId, _: TimeRange| -> Vec<HistoryPoint> {
            panic!("unrecognized categories must never query history");
        };
        let findings = correlate(&alerts, &history, UnixMillis(1_000));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].evidence.is_empty());
    }
}
