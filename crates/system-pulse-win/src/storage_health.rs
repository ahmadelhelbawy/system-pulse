//! Physical drive health (Phase 4): `IOCTL_STORAGE_QUERY_PROPERTY` +
//! `IOCTL_STORAGE_PREDICT_FAILURE` on `\\.\PhysicalDriveN`, no COM.
//!
//! **Deliberate scope limit.** The master plan says "SMART/NVMe health
//! and temps via `DeviceIoControl`" without mandating a specific
//! attribute set. Hand-parsing raw ATA SMART attribute tables (ID 5, 187,
//! 197, 198, ...) and NVMe's separate Health Information Log page would
//! require two entirely different, vendor-quirky parsers to get right,
//! and both encode "is this drive healthy" as a threshold judgement this
//! app would then be making up itself from raw counters. Instead, this
//! collector uses `IOCTL_STORAGE_PREDICT_FAILURE` — the drive
//! firmware/controller's *own* SMART verdict, already normalized across
//! ATA and NVMe by the storage driver stack — for the pass/fail signal,
//! plus `StorageDeviceProperty` (identity: model/serial/bus type/size)
//! and `StorageDeviceTemperatureProperty` (temperature, when the
//! driver reports it). This is a real, documented, cross-vendor Win32
//! surface, not a heuristic layered on raw bytes.
//!
//! Opening a physical drive handle at all requires an elevated process
//! (see the master plan's capability matrix, A2) — this collector reports
//! `NeedsElevation`, never `Failed`, when every open attempt is denied.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::StorageBusType;

const CADENCE: Duration = Duration::from_secs(3600);
/// `\\.\PhysicalDrive0` through `\\.\PhysicalDrive31` — comfortably more
/// than any real machine has; a defensive cap, not a real-world limit.
#[cfg(target_os = "windows")]
const MAX_DRIVES: u32 = 32;

/// `STORAGE_BUS_TYPE` -> the contract enum. Pure and testable without
/// Windows.
pub fn map_bus_type(raw: i32) -> StorageBusType {
    match raw {
        1 => StorageBusType::Scsi,
        3 => StorageBusType::Ata,
        7 => StorageBusType::Usb,
        8 => StorageBusType::Raid,
        10 => StorageBusType::Sas,
        11 => StorageBusType::Sata,
        14 => StorageBusType::Virtual,
        17 => StorageBusType::Nvme,
        _ => StorageBusType::Other,
    }
}

/// Combines a `STORAGE_DEVICE_DESCRIPTOR`'s vendor/product strings into
/// one display model string. Pure and testable without Windows: most
/// consumer NVMe/SATA drives leave vendor empty and put the full model
/// in product; some SCSI-style devices split the two, in which case
/// showing both avoids losing the vendor name.
pub fn combine_model(vendor: Option<&str>, product: Option<&str>) -> Option<String> {
    match (
        vendor.filter(|s| !s.is_empty()),
        product.filter(|s| !s.is_empty()),
    ) {
        (Some(v), Some(p)) if p.to_ascii_lowercase().contains(&v.to_ascii_lowercase()) => {
            Some(p.to_string())
        }
        (Some(v), Some(p)) => Some(format!("{v} {p}")),
        (None, Some(p)) => Some(p.to_string()),
        (Some(v), None) => Some(v.to_string()),
        (None, None) => None,
    }
}

#[cfg(target_os = "windows")]
mod raw {
    use super::{combine_model, map_bus_type};
    use system_pulse_core::types::StorageHealthSnapshot;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceProperty, StorageDeviceTemperatureProperty,
        GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_PREDICT_FAILURE,
        IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PREDICT_FAILURE,
        STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    /// Whether opening physical drives at all failed with access-denied
    /// at least once — the signal this module uses to report
    /// `NeedsElevation` rather than an empty (and thus misleading) list.
    pub struct ReadResult {
        pub drives: Vec<StorageHealthSnapshot>,
        pub any_access_denied: bool,
    }

    /// Bounds-checked read of a null-terminated ASCII string at `offset`
    /// into `buf` — `STORAGE_DEVICE_DESCRIPTOR`'s string offsets are
    /// relative to the descriptor's own start and `0` means "absent";
    /// never trusted to be in-bounds or actually null-terminated without
    /// checking, since a malformed/unusual driver response must degrade
    /// to `None` rather than reading past the buffer.
    fn read_c_str(buf: &[u8], offset: u32) -> Option<String> {
        if offset == 0 {
            return None;
        }
        let start = offset as usize;
        let end = buf.get(start..)?.iter().position(|&b| b == 0)? + start;
        if start >= end {
            return None;
        }
        let s = String::from_utf8_lossy(&buf[start..end]).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    fn query_device_descriptor(
        handle: HANDLE,
    ) -> Option<(
        Option<String>,
        Option<String>,
        Option<super::StorageBusTypeRaw>,
    )> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut buf = vec![0u8; 4096];
        let mut returned = 0u32;
        // SAFETY: `query` is a valid, fully-initialized input struct;
        // `buf` is a 4096-byte output buffer, comfortably larger than any
        // real `STORAGE_DEVICE_DESCRIPTOR` (variable-length strings
        // included) a real drive returns.
        #[allow(unsafe_code)]
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some(&query as *const _ as *const core::ffi::c_void),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                buf.len() as u32,
                Some(&mut returned),
                None,
            )
        };
        if ok.is_err() || (returned as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return None;
        }
        // SAFETY: `buf` was filled by the successful call above with at
        // least a full `STORAGE_DEVICE_DESCRIPTOR` header, checked above.
        #[allow(unsafe_code)]
        let desc = unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
        let vendor = read_c_str(&buf, desc.VendorIdOffset);
        let product = read_c_str(&buf, desc.ProductIdOffset);
        let serial = read_c_str(&buf, desc.SerialNumberOffset);
        let model = combine_model(vendor.as_deref(), product.as_deref());
        Some((model, serial, Some(desc.BusType.0)))
    }

    fn query_temperature(handle: HANDLE) -> Option<i32> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceTemperatureProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut out = STORAGE_TEMPERATURE_DATA_DESCRIPTOR::default();
        let mut returned = 0u32;
        // SAFETY: `query` is a valid input struct; `out` is a
        // correctly-sized output buffer for a descriptor with exactly
        // one `TemperatureInfo` entry (the common case for a single
        // drive's overall temperature — a multi-sensor drive's
        // additional entries are simply not read here).
        #[allow(unsafe_code)]
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some(&query as *const _ as *const core::ffi::c_void),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some(&mut out as *mut _ as *mut core::ffi::c_void),
                std::mem::size_of::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>() as u32,
                Some(&mut returned),
                None,
            )
        };
        if ok.is_err() || out.InfoCount == 0 {
            return None;
        }
        let temp = out.TemperatureInfo[0].Temperature;
        // `STORAGE_TEMPERATURE_VALUE_NOT_REPORTED` (32768) doesn't fit in
        // `i16`; a real "not reported" value is documented as the max
        // representable i16 in practice on this field.
        if temp == i16::MAX {
            None
        } else {
            Some(temp as i32)
        }
    }

    fn query_size(handle: HANDLE) -> Option<u64> {
        let mut out = GET_LENGTH_INFORMATION::default();
        let mut returned = 0u32;
        // SAFETY: `out` is a correctly-sized output buffer for
        // `GET_LENGTH_INFORMATION`; no input buffer is required by this
        // IOCTL.
        #[allow(unsafe_code)]
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_DISK_GET_LENGTH_INFO,
                None,
                0,
                Some(&mut out as *mut _ as *mut core::ffi::c_void),
                std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
                Some(&mut returned),
                None,
            )
        };
        if ok.is_err() || out.Length < 0 {
            return None;
        }
        Some(out.Length as u64)
    }

    fn query_predicted_failure(handle: HANDLE) -> Option<bool> {
        let mut out = STORAGE_PREDICT_FAILURE::default();
        let mut returned = 0u32;
        // SAFETY: `out` is a correctly-sized output buffer for
        // `STORAGE_PREDICT_FAILURE`; no input buffer is required by this
        // IOCTL.
        #[allow(unsafe_code)]
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_PREDICT_FAILURE,
                None,
                0,
                Some(&mut out as *mut _ as *mut core::ffi::c_void),
                std::mem::size_of::<STORAGE_PREDICT_FAILURE>() as u32,
                Some(&mut returned),
                None,
            )
        };
        if ok.is_err() {
            return None;
        }
        Some(out.PredictFailure != 0)
    }

    pub fn read() -> ReadResult {
        let mut drives = Vec::new();
        let mut any_access_denied = false;

        for i in 0..super::MAX_DRIVES {
            let device = format!(r"\\.\PhysicalDrive{i}");
            let wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: `wide` is a valid null-terminated UTF-16 path;
            // opening for read/share-all is the minimum access this
            // query needs (never write access — this collector is
            // read-only by construction).
            #[allow(unsafe_code)]
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(wide.as_ptr()),
                    windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    Default::default(),
                    None,
                )
            };
            let handle = match handle {
                Ok(h) => h,
                Err(e) => {
                    if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
                        any_access_denied = true;
                    }
                    continue; // no such drive at this index, or denied — either way, nothing to add
                }
            };

            let (model, serial, bus_type_raw) =
                query_device_descriptor(handle).unwrap_or((None, None, None));
            let size_bytes = query_size(handle);
            let temperature_c = query_temperature(handle);
            let predicted_failure = query_predicted_failure(handle);

            // SAFETY: `handle` was successfully opened above and is
            // closed exactly once, here.
            #[allow(unsafe_code)]
            unsafe {
                let _ = CloseHandle(handle);
            }

            drives.push(StorageHealthSnapshot {
                device,
                model,
                serial,
                bus_type: bus_type_raw.map(map_bus_type),
                size_bytes,
                temperature_c,
                predicted_failure,
            });
        }

        ReadResult {
            drives,
            any_access_denied,
        }
    }
}

// A tiny type alias so `raw`'s helper functions don't need to name the
// full `windows` crate type in their public-ish (crate-internal) surface.
#[cfg(target_os = "windows")]
type StorageBusTypeRaw = i32;

#[cfg(not(target_os = "windows"))]
mod raw {
    use system_pulse_core::types::StorageHealthSnapshot;

    pub struct ReadResult {
        pub drives: Vec<StorageHealthSnapshot>,
        pub any_access_denied: bool,
    }

    pub fn read() -> ReadResult {
        ReadResult {
            drives: Vec::new(),
            any_access_denied: false,
        }
    }
}

pub struct StorageHealthCollector {
    availability: Availability,
}

impl StorageHealthCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for StorageHealthCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for StorageHealthCollector {
    fn id(&self) -> CollectorId {
        CollectorId::StorageHealth
    }

    fn cadence(&self) -> Cadence {
        Cadence::Cold(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        Privilege::Admin
    }

    fn probe(&mut self) -> Availability {
        #[cfg(target_os = "windows")]
        {
            self.availability = Availability::Ok;
        }
        #[cfg(not(target_os = "windows"))]
        {
            self.availability = Availability::unsupported(
                system_pulse_core::model::UnsupportedReason::NotImplementedOnPlatform,
            );
        }
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            return CollectorOutput::StorageHealth(Sampled::unavailable(
                self.availability.clone(),
                Source::StorageIoctl,
                ctx.wall_now,
            ));
        }
        let result = raw::read();
        let sampled = if result.drives.is_empty() && result.any_access_denied {
            // At least one physical drive exists (we got access-denied,
            // not "file not found") but none could be opened — this
            // process needs elevation, not a genuinely empty disk list.
            Sampled::unavailable(
                Availability::NeedsElevation,
                Source::StorageIoctl,
                ctx.wall_now,
            )
        } else if result.drives.is_empty() {
            // No access-denied signal at all and still nothing: on a
            // real Windows host this only happens on the non-Windows
            // stub, never for real — but it's not this collector's place
            // to assert that, only to report what it actually saw.
            Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::StorageIoctl,
                ctx.wall_now,
            )
        } else {
            Sampled::ok(result.drives, Source::StorageIoctl, ctx.wall_now)
        };
        CollectorOutput::StorageHealth(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_bus_type() {
        assert_eq!(map_bus_type(1), StorageBusType::Scsi);
        assert_eq!(map_bus_type(3), StorageBusType::Ata);
        assert_eq!(map_bus_type(7), StorageBusType::Usb);
        assert_eq!(map_bus_type(8), StorageBusType::Raid);
        assert_eq!(map_bus_type(10), StorageBusType::Sas);
        assert_eq!(map_bus_type(11), StorageBusType::Sata);
        assert_eq!(map_bus_type(14), StorageBusType::Virtual);
        assert_eq!(map_bus_type(17), StorageBusType::Nvme);
    }

    #[test]
    fn unknown_bus_type_falls_back_to_other_not_a_panic() {
        assert_eq!(map_bus_type(0), StorageBusType::Other);
        assert_eq!(map_bus_type(999), StorageBusType::Other);
    }

    #[test]
    fn combine_model_prefers_product_when_it_already_contains_vendor() {
        let m = combine_model(Some("Samsung"), Some("Samsung SSD 980 PRO 1TB"));
        assert_eq!(m, Some("Samsung SSD 980 PRO 1TB".to_string()));
    }

    #[test]
    fn combine_model_joins_distinct_vendor_and_product() {
        let m = combine_model(Some("ACME"), Some("Widget9000"));
        assert_eq!(m, Some("ACME Widget9000".to_string()));
    }

    #[test]
    fn combine_model_falls_back_to_whichever_field_is_present() {
        assert_eq!(
            combine_model(None, Some("OnlyProduct")),
            Some("OnlyProduct".to_string())
        );
        assert_eq!(
            combine_model(Some("OnlyVendor"), None),
            Some("OnlyVendor".to_string())
        );
        assert_eq!(combine_model(None, None), None);
    }

    #[test]
    fn combine_model_treats_empty_strings_as_absent() {
        assert_eq!(
            combine_model(Some(""), Some("Product")),
            Some("Product".to_string())
        );
        assert_eq!(combine_model(Some(""), Some("")), None);
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = StorageHealthCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = StorageHealthCollector::new();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis(0),
        };
        match c.collect(&ctx) {
            CollectorOutput::StorageHealth(_) => {}
            _ => panic!("expected StorageHealth output"),
        }
    }
}
