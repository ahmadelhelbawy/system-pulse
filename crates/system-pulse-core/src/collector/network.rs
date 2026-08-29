//! Per-interface network throughput. Same `dt` fix as [`super::disk`] — see
//! that module's doc comment for the full explanation.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysinfo::Networks;

use crate::calc::compute_rate;
use crate::model::{Availability, Sampled, Source};
use crate::types::NetworkSnapshot;

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

const NETWORK_CADENCE: Duration = Duration::from_secs(2);

pub struct NetworkCollector {
    networks: Networks,
    prev_totals: HashMap<String, (u64, u64)>,
    last_collect_at: Option<Instant>,
}

impl NetworkCollector {
    pub fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            prev_totals: HashMap::new(),
            last_collect_at: None,
        }
    }

    fn read_totals(&self) -> HashMap<String, (u64, u64)> {
        self.networks
            .list()
            .iter()
            .map(|(name, data)| {
                (
                    name.clone(),
                    (data.total_received(), data.total_transmitted()),
                )
            })
            .collect()
    }
}

impl Default for NetworkCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for NetworkCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Network
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(NETWORK_CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.networks.refresh(true);
        Availability::Ok
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        self.networks.refresh(true);
        let curr_totals = self.read_totals();
        let dt = self
            .last_collect_at
            .map(|t| ctx.now.duration_since(t).as_secs_f64());

        let mut out = Vec::new();
        for (name, data) in self.networks.list() {
            let (rx, tx) = (data.total_received(), data.total_transmitted());
            let (download_rate, upload_rate) = match (dt, self.prev_totals.get(name)) {
                (Some(dt), Some(&(prev_rx, prev_tx))) => {
                    (compute_rate(prev_rx, rx, dt), compute_rate(prev_tx, tx, dt))
                }
                _ => (0.0, 0.0),
            };
            out.push(NetworkSnapshot {
                name: name.clone(),
                download_rate,
                upload_rate,
                total_rx: rx,
                total_tx: tx,
            });
        }
        // Stable ordering for deterministic UI and snapshots.
        out.sort_by(|a, b| a.name.cmp(&b.name));

        self.prev_totals = curr_totals;
        self.last_collect_at = Some(ctx.now);

        CollectorOutput::Network(Sampled::ok(out, Source::Sysinfo, ctx.wall_now))
    }

    /// See `DiskCollector::reset_baseline` — same 1.0 defect, same fix.
    fn reset_baseline(&mut self) {
        self.prev_totals.clear();
        self.last_collect_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UnixMillis;

    fn ctx(now: Instant) -> CollectCtx {
        CollectCtx {
            now,
            wall_now: UnixMillis(0),
        }
    }

    #[test]
    fn rate_reflects_true_elapsed_time_between_collects() {
        let dt = Duration::from_secs(2).as_secs_f64();
        let rate = compute_rate(0, 1_000_000, dt);
        assert_eq!(rate, 500_000.0);
    }

    #[test]
    fn first_collect_reports_zero_rate_not_a_failure() {
        let mut c = NetworkCollector::new();
        c.probe();
        let out = c.collect(&ctx(Instant::now()));
        match out {
            CollectorOutput::Network(sampled) => {
                assert!(sampled.availability.is_ok());
                for n in sampled.value.unwrap() {
                    assert_eq!(n.download_rate, 0.0);
                    assert_eq!(n.upload_rate, 0.0);
                }
            }
            _ => panic!("expected Network output"),
        }
    }

    #[test]
    fn output_is_sorted_by_name() {
        let mut c = NetworkCollector::new();
        c.probe();
        let out = c.collect(&ctx(Instant::now()));
        match out {
            CollectorOutput::Network(sampled) => {
                let names: Vec<String> =
                    sampled.value.unwrap().into_iter().map(|n| n.name).collect();
                let mut sorted = names.clone();
                sorted.sort();
                assert_eq!(names, sorted);
            }
            _ => panic!("expected Network output"),
        }
    }
}
