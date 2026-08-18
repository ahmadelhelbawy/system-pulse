//! Human-friendly formatting helpers shared by backend and (mirrored in) the
//! frontend. Values use binary prefixes (1024) with conventional labels.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Format a byte count with binary prefixes, e.g. `12.3 GB`.
pub fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a throughput in bytes/second, e.g. `12.3 MB/s`.
pub fn format_rate(bytes_per_sec: f64) -> String {
    if !bytes_per_sec.is_finite() || bytes_per_sec < 0.0 {
        return "0 B/s".to_string();
    }
    format!("{}/s", format_bytes(bytes_per_sec.round() as u64))
}

/// Format a 0..=100 percentage with one decimal, e.g. `42.3%`.
pub fn format_percent(v: f32) -> String {
    format!("{:.1}%", v.clamp(0.0, 100.0))
}

/// Format a duration in seconds as a compact `1d 2h 3m 4s` string.
pub fn format_uptime(total_secs: u64) -> String {
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let mins = (total_secs % 3_600) / 60;
    let secs = total_secs % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 || hours > 0 || days > 0 {
        parts.push(format!("{mins}m"));
    }
    parts.push(format!("{secs}s"));
    parts.join(" ")
}

/// Format a CPU frequency in MHz, e.g. `4.20 GHz` or `850 MHz`.
pub fn format_frequency_mhz(mhz: u64) -> String {
    if mhz >= 1000 {
        format!("{:.2} GHz", mhz as f64 / 1000.0)
    } else {
        format!("{mhz} MHz")
    }
}

/// Format a temperature in Celsius, e.g. `72°C`.
pub fn format_celsius(c: u32) -> String {
    format!("{c}°C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
    }

    #[test]
    fn rate_format() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(1024.0), "1.0 KB/s");
        assert_eq!(format_rate(-1.0), "0 B/s");
    }

    #[test]
    fn uptime_format() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(59), "59s");
        assert_eq!(format_uptime(61), "1m 1s");
        assert_eq!(format_uptime(3_661), "1h 1m 1s");
        assert_eq!(format_uptime(90_061), "1d 1h 1m 1s");
    }

    #[test]
    fn frequency_format() {
        assert_eq!(format_frequency_mhz(850), "850 MHz");
        assert_eq!(format_frequency_mhz(4200), "4.20 GHz");
    }
}
