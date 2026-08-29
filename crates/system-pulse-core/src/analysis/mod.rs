//! Health scoring (Phase 2). Deterministic and explainable by construction
//! — a fixed per-severity point penalty per active alert, nothing learned
//! or fitted — matching the master plan's "detection authority" rule that
//! carries forward to Phase 5: an optional future LLM layer may explain a
//! score, it may never compute one.
//!
//! Anomaly detection (robust statistics over `crate::history`) and
//! diagnostics correlation are Phase 5 scope and don't live here yet.

use crate::types::{DomainHealth, HealthAlert, HealthScore, Severity};

/// Every domain always appears in `HealthScore::domains`, even at a
/// perfect 100 with no contributors — so the frontend can render a
/// consistent fixed set of gauges instead of a list that grows and
/// shrinks as issues come and go.
const DOMAINS: [&str; 5] = ["cpu", "memory", "disk", "gpu", "process"];

const PENALTY_CRITICAL: i32 = 40;
const PENALTY_WARNING: i32 = 15;
const PENALTY_INFO: i32 = 5;

fn penalty(severity: Severity) -> i32 {
    match severity {
        Severity::Critical => PENALTY_CRITICAL,
        Severity::Warning => PENALTY_WARNING,
        Severity::Info => PENALTY_INFO,
    }
}

/// Scores a stabilized alert list (the output of
/// `crate::alerts::AlertEngine::evaluate`, not `health::analyze`'s raw
/// candidates — scoring on undebounced candidates would make the score
/// exactly as flappy as the alerts it's built from claim not to be).
pub fn score(alerts: &[HealthAlert]) -> HealthScore {
    let domains: Vec<DomainHealth> = DOMAINS
        .iter()
        .map(|&domain| {
            let domain_alerts: Vec<&HealthAlert> =
                alerts.iter().filter(|a| a.category == domain).collect();
            let raw = 100
                - domain_alerts
                    .iter()
                    .map(|a| penalty(a.severity))
                    .sum::<i32>();
            DomainHealth {
                domain: domain.to_string(),
                score: raw.clamp(0, 100) as u8,
                contributors: domain_alerts.iter().map(|a| a.title.clone()).collect(),
            }
        })
        .collect();

    // The mean, not the minimum: one saturated domain should pull the
    // overall number down, not zero it out while four other domains are
    // perfectly healthy evidence to the contrary.
    let overall =
        (domains.iter().map(|d| d.score as u32).sum::<u32>() / DOMAINS.len() as u32) as u8;

    HealthScore {
        overall,
        domains,
        alerts: alerts.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(category: &str, severity: Severity) -> HealthAlert {
        HealthAlert {
            id: format!("{category}:test"),
            severity,
            category: category.to_string(),
            title: format!("{category} issue"),
            detail: String::new(),
            pid: None,
        }
    }

    #[test]
    fn a_quiet_system_scores_a_perfect_100_with_all_domains_present() {
        let score = score(&[]);
        assert_eq!(score.overall, 100);
        assert_eq!(score.domains.len(), DOMAINS.len());
        assert!(score
            .domains
            .iter()
            .all(|d| d.score == 100 && d.contributors.is_empty()));
    }

    #[test]
    fn a_critical_alert_drags_down_its_own_domain_and_the_overall_mean() {
        let score = score(&[alert("memory", Severity::Critical)]);
        let memory = score.domains.iter().find(|d| d.domain == "memory").unwrap();
        assert_eq!(memory.score, 60); // 100 - 40
        assert_eq!(memory.contributors, vec!["memory issue".to_string()]);
        // Mean of [60, 100, 100, 100, 100] = 92.
        assert_eq!(score.overall, 92);
    }

    #[test]
    fn one_saturated_domain_does_not_zero_out_the_overall_score() {
        let score = score(&[
            alert("cpu", Severity::Critical),
            alert("cpu", Severity::Critical),
            alert("cpu", Severity::Critical),
        ]);
        let cpu = score.domains.iter().find(|d| d.domain == "cpu").unwrap();
        assert_eq!(cpu.score, 0); // 100 - 120, clamped
        assert!(
            score.overall > 0,
            "a single ruined domain must not zero the overall score"
        );
    }

    #[test]
    fn multiple_alerts_in_one_domain_all_appear_as_contributors() {
        let score = score(&[
            alert("disk", Severity::Warning),
            alert("gpu", Severity::Info),
        ]);
        let disk = score.domains.iter().find(|d| d.domain == "disk").unwrap();
        assert_eq!(disk.score, 85);
        let gpu = score.domains.iter().find(|d| d.domain == "gpu").unwrap();
        assert_eq!(gpu.score, 95);
    }
}
