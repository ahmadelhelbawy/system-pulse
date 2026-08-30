//! Deterministic alert hysteresis: turns a per-tick, stateless candidate
//! list into a stabilized set with debounced raise/clear transitions — the
//! master plan's "an oscillating input produces one alert, not N"
//! requirement.
//!
//! This is layered strictly on top of whatever produces the raw candidates
//! (`health::analyze` for threshold-based alerts, `analysis::anomaly` for
//! statistical ones — Phase 5), never inside them: a candidate source stays
//! a pure "is this condition true right now" function (easy to unit test
//! with a single frame), while [`HysteresisEngine`] is the only place that
//! holds state across evaluations.
//!
//! Generic over any [`Identified`] type (Phase 5) rather than hardcoded to
//! `HealthAlert`, because a second, structurally identical consumer showed
//! up immediately (anomaly findings) — not speculative reuse. [`AlertEngine`]
//! is kept as the exact name/shape existing call sites already use.

use std::collections::HashMap;

use crate::types::{HealthAlert, Severity};

/// Consecutive evaluations a condition must hold before its alert is
/// raised, and consecutive absences before it's cleared. Applied
/// symmetrically so an alert is exactly as slow to disappear as to appear
/// — a single dip below threshold doesn't clear it, matching how a single
/// spike above threshold doesn't raise it either. Note this is in units of
/// *evaluate() calls*, not wall-clock time — an engine fed from a Cold-tier
/// collector debounces just as meaningfully over that collector's own
/// (slower) cadence.
const HYSTERESIS_TICKS: u32 = 3;

/// What a [`HysteresisEngine`] needs from the candidate type: a stable
/// identity to debounce on and a severity for sort order. Deliberately not
/// `Ord`/`Eq` on the whole struct — two instances of the same alert with a
/// worsening detail string are still "the same alert" for hysteresis
/// purposes, which is exactly what keying on `id()` alone captures.
pub trait Identified {
    fn id(&self) -> &str;
    fn severity(&self) -> Severity;
}

impl Identified for HealthAlert {
    fn id(&self) -> &str {
        &self.id
    }
    fn severity(&self) -> Severity {
        self.severity
    }
}

struct AlertState<T> {
    consecutive_present: u32,
    consecutive_absent: u32,
    active: bool,
    /// The most recent instance of this alert — severity/detail may
    /// change tick-to-tick while it's continuously present (e.g. a
    /// worsening percentage) without resetting the debounce, since the
    /// identity (`id`) hasn't changed. `None` only until the first
    /// `evaluate()` call that sees this id, which happens before
    /// `active` can ever become true — so it's always `Some` by the time
    /// anything reads it back out.
    latest: Option<T>,
}

impl<T> Default for AlertState<T> {
    fn default() -> Self {
        Self {
            consecutive_present: 0,
            consecutive_absent: 0,
            active: false,
            latest: None,
        }
    }
}

/// Owns per-alert hysteresis state across evaluations. One instance lives
/// for the lifetime of whatever loop feeds it — a fresh engine (e.g. across
/// a process restart) has no memory of prior state, which is correct:
/// hysteresis smooths noise within a running session, it isn't meant to
/// survive a restart.
pub struct HysteresisEngine<T> {
    states: HashMap<String, AlertState<T>>,
    ticks: u32,
}

impl<T: Clone + Identified> Default for HysteresisEngine<T> {
    fn default() -> Self {
        Self::with_ticks(HYSTERESIS_TICKS)
    }
}

impl<T: Clone + Identified> HysteresisEngine<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A custom debounce length — e.g. a statistical anomaly detector
    /// wanting more consecutive confirmations than a hard threshold check,
    /// since transient noise is more common than a genuine step change.
    pub fn with_ticks(ticks: u32) -> Self {
        Self {
            states: HashMap::new(),
            ticks,
        }
    }

    /// `candidates` is this evaluation's raw, stateless list. Returns the
    /// stabilized alerts that should actually be surfaced, most severe
    /// first.
    pub fn evaluate(&mut self, candidates: Vec<T>) -> Vec<T> {
        let mut seen = std::collections::HashSet::with_capacity(candidates.len());
        for alert in candidates {
            let id = alert.id().to_string();
            seen.insert(id.clone());
            let state = self.states.entry(id).or_default();
            state.consecutive_present += 1;
            state.consecutive_absent = 0;
            if state.consecutive_present >= self.ticks {
                state.active = true;
            }
            state.latest = Some(alert);
        }

        // Everything not seen this evaluation is one step closer to clearing.
        for (id, state) in self.states.iter_mut() {
            if !seen.contains(id) {
                state.consecutive_present = 0;
                state.consecutive_absent += 1;
                if state.consecutive_absent >= self.ticks {
                    state.active = false;
                }
            }
        }
        // Drop bookkeeping for alerts that are both inactive and long gone
        // — otherwise a one-off spike leaves a permanent, ever-growing
        // entry in `states` for the life of the process.
        let ticks = self.ticks;
        self.states
            .retain(|_, s| s.active || s.consecutive_absent < ticks);

        let mut out: Vec<T> = self
            .states
            .values()
            .filter(|s| s.active)
            .filter_map(|s| s.latest.clone())
            .collect();
        out.sort_by_key(|a| severity_rank(a.severity()));
        out
    }
}

/// Backward-compatible name for the health-alert engine every existing
/// call site (`scheduler::hot::HotLoop`, tests) already uses.
pub type AlertEngine = HysteresisEngine<HealthAlert>;

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Severity;

    fn alert(id: &str) -> HealthAlert {
        HealthAlert {
            id: id.to_string(),
            severity: Severity::Warning,
            category: "cpu".to_string(),
            title: id.to_string(),
            detail: String::new(),
            pid: None,
        }
    }

    #[test]
    fn a_condition_present_for_fewer_than_the_debounce_ticks_never_surfaces() {
        let mut engine = AlertEngine::new();
        assert!(engine.evaluate(vec![alert("a")]).is_empty());
        assert!(engine.evaluate(vec![alert("a")]).is_empty());
        assert!(engine.evaluate(vec![]).is_empty()); // clears before ever raising
    }

    #[test]
    fn a_condition_sustained_past_the_debounce_raises_exactly_once() {
        let mut engine = AlertEngine::new();
        assert!(engine.evaluate(vec![alert("a")]).is_empty());
        assert!(engine.evaluate(vec![alert("a")]).is_empty());
        let raised = engine.evaluate(vec![alert("a")]);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].id, "a");
    }

    #[test]
    fn an_oscillating_condition_produces_one_alert_not_n() {
        // The plan's exact acceptance criterion: a value flapping around a
        // threshold must not flap the alert with it.
        let mut engine = AlertEngine::new();
        for i in 0..20 {
            let present = i % 2 == 0; // true, false, true, false, ...
            let candidates = if present { vec![alert("a")] } else { vec![] };
            let out = engine.evaluate(candidates);
            // Never reaches 3 consecutive presences, so it must never raise.
            assert!(out.is_empty(), "tick {i}: alert should never have raised");
        }
    }

    #[test]
    fn a_raised_alert_requires_sustained_absence_to_clear() {
        let mut engine = AlertEngine::new();
        for _ in 0..3 {
            engine.evaluate(vec![alert("a")]);
        }
        assert_eq!(engine.evaluate(vec![alert("a")]).len(), 1); // still active

        assert_eq!(engine.evaluate(vec![]).len(), 1); // 1 absent tick: still active
        assert_eq!(engine.evaluate(vec![]).len(), 1); // 2: still active
        assert!(engine.evaluate(vec![]).is_empty()); // 3: cleared
    }

    #[test]
    fn severity_updates_in_place_without_resetting_debounce() {
        let mut engine = AlertEngine::new();
        for _ in 0..3 {
            engine.evaluate(vec![alert("a")]);
        }
        let mut critical = alert("a");
        critical.severity = Severity::Critical;
        let out = engine.evaluate(vec![critical]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Critical);
    }
}
