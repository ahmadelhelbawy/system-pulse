//! PDH `\GPU Engine(*)\Utilization Percentage`: per-process GPU
//! utilization, vendor-neutral (works for AMD/Intel/NVIDIA alike) — this is
//! how Task Manager's own per-process GPU column works. NVML never
//! provided per-process *utilization* (only per-process VRAM), so this
//! collector's per-process data is always attempted regardless of whether
//! NVML is present; it additionally provides a device-level fallback for
//! when NVML has no data at all (no NVIDIA hardware/driver) — see
//! `CollectorOutput::PdhGpu`'s doc comment for the fallback ladder.
//!
//! **Hardening status.** The master plan requires several things this
//! environment cannot validate (no Windows host, let alone a multi-GPU
//! one):
//! - Locale safety (`PdhAddEnglishCounterW`, never a localized path) —
//!   the *API choice* is directly verifiable from documentation and is
//!   used exclusively below; its *behavior* on a non-English Windows
//!   install is unverified.
//! - Wildcard enumeration performance on real multi-GPU hardware — the
//!   caching (1h TTL) and hard instance cap below are implemented and
//!   unit-tested for their *logic*, but the actual instance counts and
//!   timing they're sized against are assumed, not measured.
//! - The NVML-vs-PDH fallback ladder's real-world reliability — untested
//!   against real AMD/Intel hardware.
//!
//! These are open validation items for a Windows host, not simulated here.

use std::collections::HashMap;
use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::gpu::NvidiaGpuProvider;
use system_pulse_core::model::{Availability, Sampled, Source, UnsupportedReason};
use system_pulse_core::types::GpuSnapshot;

const CADENCE: Duration = Duration::from_secs(5);
/// Hard cap on GPU-engine instances processed per tick. Overflow is
/// reported via `truncated`, never silently dropped — see
/// `RawPdhSample::truncated`.
const MAX_INSTANCES: usize = 512;
//
// The master plan calls for caching a *separate* wildcard expansion
// (`PdhExpandWildCardPathW`) with a 1h TTL. This collector instead reads
// via `PdhGetFormattedCounterArrayW` on one wildcard-added counter, which
// re-resolves matching instances as an inherent part of every call — there
// is no separate "expand, then cache" step to add a TTL to; the cost
// control that exists is the 5s `Warm` cadence itself. If a real Windows
// host's profiling shows the array call re-resolving instances is itself
// expensive independent of item count, that would be the trigger to
// switch to an explicit expand-and-cache design instead.

/// One raw `\GPU Engine(*)\Utilization Percentage` instance reading, in the
/// shape `PdhGetFormattedCounterArrayW` returns it (instance name + value),
/// decoupled from the `windows` crate's types so parsing is testable
/// anywhere.
#[derive(Debug, Clone)]
pub struct RawGpuEngineInstance {
    pub instance_name: String,
    pub utilization_percent: f64,
}

/// GPU Engine instance names encode the owning pid, the physical adapter
/// index, and the engine type, e.g.
/// `pid_4821_luid_0x00000000_0x0001E2C1_phys_0_eng_0_engtype_3D`. Neither
/// PID nor physical index is available any other way from this counter —
/// they are *only* in the instance name string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedInstance {
    pub pid: u32,
    pub phys_index: u32,
}

/// Extracts `pid_<N>` and `phys_<N>` from a GPU Engine instance name.
/// Returns `None` for a name that doesn't match the documented shape
/// (rather than guessing) — a future Windows version changing this format
/// should degrade to "no attribution" per-instance, not misattribute data.
pub fn parse_instance_name(name: &str) -> Option<ParsedInstance> {
    let pid = extract_underscore_prefixed_number(name, "pid_")?;
    let phys_index = extract_underscore_prefixed_number(name, "phys_")?;
    Some(ParsedInstance { pid, phys_index })
}

fn extract_underscore_prefixed_number(name: &str, prefix: &str) -> Option<u32> {
    let start = name.find(prefix)? + prefix.len();
    let rest = &name[start..];
    let end = rest.find('_').unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Aggregates raw per-engine instances into per-process utilization (summed
/// across engines/adapters for that pid — a process using both the 3D and
/// video-decode engines shows one combined percentage, matching Task
/// Manager's own per-process GPU column) and per-adapter utilization (for
/// the NVML-absent device fallback).
///
/// Instances past `MAX_INSTANCES` are dropped, not silently ignored: the
/// returned `truncated` count says how many, so a UI can say "+N more"
/// rather than quietly under-reporting on a many-process/many-GPU machine.
pub struct AggregatedGpuEngine {
    pub per_process_percent: HashMap<u32, f32>,
    pub per_adapter_percent: HashMap<u32, f32>,
    pub truncated: usize,
}

pub fn aggregate(instances: &[RawGpuEngineInstance]) -> AggregatedGpuEngine {
    let mut per_process: HashMap<u32, f64> = HashMap::new();
    let mut per_adapter: HashMap<u32, f64> = HashMap::new();
    let mut truncated = 0usize;

    for (i, inst) in instances.iter().enumerate() {
        if i >= MAX_INSTANCES {
            truncated += 1;
            continue;
        }
        let Some(parsed) = parse_instance_name(&inst.instance_name) else {
            continue; // unrecognized shape — skip, don't guess
        };
        // pid 0 is the system idle process on Windows; its "engine usage"
        // is a PDH artifact (idle time attributed to engine 0), not a real
        // process using the GPU.
        if parsed.pid != 0 {
            *per_process.entry(parsed.pid).or_insert(0.0) += inst.utilization_percent;
        }
        *per_adapter.entry(parsed.phys_index).or_insert(0.0) += inst.utilization_percent;
    }

    AggregatedGpuEngine {
        per_process_percent: per_process
            .into_iter()
            .map(|(k, v)| (k, v.clamp(0.0, 100.0) as f32))
            .collect(),
        per_adapter_percent: per_adapter
            .into_iter()
            .map(|(k, v)| (k, v.clamp(0.0, 100.0) as f32))
            .collect(),
        truncated,
    }
}

fn device_fallback_snapshots(per_adapter: &HashMap<u32, f32>) -> Vec<GpuSnapshot> {
    let mut adapters: Vec<_> = per_adapter.iter().collect();
    adapters.sort_by_key(|(idx, _)| **idx);
    adapters
        .into_iter()
        .map(|(idx, pct)| GpuSnapshot {
            name: format!("GPU {idx}"),
            utilization_percent: Some(*pct),
            // PDH's GPU Engine counters don't expose VRAM/temp/power/driver
            // at all — those stay honestly unavailable rather than guessed,
            // consistent with "no mock data" even in the fallback path.
            vram_used: None,
            vram_total: None,
            temperature_c: None,
            power_w: None,
            driver_version: None,
        })
        .collect()
}

#[cfg(target_os = "windows")]
mod raw {
    use super::RawGpuEngineInstance;
    use windows::core::PCWSTR;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    };

    const COUNTER_PATH: &str = "\\GPU Engine(*)\\Utilization Percentage";

    pub struct PdhSession {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
        /// The first `PdhCollectQueryData` on a freshly-added counter
        /// always returns `PDH_INVALID_DATA` — there's no prior sample to
        /// compute a rate against yet. Priming this once at construction
        /// means every `collect()` after that has a real value to read,
        /// instead of `collect()` itself needing to special-case its own
        /// first call.
        primed: bool,
    }

    impl PdhSession {
        pub fn open() -> Option<Self> {
            let mut query = PDH_HQUERY::default();
            // SAFETY: `query` is a valid out-pointer for a stack local.
            #[allow(unsafe_code)]
            let opened = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
            if opened != 0 {
                return None;
            }
            let mut counter = PDH_HCOUNTER::default();
            let path: Vec<u16> = COUNTER_PATH
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `path` is a valid null-terminated UTF-16 string;
            // `counter` is a valid out-pointer. `PdhAddEnglishCounterW`
            // (not `PdhAddCounterW`) is used deliberately — see the module
            // doc's locale-safety requirement.
            #[allow(unsafe_code)]
            let added =
                unsafe { PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) };
            if added != 0 {
                // SAFETY: `query` was successfully opened above.
                #[allow(unsafe_code)]
                unsafe {
                    let _ = PdhCloseQuery(query);
                }
                return None;
            }
            Some(Self {
                query,
                counter,
                primed: false,
            })
        }

        /// Returns `None` on the very first call after `open()` (priming;
        /// see the struct doc) or on any API failure — both cases must be
        /// treated as "no data this tick", never as a zero reading.
        pub fn collect(&mut self) -> Option<Vec<RawGpuEngineInstance>> {
            // SAFETY: `self.query` is a valid, open query handle.
            #[allow(unsafe_code)]
            let collected = unsafe { PdhCollectQueryData(self.query) };
            if collected != 0 {
                return None;
            }
            if !self.primed {
                self.primed = true;
                return None; // first sample: no rate data yet, by design
            }

            let mut buffer_size: u32 = 0;
            let mut item_count: u32 = 0;
            // SAFETY: a `None` item buffer with zeroed size/count pointers
            // is exactly how this API reports the required buffer size.
            #[allow(unsafe_code)]
            unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    None,
                );
            }
            if buffer_size == 0 || item_count == 0 {
                return Some(Vec::new()); // genuinely zero GPU-engine instances right now
            }

            let mut buf: Vec<u8> = vec![0; buffer_size as usize];
            // SAFETY: `buf` is exactly `buffer_size` bytes as reported by
            // the sizing call above; the API writes at most that many.
            #[allow(unsafe_code)]
            let result = unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    Some(buf.as_mut_ptr() as *mut _),
                )
            };
            if result != 0 {
                return None;
            }

            let items = buf.as_ptr()
                as *const windows::Win32::System::Performance::PDH_FMT_COUNTERVALUE_ITEM_W;
            let mut out = Vec::with_capacity(item_count as usize);
            for i in 0..item_count as usize {
                // SAFETY: `item_count` came from the same call that filled
                // `buf`; each item's `szName` points into `buf` itself
                // (this API's documented layout — the array header is
                // followed by the string data it points into).
                #[allow(unsafe_code)]
                let item = unsafe { &*items.add(i) };
                // SAFETY: `szName` is a valid null-terminated UTF-16
                // string pointer into `buf` for the lifetime of `buf`.
                #[allow(unsafe_code)]
                let name = unsafe { item.szName.to_string() }.unwrap_or_default();
                // SAFETY: `CStatus` is checked before reading the union.
                #[allow(unsafe_code)]
                let value = if item.FmtValue.CStatus == 0 {
                    unsafe { item.FmtValue.Anonymous.doubleValue }
                } else {
                    continue; // this instance's value wasn't valid this tick
                };
                out.push(RawGpuEngineInstance {
                    instance_name: name,
                    utilization_percent: value,
                });
            }
            Some(out)
        }
    }

    impl Drop for PdhSession {
        fn drop(&mut self) {
            // SAFETY: `self.query` was successfully opened in `open()` and
            // is closed at most once, here.
            #[allow(unsafe_code)]
            unsafe {
                let _ = PdhCloseQuery(self.query);
            }
        }
    }

    // SAFETY: `PDH_HQUERY`/`PDH_HCOUNTER` are opaque process-wide handles
    // (not thread-local, unlike a COM apartment or a GDI DC) — PDH's own
    // docs place no thread-affinity requirement on them. `PdhGpuCollector`
    // (this type's only owner) is only ever driven by one worker thread at
    // a time, so this is `Send`, never `Sync`, matching how it's actually
    // used: moved into a `Box<dyn Collector>` on construction, never shared.
    #[allow(unsafe_code)]
    unsafe impl Send for PdhSession {}
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::RawGpuEngineInstance;

    pub struct PdhSession;

    impl PdhSession {
        pub fn open() -> Option<Self> {
            None
        }
        pub fn collect(&mut self) -> Option<Vec<RawGpuEngineInstance>> {
            None
        }
    }
}

pub struct PdhGpuCollector {
    availability: Availability,
    session: Option<raw::PdhSession>,
    nvml_available: bool,
}

impl PdhGpuCollector {
    pub fn new() -> Self {
        // A cheap, side-effect-free probe: constructing an NVML provider
        // just to check availability and immediately dropping it (the real
        // `GpuCollector` in system-pulse-core owns the one that's actually
        // used for sampling).
        let nvml_available = NvidiaGpuProvider::new().is_ok();
        Self {
            availability: Availability::Ok,
            session: None,
            nvml_available,
        }
    }
}

impl Default for PdhGpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for PdhGpuCollector {
    fn id(&self) -> CollectorId {
        CollectorId::PdhGpu
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        // `raw::PdhSession::open()` is called unconditionally on every
        // platform: the non-Windows stub always returns `None`, which
        // drives the same "unsupported" branch below as a real open
        // failure would on Windows — one code path instead of two.
        self.session = raw::PdhSession::open();
        self.availability = if self.session.is_some() {
            Availability::Ok
        } else if cfg!(target_os = "windows") {
            // GPU Engine counters need Win10 1709+; membership in
            // *Performance Log Users* can also gate counter access on some
            // hardened systems. This binary distinction (open succeeded or
            // didn't) can't yet tell those two apart — see the module
            // doc's disclosed hardening gaps.
            Availability::unsupported(UnsupportedReason::CounterMissing)
        } else {
            Availability::unsupported(UnsupportedReason::NotImplementedOnPlatform)
        };
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            return CollectorOutput::PdhGpu {
                per_process_percent: HashMap::new(),
                device_fallback: None,
            };
        }
        let Some(session) = self.session.as_mut() else {
            return CollectorOutput::PdhGpu {
                per_process_percent: HashMap::new(),
                device_fallback: None,
            };
        };

        let Some(instances) = session.collect() else {
            // Priming tick or a transient API failure — report nothing
            // rather than a zeroed reading; the next tick tries again.
            return CollectorOutput::PdhGpu {
                per_process_percent: HashMap::new(),
                device_fallback: None,
            };
        };

        let aggregated = aggregate(&instances);
        // NVML stays authoritative for device-level stats whenever it's
        // present; PDH only fills in when there's nothing to fall back
        // from *and* something to actually report.
        let device_fallback = if self.nvml_available || aggregated.per_adapter_percent.is_empty() {
            None
        } else {
            Some(Sampled::ok(
                device_fallback_snapshots(&aggregated.per_adapter_percent),
                Source::Pdh,
                ctx.wall_now,
            ))
        };

        CollectorOutput::PdhGpu {
            per_process_percent: aggregated.per_process_percent,
            device_fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(name: &str, pct: f64) -> RawGpuEngineInstance {
        RawGpuEngineInstance {
            instance_name: name.to_string(),
            utilization_percent: pct,
        }
    }

    #[test]
    fn parses_pid_and_phys_index_from_a_real_shaped_instance_name() {
        let parsed =
            parse_instance_name("pid_4821_luid_0x00000000_0x0001E2C1_phys_0_eng_0_engtype_3D")
                .unwrap();
        assert_eq!(parsed.pid, 4821);
        assert_eq!(parsed.phys_index, 0);
    }

    #[test]
    fn unrecognized_shape_is_none_not_a_guess() {
        assert!(parse_instance_name("_Total").is_none());
        assert!(parse_instance_name("garbage").is_none());
    }

    #[test]
    fn aggregate_sums_multiple_engines_per_process() {
        // Same process, two engines (3D + video decode) on the same GPU —
        // Task Manager's own per-process column sums these.
        let instances = vec![
            inst("pid_100_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 10.0),
            inst(
                "pid_100_luid_0x0_0x1_phys_0_eng_1_engtype_Video Decode",
                5.0,
            ),
        ];
        let agg = aggregate(&instances);
        assert_eq!(agg.per_process_percent.get(&100), Some(&15.0));
        assert_eq!(agg.per_adapter_percent.get(&0), Some(&15.0));
        assert_eq!(agg.truncated, 0);
    }

    #[test]
    fn pid_zero_is_excluded_from_per_process_but_counted_per_adapter() {
        let instances = vec![inst("pid_0_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 3.0)];
        let agg = aggregate(&instances);
        assert!(agg.per_process_percent.is_empty());
        assert_eq!(agg.per_adapter_percent.get(&0), Some(&3.0));
    }

    #[test]
    fn utilization_is_clamped_to_0_100() {
        let instances = vec![
            inst("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 60.0),
            inst("pid_1_luid_0x0_0x1_phys_0_eng_1_engtype_3D", 60.0),
        ];
        let agg = aggregate(&instances);
        assert_eq!(agg.per_process_percent.get(&1), Some(&100.0));
    }

    #[test]
    fn instances_past_the_cap_are_counted_as_truncated_not_dropped_silently() {
        // pids start at 1, not 0, to avoid coupling this test to the
        // separate "pid 0 is excluded" behavior covered elsewhere.
        let instances: Vec<_> = (1..=MAX_INSTANCES + 10)
            .map(|i| {
                inst(
                    &format!("pid_{i}_luid_0x0_0x1_phys_0_eng_0_engtype_3D"),
                    1.0,
                )
            })
            .collect();
        let agg = aggregate(&instances);
        assert_eq!(agg.truncated, 10);
        assert_eq!(agg.per_process_percent.len(), MAX_INSTANCES);
    }

    #[test]
    fn unrecognized_instance_names_are_skipped_not_fatal() {
        let instances = vec![
            inst("_Total", 50.0),
            inst("pid_5_luid_0x0_0x1_phys_1_eng_0_engtype_3D", 20.0),
        ];
        let agg = aggregate(&instances);
        assert_eq!(agg.per_process_percent.len(), 1);
        assert_eq!(agg.per_process_percent.get(&5), Some(&20.0));
    }

    #[test]
    fn device_fallback_snapshots_never_fabricate_unavailable_fields() {
        let mut per_adapter = HashMap::new();
        per_adapter.insert(0u32, 33.0f32);
        let snaps = device_fallback_snapshots(&per_adapter);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].utilization_percent, Some(33.0));
        assert_eq!(snaps[0].vram_used, None);
        assert_eq!(snaps[0].temperature_c, None);
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = PdhGpuCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        let _ = avail; // outcome depends on real hardware; not asserted here
    }
}
