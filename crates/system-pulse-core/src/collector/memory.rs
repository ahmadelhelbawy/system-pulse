//! Memory (and swap) utilization. Ported behaviorally unchanged from 1.0;
//! sysinfo doesn't expose a failure signal for memory refresh on either
//! supported platform, so this collector is always `Ok` once constructed.

use std::sync::Arc;

use parking_lot::Mutex;
use sysinfo::System;

use crate::calc::percent;
use crate::model::{Availability, Sampled, Source};
use crate::types::MemorySnapshot;

use super::{Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege};

pub struct MemoryCollector {
    sys: Arc<Mutex<System>>,
}

impl MemoryCollector {
    pub fn new(sys: Arc<Mutex<System>>) -> Self {
        Self { sys }
    }
}

impl Collector for MemoryCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Memory
    }

    fn cadence(&self) -> Cadence {
        Cadence::Hot
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        self.sys.lock().refresh_memory();
        Availability::Ok
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        let mut sys = self.sys.lock();
        sys.refresh_memory();
        let snapshot = MemorySnapshot {
            total: sys.total_memory(),
            used: sys.used_memory(),
            available: sys.available_memory(),
            used_percent: percent(sys.used_memory(), sys.total_memory()),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
        };
        CollectorOutput::Memory(Sampled::ok(snapshot, Source::Sysinfo, ctx.wall_now))
    }
}
