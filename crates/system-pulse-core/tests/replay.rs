//! Replays the committed, sanitized NDJSON fixture through the real
//! `TelemetrySnapshot` deserializer and the health analysis layer,
//! headlessly — no hardware, no sampling thread. This is what makes
//! `Deserialize` on every contract type worth having: a capture from a real
//! machine can be checked into the repo and exercised in CI forever,
//! instead of the wire format being write-only.

use std::fs;
use std::path::PathBuf;

use system_pulse_core::health::{analyze, HealthInput};
use system_pulse_core::TelemetrySnapshot;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("replay")
        .join("phase1a-baseline.ndjson")
}

fn load_frames() -> Vec<TelemetrySnapshot> {
    let content = fs::read_to_string(fixture_path()).expect("phase1a-baseline.ndjson must exist");
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<TelemetrySnapshot>(line)
                .unwrap_or_else(|e| panic!("failed to deserialize a captured frame: {e}\n{line}"))
        })
        .collect()
}

#[test]
fn every_captured_frame_deserializes_and_drives_health_analysis() {
    let frames = load_frames();
    assert!(
        !frames.is_empty(),
        "fixture must contain at least one frame"
    );

    for frame in &frames {
        let empty_procs = Vec::new();
        let empty_disks = Vec::new();
        let empty_gpu = Vec::new();
        let cpu_percent = frame
            .cpu
            .value
            .as_ref()
            .map(|c| c.total_percent)
            .unwrap_or(0.0);

        // Must not panic on real, captured data — including frames where a
        // section is genuinely unavailable (this fixture's first frame has
        // disk/network/gpu still `Failed{Timeout}`, since the warm-tier
        // collectors hadn't published yet when it was assembled).
        let alerts = analyze(&HealthInput {
            cpu_percent,
            cpu_history: &[cpu_percent],
            memory_used_percent: frame
                .memory
                .value
                .as_ref()
                .map(|m| m.used_percent)
                .unwrap_or(0.0),
            memory_total: frame.memory.value.as_ref().map(|m| m.total).unwrap_or(0),
            processes: frame.processes.value.as_deref().unwrap_or(&empty_procs),
            disks: frame.disks.value.as_deref().unwrap_or(&empty_disks),
            gpu: frame.gpu.value.as_deref().unwrap_or(&empty_gpu),
        });

        // A sane bound, not a hardcoded exact count: this is real captured
        // data, not a synthetic fixture tuned to produce zero alerts.
        assert!(
            alerts.len() < 50,
            "implausible alert volume: {}",
            alerts.len()
        );
    }
}

#[test]
fn captured_frames_have_plausible_headline_values() {
    // A basic sanity check independent of health analysis: every `Ok`
    // cpu/memory reading must be in a physically plausible range. This is
    // the kind of check that would have caught the 1.0 disk/network `dt`
    // bug immediately, had it been run against a captured trace of it.
    for frame in load_frames() {
        if let Some(cpu) = frame.cpu.value {
            assert!(
                (0.0..=100.0).contains(&cpu.total_percent),
                "cpu.totalPercent out of range: {}",
                cpu.total_percent
            );
        }
        if let Some(mem) = frame.memory.value {
            assert!(
                (0.0..=100.0).contains(&mem.used_percent),
                "memory.usedPercent out of range: {}",
                mem.used_percent
            );
            assert!(mem.used <= mem.total, "memory.used exceeds memory.total");
        }
        if let Some(disks) = frame.disks.value {
            for d in disks {
                assert!(
                    (0.0..=100.0).contains(&d.used_percent),
                    "disk {} usedPercent out of range: {}",
                    d.name,
                    d.used_percent
                );
                assert!(
                    d.read_rate >= 0.0 && d.write_rate >= 0.0,
                    "negative disk rate"
                );
            }
        }
        if let Some(networks) = frame.networks.value {
            for n in networks {
                assert!(
                    n.download_rate >= 0.0 && n.upload_rate >= 0.0,
                    "negative network rate for {}",
                    n.name
                );
            }
        }
    }
}
