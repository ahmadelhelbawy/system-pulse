//! Headless probe: runs the real telemetry engine for N seconds and prints
//! observed values. Used on non-Windows hosts to validate the pipeline and to
//! gather real runtime observations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use system_pulse_core::format::{format_bytes, format_percent, format_rate, format_uptime};
use system_pulse_core::model::{Availability, Sampled};
use system_pulse_core::sampling::{
    static_system_info, Backpressure, TelemetryService, TelemetrySink,
};
use system_pulse_core::TelemetrySnapshot;

struct PrintSink {
    json: bool,
    count: AtomicU64,
}

/// Renders a `Sampled<T>` for the text-mode summary line: the value via
/// `f` when available, or a short tag naming why not — the probe's own
/// demonstration that "unavailable" is never rendered as a bare zero.
fn fmt_sampled<T>(s: &Sampled<T>, f: impl FnOnce(&T) -> String) -> String {
    match (&s.value, &s.availability) {
        (Some(v), Availability::Ok) => f(v),
        (Some(v), Availability::Stale { .. }) => format!("{} (stale)", f(v)),
        (_, Availability::Unsupported { .. }) => "unsupported".to_string(),
        (_, Availability::NeedsElevation) => "needs-elevation".to_string(),
        (_, Availability::Failed { .. }) => "failed".to_string(),
        _ => "unavailable".to_string(),
    }
}

impl TelemetrySink for PrintSink {
    fn try_emit(&self, snap: TelemetrySnapshot) -> Result<(), Backpressure> {
        let n = self.count.fetch_add(1, Ordering::Relaxed);
        if self.json {
            if let Ok(j) = serde_json::to_string(&snap) {
                println!("{j}");
            }
            return Ok(());
        }
        if !n.is_multiple_of(2) {
            return Ok(()); // compact mode prints every other frame
        }

        let top = snap
            .processes
            .value
            .as_ref()
            .and_then(|ps| ps.first())
            .map(|p| format!("{} ({}%)", p.name, p.cpu_percent))
            .unwrap_or_else(|| "-".to_string());

        let cpu = fmt_sampled(&snap.cpu, |c| format_percent(c.total_percent));
        let mem = fmt_sampled(&snap.memory, |m| {
            format!(
                "{} / {} ({})",
                format_bytes(m.used),
                format_bytes(m.total),
                format_percent(m.used_percent)
            )
        });
        let disk_io = fmt_sampled(&snap.disk_io, |d| {
            format!(
                "r {} w {}",
                format_rate(d.read_rate),
                format_rate(d.write_rate)
            )
        });
        let net = fmt_sampled(&snap.networks, |ns| {
            format!(
                "d {} u {}",
                format_rate(ns.iter().map(|n| n.download_rate).sum()),
                format_rate(ns.iter().map(|n| n.upload_rate).sum())
            )
        });

        println!(
            "cpu {cpu:>5}  mem {mem}  disk {disk_io}  net {net}  top: {top}  up {}",
            format_uptime(snap.uptime_secs),
        );
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seconds: u64 = args
        .iter()
        .find_map(|a| a.strip_prefix("--seconds=").and_then(|s| s.parse().ok()))
        .unwrap_or(10);
    let json = args.iter().any(|a| a == "--json");
    let idle = args.iter().any(|a| a == "--idle");

    let info = static_system_info();
    eprintln!(
        "system-pulse-probe: {} {} ({}) kernel {} host {} cpu {} x{} ram {}",
        info.os_name,
        info.os_version,
        info.arch,
        info.kernel_version,
        info.hostname,
        info.cpu_model,
        info.cpu_cores,
        format_bytes(info.total_memory),
    );

    let sink: Arc<dyn TelemetrySink + Send + Sync> = Arc::new(PrintSink {
        json,
        count: AtomicU64::new(0),
    });
    let service = TelemetryService::new(sink);
    service.set_visible(!idle);
    service.spawn();

    std::thread::sleep(Duration::from_secs(seconds));
    service.stop();

    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        if let Some(rss) = status
            .lines()
            .find(|l| l.starts_with("VmRSS:"))
            .and_then(|l| l.split_whitespace().nth(1))
        {
            eprintln!("probe peak RSS (VmRSS): {rss} kB");
        }
    }
}
