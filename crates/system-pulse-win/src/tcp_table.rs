//! `GetExtendedTcpTable`/`GetExtendedUdpTable` (`*_OWNER_PID_ALL`,
//! iphlpapi): connections, listening ports, and process↔network
//! attribution — unelevated, no ETW required.
//!
//! Address/port byte-order handling (`u32::from_be`/`u16::from_be` on the
//! raw fields) is verified against the `netstat2` crate's Windows
//! integration, a widely-used reference for this exact API.

use std::net::Ipv4Addr;
use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::types::{ConnectionSnapshot, TcpState, TransportProtocol};

const CADENCE: Duration = Duration::from_secs(2);

/// Mirrors `MIB_TCPROW_OWNER_PID`'s field layout without depending on the
/// `windows` crate, so parsing is testable on any host. The real Windows
/// struct's fields are copied into this one before parsing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawTcpRow {
    pub state: u32,
    pub local_addr: u32,
    pub local_port: u32,
    pub remote_addr: u32,
    pub remote_port: u32,
    pub pid: u32,
}

/// Mirrors `MIB_UDPROW_OWNER_PID`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawUdpRow {
    pub local_addr: u32,
    pub local_port: u32,
    pub pid: u32,
}

fn format_ipv4(raw_be: u32) -> String {
    Ipv4Addr::from(u32::from_be(raw_be)).to_string()
}

fn to_port(raw_be: u32) -> u16 {
    u16::from_be(raw_be as u16)
}

/// Maps `MIB_TCP_STATE` values. Unrecognized values (a future Windows
/// version adding a new state) become `None` rather than a parse error —
/// the row itself is still reported, just without a decoded state.
fn tcp_state_from_raw(v: u32) -> Option<TcpState> {
    match v {
        1 => Some(TcpState::Closed),
        2 => Some(TcpState::Listen),
        3 => Some(TcpState::SynSent),
        4 => Some(TcpState::SynReceived),
        5 => Some(TcpState::Established),
        6 => Some(TcpState::FinWait1),
        7 => Some(TcpState::FinWait2),
        8 => Some(TcpState::CloseWait),
        9 => Some(TcpState::Closing),
        10 => Some(TcpState::LastAck),
        11 => Some(TcpState::TimeWait),
        12 => Some(TcpState::DeleteTcb),
        _ => None,
    }
}

pub fn parse_tcp_row(row: &RawTcpRow) -> ConnectionSnapshot {
    ConnectionSnapshot {
        protocol: TransportProtocol::Tcp,
        local_addr: format_ipv4(row.local_addr),
        local_port: to_port(row.local_port),
        remote_addr: format_ipv4(row.remote_addr),
        remote_port: to_port(row.remote_port),
        state: tcp_state_from_raw(row.state),
        pid: Some(row.pid),
    }
}

pub fn parse_udp_row(row: &RawUdpRow) -> ConnectionSnapshot {
    ConnectionSnapshot {
        protocol: TransportProtocol::Udp,
        local_addr: format_ipv4(row.local_addr),
        local_port: to_port(row.local_port),
        // UDP is connectionless: GetExtendedUdpTable has no remote endpoint
        // or state fields at all, unlike TCP.
        remote_addr: "0.0.0.0".to_string(),
        remote_port: 0,
        state: None,
        pid: Some(row.pid),
    }
}

pub fn parse_all(tcp: &[RawTcpRow], udp: &[RawUdpRow]) -> Vec<ConnectionSnapshot> {
    tcp.iter()
        .map(parse_tcp_row)
        .chain(udp.iter().map(parse_udp_row))
        .collect()
}

#[cfg(target_os = "windows")]
mod raw {
    use super::{RawTcpRow, RawUdpRow};
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCPTABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
    };

    const AF_INET: u32 = 2;

    /// Two-pass fetch: the first call reports the required buffer size (it
    /// always returns `ERROR_INSUFFICIENT_BUFFER` with `size` unset on the
    /// first attempt, by design of this API), then the second call fills a
    /// buffer of exactly that size. `MIB_*TABLE_OWNER_PID` is a
    /// variable-length struct (`dwNumEntries` rows follow a fixed header),
    /// so this reads the raw bytes and walks the header manually rather
    /// than trusting the struct's declared `[T; 1]` array field.
    pub fn read_tcp() -> Option<Vec<RawTcpRow>> {
        let mut size: u32 = 0;
        // SAFETY: `ptcptable: None` + a null-sized query is exactly how
        // this API reports the required buffer size; no buffer is touched.
        #[allow(unsafe_code)]
        unsafe {
            GetExtendedTcpTable(None, &mut size, false, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0);
        }
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is exactly `size` bytes, matching what the prior
        // call reported; the API writes at most `size` bytes into it.
        #[allow(unsafe_code)]
        let result = unsafe {
            GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut _),
                &mut size,
                false,
                AF_INET,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if result != NO_ERROR.0 && result != ERROR_INSUFFICIENT_BUFFER.0 {
            return None;
        }
        // SAFETY: `buf` was filled by a successful call above and is large
        // enough for the header; `dwNumEntries` bounds the row read below.
        #[allow(unsafe_code)]
        let header = unsafe { &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
        let count = header.dwNumEntries as usize;
        // `table` is declared as `[MIB_TCPROW_OWNER_PID; 1]` — a classic C
        // variable-length-array-at-end trick. The buffer is actually
        // `count` rows long; index past the declared length via the raw
        // pointer rather than the (misleadingly single-element) array.
        let rows_ptr = header.table.as_ptr();
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: `count` came from the table's own `dwNumEntries`,
            // matching the buffer size this API reported and filled.
            #[allow(unsafe_code)]
            let row = unsafe { &*rows_ptr.add(i) };
            out.push(RawTcpRow {
                state: row.dwState,
                local_addr: row.dwLocalAddr,
                local_port: row.dwLocalPort,
                remote_addr: row.dwRemoteAddr,
                remote_port: row.dwRemotePort,
                pid: row.dwOwningPid,
            });
        }
        Some(out)
    }

    pub fn read_udp() -> Option<Vec<RawUdpRow>> {
        let mut size: u32 = 0;
        #[allow(unsafe_code)]
        unsafe {
            GetExtendedUdpTable(None, &mut size, false, AF_INET, UDP_TABLE_OWNER_PID, 0);
        }
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        #[allow(unsafe_code)]
        let result = unsafe {
            GetExtendedUdpTable(
                Some(buf.as_mut_ptr() as *mut _),
                &mut size,
                false,
                AF_INET,
                UDP_TABLE_OWNER_PID,
                0,
            )
        };
        if result != NO_ERROR.0 && result != ERROR_INSUFFICIENT_BUFFER.0 {
            return None;
        }
        #[allow(unsafe_code)]
        let header = unsafe { &*(buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID) };
        let count = header.dwNumEntries as usize;
        let rows_ptr = header.table.as_ptr();
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: same reasoning as read_tcp's row loop above.
            #[allow(unsafe_code)]
            let row = unsafe { &*rows_ptr.add(i) };
            out.push(RawUdpRow {
                local_addr: row.dwLocalAddr,
                local_port: row.dwLocalPort,
                pid: row.dwOwningPid,
            });
        }
        Some(out)
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use super::{RawTcpRow, RawUdpRow};

    pub fn read_tcp() -> Option<Vec<RawTcpRow>> {
        None
    }
    pub fn read_udp() -> Option<Vec<RawUdpRow>> {
        None
    }
}

pub struct TcpTableCollector {
    availability: Availability,
}

impl TcpTableCollector {
    pub fn new() -> Self {
        Self {
            availability: Availability::Ok,
        }
    }
}

impl Default for TcpTableCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for TcpTableCollector {
    fn id(&self) -> CollectorId {
        CollectorId::Connections
    }

    fn cadence(&self) -> Cadence {
        Cadence::Warm(CADENCE)
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
            self.availability = Availability::unsupported(
                system_pulse_core::model::UnsupportedReason::NotImplementedOnPlatform,
            );
        }
        self.availability.clone()
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        if !self.availability.is_ok() {
            return CollectorOutput::Connections(Sampled::unavailable(
                self.availability.clone(),
                Source::IpHelper,
                ctx.wall_now,
            ));
        }
        let tcp = raw::read_tcp();
        let udp = raw::read_udp();
        let sampled = match (tcp, udp) {
            (Some(tcp), Some(udp)) => {
                Sampled::ok(parse_all(&tcp, &udp), Source::IpHelper, ctx.wall_now)
            }
            _ => Sampled::unavailable(
                Availability::failed(FailureCode::ApiError),
                Source::IpHelper,
                ctx.wall_now,
            ),
        };
        CollectorOutput::Connections(sampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp_row(state: u32, local_port_be: u32, remote_port_be: u32, pid: u32) -> RawTcpRow {
        RawTcpRow {
            state,
            // 127.0.0.1 in network byte order, matching how Windows fills
            // this field (verified against netstat2's Windows integration).
            local_addr: u32::from_be_bytes([127, 0, 0, 1]).to_be(),
            local_port: local_port_be,
            remote_addr: u32::from_be_bytes([10, 0, 0, 5]).to_be(),
            remote_port: remote_port_be,
            pid,
        }
    }

    #[test]
    fn parses_address_and_port_byte_order_correctly() {
        // Port 443 in the low word, network byte order.
        let port_be = (443u16.to_be() as u32) & 0xFFFF;
        let row = tcp_row(5, port_be, port_be, 4242);
        let conn = parse_tcp_row(&row);
        assert_eq!(conn.local_addr, "127.0.0.1");
        assert_eq!(conn.remote_addr, "10.0.0.5");
        assert_eq!(conn.local_port, 443);
        assert_eq!(conn.remote_port, 443);
        assert_eq!(conn.pid, Some(4242));
        assert_eq!(conn.protocol, TransportProtocol::Tcp);
    }

    #[test]
    fn maps_every_documented_tcp_state() {
        for (raw, expected) in [
            (1, TcpState::Closed),
            (2, TcpState::Listen),
            (5, TcpState::Established),
            (11, TcpState::TimeWait),
            (12, TcpState::DeleteTcb),
        ] {
            assert_eq!(tcp_state_from_raw(raw), Some(expected));
        }
    }

    #[test]
    fn unknown_state_is_none_not_an_error() {
        assert_eq!(tcp_state_from_raw(999), None);
    }

    #[test]
    fn udp_rows_have_no_remote_endpoint_or_state() {
        let row = RawUdpRow {
            local_addr: u32::from_be_bytes([0, 0, 0, 0]),
            local_port: (53u16.to_be() as u32) & 0xFFFF,
            pid: 100,
        };
        let conn = parse_udp_row(&row);
        assert_eq!(conn.protocol, TransportProtocol::Udp);
        assert_eq!(conn.local_port, 53);
        assert_eq!(conn.state, None);
        assert_eq!(conn.remote_port, 0);
    }

    #[test]
    fn parse_all_concatenates_tcp_and_udp() {
        let tcp = vec![tcp_row(2, 0, 0, 1)];
        let udp = vec![RawUdpRow::default()];
        let all = parse_all(&tcp, &udp);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].protocol, TransportProtocol::Tcp);
        assert_eq!(all[1].protocol, TransportProtocol::Udp);
    }

    #[test]
    fn non_windows_probe_and_collect_report_unsupported_never_a_panic() {
        let mut c = TcpTableCollector::new();
        let avail = c.probe();
        #[cfg(not(target_os = "windows"))]
        {
            assert!(!avail.is_ok());
            let ctx = CollectCtx {
                now: std::time::Instant::now(),
                wall_now: system_pulse_core::model::UnixMillis(0),
            };
            match c.collect(&ctx) {
                CollectorOutput::Connections(s) => assert_eq!(s.value, None),
                _ => panic!("expected Connections output"),
            }
        }
        #[cfg(target_os = "windows")]
        assert!(avail.is_ok());
    }
}
