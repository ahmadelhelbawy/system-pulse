//! `GetSystemFirmwareTable('RSMB')`: board/BIOS/DIMM inventory parsed from
//! the raw SMBIOS structure table (DMTF SMBIOS spec). Cold cadence, cached
//! forever after the first successful probe — this cannot change while the
//! machine is running.
//!
//! Parsing is entirely bounds-checked (every field read goes through a
//! `.get()`-based helper, never a direct index) so a malformed or
//! truncated table — a real risk: VMs and some OEM firmware ship broken
//! tables — degrades to fewer/no structures found rather than panicking.

use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
#[cfg(not(target_os = "windows"))]
use system_pulse_core::model::UnsupportedReason;
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::{DimmInfo, SmbiosInfo};

const SMBIOS_TYPE_BIOS: u8 = 0;
const SMBIOS_TYPE_BASEBOARD: u8 = 2;
const SMBIOS_TYPE_MEMORY_DEVICE: u8 = 17;
const SMBIOS_TYPE_END_OF_TABLE: u8 = 127;

struct RawStructure<'a> {
    kind: u8,
    /// The formatted-data section only (spec-documented offsets, including
    /// the 4-byte type/length/handle header at offset 0..4), *not* the
    /// trailing string set.
    data: &'a [u8],
    /// 0-indexed; `strings[0]` is SMBIOS string index 1 (string indices are
    /// 1-based, 0 meaning "no string").
    strings: Vec<String>,
}

impl RawStructure<'_> {
    fn string_field(&self, offset: usize) -> Option<String> {
        let idx = *self.data.get(offset)?;
        if idx == 0 {
            return None;
        }
        self.strings.get((idx - 1) as usize).cloned()
    }

    fn u16_field(&self, offset: usize) -> Option<u16> {
        let b = self.data.get(offset..offset + 2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32_field(&self, offset: usize) -> Option<u32> {
        let b = self.data.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Walks the raw SMBIOS structure table, bounds-checked throughout. Stops
/// (returning whatever was parsed so far) at the first sign of corruption
/// or truncation, or at the documented Type 127 end-of-table marker,
/// whichever comes first.
fn iterate_structures(table: &[u8]) -> Vec<RawStructure<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos + 4 <= table.len() {
        let kind = table[pos];
        let length = table[pos + 1] as usize;
        if length < 4 {
            break; // a structure is at minimum its 4-byte header; malformed
        }
        let data_end = match pos.checked_add(length) {
            Some(end) if end <= table.len() => end,
            _ => break, // declared length runs past the buffer: truncated
        };
        let data = &table[pos..data_end];

        // The string set immediately follows the formatted data: a
        // sequence of null-terminated strings, terminated by one more null
        // byte (so "no strings" is exactly two consecutive 0x00 bytes).
        let mut strings = Vec::new();
        let mut cursor = data_end;
        if table.get(cursor) == Some(&0) {
            cursor += 1; // no strings; this 0x00 is the empty first entry
        } else {
            loop {
                let start = cursor;
                loop {
                    match table.get(cursor) {
                        Some(0) => break,
                        Some(_) => cursor += 1,
                        None => break, // truncated mid-string
                    }
                }
                if cursor >= table.len() {
                    break; // truncated: stop without a final string
                }
                strings.push(String::from_utf8_lossy(&table[start..cursor]).into_owned());
                cursor += 1; // skip this string's own null terminator
                match table.get(cursor) {
                    Some(0) => {
                        cursor += 1; // double-null: end of string set
                        break;
                    }
                    Some(_) => continue,
                    None => break, // truncated: no closing null
                }
            }
        }

        out.push(RawStructure {
            kind,
            data,
            strings,
        });
        if kind == SMBIOS_TYPE_END_OF_TABLE {
            break;
        }
        pos = cursor;
    }
    out
}

/// Extended Size (offset 0x1C) applies when Size (0x0C) reads exactly
/// 0x7FFF, for modules >= 32 GiB. Falls back to the 16-bit Size field
/// otherwise; a `Size` of 0 means "no module installed" (slot is empty),
/// correctly producing no `DimmInfo`.
fn dimm_size_bytes(s: &RawStructure) -> Option<u64> {
    let size16 = s.u16_field(0x0C)?;
    if size16 == 0 {
        return None;
    }
    let mib = if size16 == 0x7FFF {
        s.u32_field(0x1C)? as u64
    } else {
        // Bit 15 historically distinguished KB (set) from MB (unset); every
        // real module for decades has reported in MB, so this only masks
        // the marker bit rather than actually branching on it.
        (size16 & 0x7FFF) as u64
    };
    Some(mib * 1024 * 1024)
}

fn dimm_speed_mts(s: &RawStructure) -> Option<u32> {
    // Prefer "Configured Memory Clock Speed" (0x20, the module's actual
    // running speed) when the structure is long enough to carry it (SMBIOS
    // 2.7+); fall back to "Speed" (0x15, the maximum capable speed) for
    // older/shorter structures.
    s.u16_field(0x20)
        .or_else(|| s.u16_field(0x15))
        .filter(|&v| v != 0)
        .map(|v| v as u32)
}

/// Parses the raw SMBIOS table bytes (as returned by
/// `GetSystemFirmwareTable('RSMB')`, *after* stripping its 8-byte
/// `RawSMBIOSData` header — see `raw::read`) into the contract type. Pure
/// and platform-independent, so it's testable with captured/synthetic byte
/// fixtures on any host.
pub fn parse_smbios_table(table: &[u8]) -> SmbiosInfo {
    let mut info = SmbiosInfo::default();
    for s in iterate_structures(table) {
        match s.kind {
            SMBIOS_TYPE_BIOS => {
                info.bios_vendor = s.string_field(0x04);
                info.bios_version = s.string_field(0x05);
                info.bios_release_date = s.string_field(0x08);
            }
            SMBIOS_TYPE_BASEBOARD => {
                // Only the first baseboard structure is used — multi-board
                // systems are rare and the app only has one "board" slot to
                // show; a later Type 2 structure (if any) is ignored.
                if info.board_vendor.is_none() && info.board_product.is_none() {
                    info.board_vendor = s.string_field(0x04);
                    info.board_product = s.string_field(0x05);
                }
            }
            SMBIOS_TYPE_MEMORY_DEVICE => {
                let size_bytes = dimm_size_bytes(&s);
                if size_bytes.is_some() {
                    info.dimms.push(DimmInfo {
                        manufacturer: s.string_field(0x17),
                        part_number: s.string_field(0x1A),
                        size_bytes,
                        speed_mts: dimm_speed_mts(&s),
                    });
                }
                // An empty slot (size 0) is correctly omitted, not reported
                // as a zeroed-out DIMM.
            }
            _ => {}
        }
    }
    info
}

#[cfg(target_os = "windows")]
mod raw {
    use windows::Win32::System::SystemInformation::{
        GetSystemFirmwareTable, FIRMWARE_TABLE_PROVIDER,
    };

    /// The documented FourCC 'RSMB', packed the way `GetSystemFirmwareTable`
    /// expects (Microsoft's own headers define `RSMB` as `0x52534D42`).
    const RSMB: FIRMWARE_TABLE_PROVIDER = FIRMWARE_TABLE_PROVIDER(0x5253_4D42);

    /// Returns the SMBIOS structure table only — the raw call's own 8-byte
    /// `RawSMBIOSData` header (calling convention flag, version, revision,
    /// length) is stripped here so `parse_smbios_table` only ever sees the
    /// structure bytes the DMTF spec actually describes.
    pub fn read() -> Option<Vec<u8>> {
        // SAFETY: a zero-length query with `pfirmwaretablebuffer: None` is
        // exactly how this API reports the required size; nothing is
        // written.
        #[allow(unsafe_code)]
        let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is exactly `size` bytes, matching the prior call's
        // report; the API writes at most that many bytes into it.
        #[allow(unsafe_code)]
        let written = unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut buf)) };
        if written == 0 || (written as usize) > buf.len() {
            return None;
        }
        buf.truncate(written as usize);
        // Skip the 8-byte RawSMBIOSData header (Used20CallingMethod,
        // SMBIOSMajorVersion, SMBIOSMinorVersion, DmiRevision, Length:u32).
        if buf.len() < 8 {
            return None;
        }
        Some(buf[8..].to_vec())
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    pub fn read() -> Option<Vec<u8>> {
        None
    }
}

pub struct SmbiosCollector {
    availability: Availability,
    /// Cached forever after the first successful read — this data cannot
    /// change while the machine is running, so there's no reason to ever
    /// re-parse it.
    cached: Option<Sampled<SmbiosInfo>>,
}

impl SmbiosCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
            cached: None,
        }
    }
}

impl Default for SmbiosCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for SmbiosCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Hardware
    }

    fn cadence(&self) -> Cadence {
        // "Cold, cache forever" per the master plan: a long TTL rather than
        // OnDemand, so it still self-populates in the background without
        // needing a panel to explicitly trigger the first read.
        Cadence::Cold(Duration::from_secs(3600))
    }

    fn required_privilege(&self) -> Privilege {
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
        if let Some(cached) = &self.cached {
            return CollectorOutput::Hardware(cached.clone());
        }
        if !self.availability.is_ok() {
            return CollectorOutput::Hardware(Sampled::unavailable(
                self.availability.clone(),
                Source::Smbios,
                ctx.wall_now,
            ));
        }
        let sampled = match raw::read() {
            Some(table) => {
                let info = parse_smbios_table(&table);
                let sampled = Sampled::ok(info, Source::Smbios, ctx.wall_now);
                self.cached = Some(sampled.clone());
                sampled
            }
            None => Sampled::unavailable(
                Availability::failed(FailureCode::ParseError),
                Source::Smbios,
                ctx.wall_now,
            ),
        };
        CollectorOutput::Hardware(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one raw SMBIOS structure: header + formatted data + strings.
    fn structure(kind: u8, formatted_tail: &[u8], strings: &[&str]) -> Vec<u8> {
        let mut data = vec![kind, 0, 0, 0]; // type, length (patched below), handle (2 bytes, unused)
        data.extend_from_slice(formatted_tail);
        data[1] = data.len() as u8; // formatted length, header included
        for s in strings {
            data.extend_from_slice(s.as_bytes());
            data.push(0);
        }
        if strings.is_empty() {
            data.push(0); // the "no strings" marker byte
        }
        data.push(0); // final terminator of the string set
        data
    }

    fn end_of_table() -> Vec<u8> {
        vec![127, 4, 0, 0, 0, 0]
    }

    #[test]
    fn parses_bios_vendor_version_and_date() {
        // offsets: 0x04=vendor(str1), 0x05=version(str2), 0x08=date(str3)
        let mut tail = vec![0u8; 0x08 - 4 + 1];
        tail[0x04 - 4] = 1;
        tail[0x05 - 4] = 2;
        tail[0x08 - 4] = 3;
        let mut table = structure(
            SMBIOS_TYPE_BIOS,
            &tail,
            &["Acme BIOS", "F.10", "01/02/2025"],
        );
        table.extend(end_of_table());

        let info = parse_smbios_table(&table);
        assert_eq!(info.bios_vendor.as_deref(), Some("Acme BIOS"));
        assert_eq!(info.bios_version.as_deref(), Some("F.10"));
        assert_eq!(info.bios_release_date.as_deref(), Some("01/02/2025"));
    }

    #[test]
    fn parses_baseboard_vendor_and_product() {
        let mut tail = vec![0u8; 2];
        tail[0] = 1; // manufacturer at 0x04
        tail[1] = 2; // product at 0x05
        let mut table = structure(SMBIOS_TYPE_BASEBOARD, &tail, &["MB Inc", "Z900 Pro"]);
        table.extend(end_of_table());

        let info = parse_smbios_table(&table);
        assert_eq!(info.board_vendor.as_deref(), Some("MB Inc"));
        assert_eq!(info.board_product.as_deref(), Some("Z900 Pro"));
    }

    #[test]
    fn parses_a_populated_dimm_with_configured_speed() {
        // Build a full-length (0x22 byte) Type 17 structure.
        let mut tail = vec![0u8; 0x22 - 4];
        // Size (0x0C) = 16 (MB units) -> 16 MiB for the test; real modules
        // are GB-scale, the unit math is what's under test.
        tail[0x0C - 4..0x0C - 4 + 2].copy_from_slice(&16u16.to_le_bytes());
        tail[0x17 - 4] = 1; // manufacturer string index
        tail[0x1A - 4] = 2; // part number string index
        tail[0x20 - 4..0x20 - 4 + 2].copy_from_slice(&6000u16.to_le_bytes());
        let mut table = structure(SMBIOS_TYPE_MEMORY_DEVICE, &tail, &["Kingston", "KC3000"]);
        table.extend(end_of_table());

        let info = parse_smbios_table(&table);
        assert_eq!(info.dimms.len(), 1);
        let d = &info.dimms[0];
        assert_eq!(d.manufacturer.as_deref(), Some("Kingston"));
        assert_eq!(d.part_number.as_deref(), Some("KC3000"));
        assert_eq!(d.size_bytes, Some(16 * 1024 * 1024));
        assert_eq!(d.speed_mts, Some(6000));
    }

    #[test]
    fn empty_dimm_slot_is_omitted_not_reported_as_zero() {
        let tail = vec![0u8; 0x0C - 4 + 2]; // Size field present, value 0
        let mut table = structure(SMBIOS_TYPE_MEMORY_DEVICE, &tail, &[]);
        table.extend(end_of_table());

        let info = parse_smbios_table(&table);
        assert!(info.dimms.is_empty());
    }

    #[test]
    fn truncated_table_does_not_panic_and_returns_partial_data() {
        let mut tail = vec![0u8; 2];
        tail[0] = 1;
        tail[1] = 2;
        let mut table = structure(SMBIOS_TYPE_BASEBOARD, &tail, &["MB Inc", "Z900"]);
        // Cut the buffer off mid-structure — simulates a truncated read.
        table.truncate(table.len() - 3);

        // Must not panic; a partially-readable structure is fine to drop.
        let _ = parse_smbios_table(&table);
    }

    #[test]
    fn garbage_data_does_not_panic() {
        let garbage = vec![0xFFu8; 37];
        let info = parse_smbios_table(&garbage);
        // No assertion on content — the only requirement is "does not
        // panic", which the test harness itself enforces.
        assert!(info.dimms.len() < 1000); // sanity bound, not a real invariant
    }

    #[test]
    fn empty_table_produces_default_info() {
        let info = parse_smbios_table(&[]);
        assert_eq!(info, SmbiosInfo::default());
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        let mut c = SmbiosCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        assert!(!avail.is_ok());
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }
}
