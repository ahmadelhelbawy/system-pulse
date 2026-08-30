//! Optional sensor bridge (Phase 4) — reads LibreHardwareMonitor's WMI
//! namespace (`root\LibreHardwareMonitor`, exposed automatically while
//! LHM is running) if present. Never installs, launches, or otherwise
//! manages LibreHardwareMonitor itself; if it isn't already running with
//! that namespace exposed, this reports `Unsupported`, not an error.
//!
//! **Deliberate scope limit.** The master plan names both
//! LibreHardwareMonitor (WMI) and HWiNFO (a named shared-memory segment,
//! `Global\HWiNFO_SENS_SM2`, with its own documented binary struct
//! layout) as acceptable bridge sources. Only the WMI one is implemented
//! here: it reuses the exact WMI connection pattern already proven safe
//! by the COM/WebView2 spike (`crate::com_spike`) and by Task Scheduler
//! (Phase 3), whereas HWiNFO's shared-memory protocol is a distinct
//! parser with no code to share. Per the plan's own framing ("a sensor
//! source", not "every sensor source"), one working, real bridge
//! satisfies the requirement; HWiNFO support is a clearly separable
//! follow-up, not started here.
//!
//! Self-contained per `collect()` call, like `scheduled_tasks` — COM is
//! initialized, used, and torn down within one call on whatever thread
//! it runs on, so no COM interface ever needs to be `Send` or to survive
//! across a tick.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, Sampled, Source, UnsupportedReason};

/// Not 1 Hz (WMI is too slow for the hot path, per the master plan's own
/// "WMI... must never touch the 1Hz loop" rule) but fresher than the
/// hourly Cold default other Phase 3/4 inventory collectors use — sensor
/// readings (temperatures especially) are meaningfully time-varying in a
/// way a static inventory list isn't.
const CADENCE: Duration = Duration::from_secs(10);

#[cfg(target_os = "windows")]
const LHM_SOURCE_NAME: &str = "LibreHardwareMonitor";

#[cfg(target_os = "windows")]
mod raw {
    use super::LHM_SOURCE_NAME;
    use system_pulse_core::types::{SensorBridgeSnapshot, SensorReading};
    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VT_R4, VT_R8};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
        WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE,
    };

    /// Reads a `BSTR`-typed property's string value. `None` for any other
    /// variant type rather than a coerced/guessed string.
    fn variant_to_string(v: &VARIANT) -> Option<String> {
        // SAFETY: reading the `vt` discriminant of a `VARIANT` is always
        // valid; it's the same for every union arm.
        #[allow(unsafe_code)]
        let vt = unsafe { v.Anonymous.Anonymous.vt };
        if vt != VT_BSTR {
            return None;
        }
        // SAFETY: `vt == VT_BSTR` was just checked, so the `bstrVal` arm
        // of the union is the one that was actually written.
        #[allow(unsafe_code)]
        let bstr = unsafe { &v.Anonymous.Anonymous.Anonymous.bstrVal };
        if bstr.is_empty() {
            None
        } else {
            Some(bstr.to_string())
        }
    }

    /// Reads a numeric property as `f64`, accepting the two float variant
    /// types WMI actually uses for measurement values. `None` for any
    /// other type rather than a coerced/guessed number.
    fn variant_to_f64(v: &VARIANT) -> Option<f64> {
        // SAFETY: reading the `vt` discriminant is always valid.
        #[allow(unsafe_code)]
        let vt = unsafe { v.Anonymous.Anonymous.vt };
        // SAFETY: each arm is read only after confirming `vt` matches the
        // union member that was actually written.
        #[allow(unsafe_code)]
        unsafe {
            if vt == VT_R4 {
                Some(v.Anonymous.Anonymous.Anonymous.fltVal as f64)
            } else if vt == VT_R8 {
                Some(v.Anonymous.Anonymous.Anonymous.dblVal)
            } else {
                None
            }
        }
    }

    fn get_string(obj: &IWbemClassObject, name: &str) -> Option<String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut val = VARIANT::default();
        // SAFETY: `obj` is a valid, live `IWbemClassObject`; `wide` is a
        // valid null-terminated UTF-16 property name; `val` is a valid
        // out-pointer for the property's value.
        #[allow(unsafe_code)]
        let ok = unsafe {
            obj.Get(
                windows::core::PCWSTR(wide.as_ptr()),
                0,
                &mut val,
                None,
                None,
            )
        };
        if ok.is_err() {
            return None;
        }
        variant_to_string(&val)
    }

    fn get_f64(obj: &IWbemClassObject, name: &str) -> Option<f64> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut val = VARIANT::default();
        // SAFETY: same as `get_string` above.
        #[allow(unsafe_code)]
        let ok = unsafe {
            obj.Get(
                windows::core::PCWSTR(wide.as_ptr()),
                0,
                &mut val,
                None,
                None,
            )
        };
        if ok.is_err() {
            return None;
        }
        variant_to_f64(&val)
    }

    /// `None` if LibreHardwareMonitor's WMI namespace isn't reachable at
    /// all (not currently running, or running without WMI exposure) —
    /// the "absent bridge" case, distinct from `Some(empty)` (running,
    /// but reporting zero sensors, which is also a legitimate real state
    /// this function passes through unchanged).
    pub fn read() -> Option<SensorBridgeSnapshot> {
        // SAFETY: fresh call, self-contained — see the module doc for why
        // this never shares COM state across calls or threads.
        #[allow(unsafe_code)]
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if init.is_err() {
            return None;
        }

        let result = (|| {
            // SAFETY: standard `CoCreateInstance` usage for the
            // documented WMI entry point.
            #[allow(unsafe_code)]
            let locator: IWbemLocator =
                unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.ok()?;
            // SAFETY: `ConnectServer` per its documented signature; empty
            // user/password/locale means "use the caller's own token" —
            // if LibreHardwareMonitor isn't running (or wasn't started
            // with WMI exposure), this namespace simply doesn't exist and
            // the call fails, which this function correctly reports as
            // "bridge absent" via the `?` below.
            #[allow(unsafe_code)]
            let services: IWbemServices = unsafe {
                locator.ConnectServer(
                    &BSTR::from(r"root\LibreHardwareMonitor"),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
            }
            .ok()?;
            // SAFETY: `CoSetProxyBlanket` on the freshly-acquired proxy —
            // the same per-proxy pattern used throughout this crate
            // rather than any process-wide security call.
            #[allow(unsafe_code)]
            unsafe {
                CoSetProxyBlanket(
                    &services,
                    RPC_C_AUTHN_WINNT,
                    RPC_C_AUTHZ_NONE,
                    None,
                    RPC_C_AUTHN_LEVEL_CALL,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    None,
                    EOAC_NONE,
                )
            }
            .ok()?;

            // SAFETY: a minimal, read-only WQL query against the
            // documented `Sensor` class LibreHardwareMonitor exposes.
            #[allow(unsafe_code)]
            let enumerator = unsafe {
                services.ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from("SELECT Name, SensorType, Value FROM Sensor"),
                    WBEM_GENERIC_FLAG_TYPE(
                        WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0,
                    ),
                    None,
                )
            }
            .ok()?;

            let mut readings = Vec::new();
            loop {
                let mut row = [None; 1];
                let mut returned = 0u32;
                // SAFETY: `enumerator` is a valid, live enumerator; `row`
                // is a 1-element buffer for `Next` to fill, exactly its
                // documented single-row-fetch idiom.
                #[allow(unsafe_code)]
                let next = unsafe { enumerator.Next(WBEM_INFINITE, &mut row, &mut returned) };
                if next.is_err() || returned == 0 {
                    break;
                }
                let Some(obj) = row[0].take() else { break };
                let (Some(name), Some(value)) = (get_string(&obj, "Name"), get_f64(&obj, "Value"))
                else {
                    continue; // a malformed/partial row is skipped, not fabricated
                };
                let kind = get_string(&obj, "SensorType").unwrap_or_else(|| "Other".to_string());
                readings.push(SensorReading { name, kind, value });
            }

            Some(SensorBridgeSnapshot {
                source: Some(LHM_SOURCE_NAME.to_string()),
                readings,
            })
        })();

        // SAFETY: matches the successful `CoInitializeEx` above, same
        // thread, after all COM use in this call is finished.
        #[allow(unsafe_code)]
        unsafe {
            CoUninitialize()
        };
        result
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use system_pulse_core::types::SensorBridgeSnapshot;

    pub fn read() -> Option<SensorBridgeSnapshot> {
        None
    }
}

pub struct SensorBridgeCollector {
    availability: Availability,
}

impl SensorBridgeCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for SensorBridgeCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for SensorBridgeCollector {
    fn id(&self) -> CollectorId {
        CollectorId::SensorBridge
    }

    fn cadence(&self) -> Cadence {
        Cadence::Cold(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        // No elevation needed: LibreHardwareMonitor's WMI namespace is
        // readable by a standard user while it's running.
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        #[cfg(target_os = "windows")]
        {
            self.availability = Availability::Ok;
        }
        #[cfg(not(target_os = "windows"))]
        {
            self.availability =
                Availability::unsupported(UnsupportedReason::NotImplementedOnPlatform);
        }
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            return CollectorOutput::SensorBridge(Sampled::unavailable(
                self.availability.clone(),
                Source::SensorBridge,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            Some(snapshot) => Sampled::ok(snapshot, Source::SensorBridge, ctx.wall_now),
            None => {
                #[cfg(target_os = "windows")]
                let availability = Availability::unsupported(UnsupportedReason::DriverAbsent);
                #[cfg(not(target_os = "windows"))]
                let availability =
                    Availability::unsupported(UnsupportedReason::NotImplementedOnPlatform);
                Sampled::unavailable(availability, Source::SensorBridge, ctx.wall_now)
            }
        };
        CollectorOutput::SensorBridge(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = SensorBridgeCollector::new();
        let avail = c.probe();
        assert!(!avail.is_ok() || cfg!(target_os = "windows"));
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = SensorBridgeCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::SensorBridge(_s) => {
                #[cfg(not(target_os = "windows"))]
                {
                    assert_eq!(_s.value, None);
                    assert!(!_s.availability.is_ok());
                }
            }
            _ => panic!("expected SensorBridge output"),
        }
    }

    #[test]
    fn absent_bridge_is_unsupported_never_ok_with_empty_readings() {
        // A collector reporting `Ok(SensorBridgeSnapshot { readings: [],
        // .. })` when the bridge isn't running at all would look
        // identical to "found it, zero sensors" — this is the contract
        // guarding against exactly that conflation on the non-Windows
        // stub, which always represents "bridge absent."
        let mut c = SensorBridgeCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        #[cfg(not(target_os = "windows"))]
        match c.collect(&ctx) {
            CollectorOutput::SensorBridge(s) => assert!(!s.availability.is_ok()),
            _ => panic!("expected SensorBridge output"),
        }
        #[cfg(target_os = "windows")]
        let _ = c.collect(&ctx);
    }
}
