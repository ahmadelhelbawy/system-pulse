//! Deterministic alert hysteresis: turns `health::analyze`'s per-tick,
//! stateless candidate list into a stabilized set with debounced
//! raise/clear transitions — the master plan's "an oscillating input
//! produces one alert, not N" requirement.
//!
//! This is layered strictly on top of `health::analyze`, never inside it:
//! `analyze` stays a pure "is this condition true right now" function
//! (easy to unit test with a single frame), while `AlertEngine` is the
//! only place that holds state across ticks. `HealthAlert::id` (see
//! `types.rs`) is the identity debounced on.

use std::collections::HashMap;

use crate::types::HealthAlert;

/// Consecutive ticks a condition must hold before its alert is raised, and
/// consecutive ticks of absence before it's cleared. Applied symmetrically
/// so an alert is exactly as slow to disappear as to appear — a single-tick
/// dip below threshold doesn't clear it, matching how a single-tick spike
/// above threshold doesn't raise it either.
const HYSTERESIS_TICKS: u32 = 3;

#[derive(Default)]
struct AlertState {
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
    latest: Option<HealthAlert>,
}

/// Owns per-alert hysteresis state across ticks. One instance lives for
/// the lifetime of the hot loop (see `scheduler::hot::HotLoop`) — a fresh
/// engine (e.g. across a process restart) has no memory of prior state,
/// which is correct: hysteresis smooths noise within a running session,
/// it isn't meant to survive a restart.
#[derive(Default)]
pub struct AlertEngine {
    states: HashMap<String, AlertState>,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// `candidates` is this tick's raw, stateless list from
    /// `health::analyze`. Returns the stabilized alerts that should
    /// actually be surfaced, most severe first.
    pub fn evaluate(&mut self, candidates: Vec<HealthAlert>) -> Vec<HealthAlert> {
        let mut seen = std::collections::HashSet::with_capacity(candidates.len());
        for alert in candidates {
            seen.insert(alert.id.clone());
            let state = self.states.entry(alert.id.clone()).or_default();
            state.consecutive_present += 1;
            state.consecutive_absent = 0;
            if state.consecutive_present >= HYSTERESIS_TICKS {
                state.active = true;
            }
            state.latest = Some(alert);
        }

        // Everything not seen this tick is one step closer to clearing.
        for (id, state) in self.states.iter_mut() {
            if !seen.contains(id) {
                state.consecutive_present = 0;
                state.consecutive_absent += 1;
                if state.consecutive_absent >= HYSTERESIS_TICKS {
                    state.active = false;
                }
            }
        }
        // Drop bookkeeping for alerts that are both inactive and long gone
        // — otherwise a one-off spike leaves a permanent, ever-growing
        // entry in `states` for the life of the process.
        self.states
            .retain(|_, s| s.active || s.consecutive_absent < HYSTERESIS_TICKS);

        let mut out: Vec<HealthAlert> = self
            .states
            .values()
            .filter(|s| s.active)
            .filter_map(|s| s.latest.clone())
            .collect();
        out.sort_by_key(|a| severity_rank(a.severity));
        out
    }
}

fn severity_rank(s: crate::types::Severity) -> u8 {
    match s {
        crate::types::Severity::Critical => 0,
        crate::types::Severity::Warning => 1,
        crate::types::Severity::Info => 2,
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
