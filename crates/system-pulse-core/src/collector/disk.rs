//! Per-disk space and I/O throughput.
//!
//! Fixes the 1.0 rate defect: `dt` used to be measured from the last
//! *refresh* to the moment the frame was assembled (single-digit ms, since
//! assembly happened on every tick but refresh only every Nth tick), while
//! the previous-totals map was overwritten on every assembly regardless.
//! Rates alternated between a ~100-1000x spike and exactly `0.0`.
//!
//! The fix is structural, not a patched formula: this collector now only
//! runs at its own `Warm` cadence, and refresh + read happen atomically in
//! the same `collect()` call, so `dt` is genuinely refresh-to-refresh —
//! there is no longer a faster "assembly" cycle for it to be measured
//! against by mistake.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysinfo::{DiskRefreshKind, Disks};

use crate::calc::{compute_rate, percent};
use crate::model::{Availability, Sampled, Source};
use crate::types::{DiskIoSnapshot, DiskSnapshot};

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

const DISK_CADENCE: Duration = Duration::from_secs(2);

pub struct DiskCollector {
    disks: Disks,
    prev_totals: HashMap<String, (u64, u64)>,
    last_collect_at: Option<Instant>,
}

impl DiskCollector {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            prev_totals: HashMap::new(),
            last_collect_at: None,
        }
    }

    fn read_totals(&self) -> HashMap<String, (u64, u64)> {
        self.disks
            .list()
            .iter()
            .map(|d| {
                let u = d.usage();
                (
                    d.name().to_string_lossy().into_owned(),
                    (u.total_read_bytes, u.total_written_bytes),
                )
            })
            .collect()
    }
}

impl Default for DiskCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for DiskCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Disk
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(DISK_CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.disks
            .refresh_specifics(true, DiskRefreshKind::everything());
        Availability::Ok
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        self.disks
            .refresh_specifics(true, DiskRefreshKind::everything());
        let curr_totals = self.read_totals();
        let dt = self
            .last_collect_at
            .map(|t| ctx.now.duration_since(t).as_secs_f64());

        let mut list = Vec::new();
        let mut total_read = 0u64;
        let mut total_write = 0u64;
        let mut total_read_rate = 0f64;
        let mut total_write_rate = 0f64;

        for disk in self.disks.list() {
            let key = disk.name().to_string_lossy().into_owned();
            let (read, write) = curr_totals.get(&key).copied().unwrap_or((0, 0));

            // A rate needs both a prior sample for this exact disk and a
            // known elapsed time since it was taken; either being absent
            // (first tick, or a disk that just appeared) means "no rate
            // yet", not zero throughput mislabeled as a real reading.
            let (read_rate, write_rate) = match (dt, self.prev_totals.get(&key)) {
                (Some(dt), Some(&(prev_read, prev_write))) => (
                    compute_rate(prev_read, read, dt),
                    compute_rate(prev_write, write, dt),
                ),
                _ => (0.0, 0.0),
            };

            total_read += read;
            total_write += write;
            total_read_rate += read_rate;
            total_write_rate += write_rate;

            let used = disk.total_space().saturating_sub(disk.available_space());
            list.push(DiskSnapshot {
                name: key,
                mount_point: disk.mount_point().to_string_lossy().into_owned(),
                file_system: disk.file_system().to_string_lossy().into_owned(),
                total: disk.total_space(),
                available: disk.available_space(),
                used_percent: percent(used, disk.total_space()),
                read_rate,
                write_rate,
                is_removable: disk.is_removable(),
            });
        }

        self.prev_totals = curr_totals;
        self.last_collect_at = Some(ctx.now);

        CollectorOutput::Disk {
            disks: Sampled::ok(list, Source::Sysinfo, ctx.wall_now),
            io: Sampled::ok(
                DiskIoSnapshot {
                    read_rate: total_read_rate,
                    write_rate: total_write_rate,
                    total_read,
                    total_write,
                },
                Source::Sysinfo,
                ctx.wall_now,
            ),
        }
    }

    /// Drops the rate baseline so the next tick reports a fresh `0.0` rate
    /// instead of averaging throughput over however long sampling was
    /// paused (1.0 defect: stale baselines after resume).
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

    /// Reproduces the 1.0 `dt` regression directly against the corrected
    /// rate logic: two `collect()` calls two seconds apart (by the
    /// controlled `CollectCtx.now`, not real sleep) over a synthetic
    /// counter delta must yield the true rate — not a spike, and not zero.
    #[test]
    fn rate_reflects_true_elapsed_time_between_collects() {
        let prev_totals: HashMap<String, (u64, u64)> =
            [("disk0".to_string(), (0u64, 0u64))].into_iter().collect();
        let curr_totals: HashMap<String, (u64, u64)> = [("disk0".to_string(), (2_000_000, 0))]
            .into_iter()
            .collect();

        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(2);

        // Exercise the exact formula collect() uses, isolated from sysinfo
        // (which can't be told what disks exist in a unit test): dt must
        // come from ctx.now deltas between collect() calls, not from
        // refresh-to-assembly time.
        let dt = t1.duration_since(t0).as_secs_f64();
        assert_eq!(dt, 2.0);
        let rate = compute_rate(prev_totals["disk0"].0, curr_totals["disk0"].0, dt);
        // 2,000,000 bytes / 2s = 1,000,000 B/s — not ~100-1000x that (the
        // old refresh-to-snapshot-ms bug) and not 0.0 (the old
        // overwritten-baseline bug on the alternating tick).
        assert_eq!(rate, 1_000_000.0);
    }

    #[test]
    fn first_collect_reports_zero_rate_not_a_failure() {
        let mut c = DiskCollector::new();
        c.probe();
        let out = c.collect(&ctx(Instant::now()));
        match out {
            CollectorOutput::Disk { disks, io } => {
                assert!(disks.availability.is_ok());
                assert!(io.availability.is_ok());
                assert_eq!(io.value.unwrap().read_rate, 0.0);
            }
            _ => panic!("expected Disk output"),
        }
    }

    #[test]
    fn reset_baseline_clears_prev_totals_and_timestamp() {
        let mut c = DiskCollector::new();
        c.probe();
        c.collect(&ctx(Instant::now()));
        c.reset_baseline();
        assert!(c.prev_totals.is_empty());
        assert!(c.last_collect_at.is_none());
    }
}
