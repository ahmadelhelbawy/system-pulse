//! Headless probe: runs the real telemetry engine for N seconds and prints
//! observed values. Used on non-Windows hosts to validate the pipeline and to
//! gather real runtime observations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use system_pulse_core::format::{format_bytes, format_percent, format_rate, format_uptime};
use system_pulse_core::gpu::default_gpu_provider;
use system_pulse_core::sampling::{static_system_info, TelemetryService, TelemetrySink};
use system_pulse_core::TelemetrySnapshot;

struct PrintSink {
    json: bool,
    count: AtomicU64,
}

impl TelemetrySink for PrintSink {
    fn emit(&self, snap: TelemetrySnapshot) {
        let n = self.count.fetch_add(1, Ordering::Relaxed);
        if self.json {
            if let Ok(j) = serde_json::to_string(&snap) {
                println!("{j}");
            }
            return;
        }
        if !n.is_multiple_of(2) {
            return; // compact mode prints every other frame
        }
        let top = snap
            .processes
            .first()
            .map(|p| format!("{} ({}%)", p.name, p.cpu_percent))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "cpu {:>5}  mem {} / {} ({})  disk r {} w {}  net d {} u {}  top: {}  up {}",
            format_percent(snap.cpu.total_percent),
            format_bytes(snap.memory.used),
            format_bytes(snap.memory.total),
            format_percent(snap.memory.used_percent),
            format_rate(snap.disk_io.read_rate),
            format_rate(snap.disk_io.write_rate),
            format_rate(snap.networks.iter().map(|n| n.download_rate).sum()),
            format_rate(snap.networks.iter().map(|n| n.upload_rate).sum()),
            top,
            format_uptime(snap.uptime_secs),
        );
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
    let service = TelemetryService::new(sink, default_gpu_provider());
    service.set_visible(!idle);
    let _handle = service.spawn();

    std::thread::sleep(Duration::from_secs(seconds));

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
