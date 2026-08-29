//! Sanitizes probe NDJSON before it is committed as a replay fixture.
//!
//! Replay fixtures (`fixtures/replay/*.ndjson`) are committed to the repo, so
//! nothing that identifies the machine or user that captured them may survive
//! this pass: home-directory usernames embedded in executable paths, process
//! names outside a small well-known-system allowlist, non-null user names,
//! and network interface names are all replaced with generic, deterministic
//! stand-ins. Numeric/rate fields are left untouched — they're the whole
//! point of a replay fixture.
//!
//! Operates on the generic JSON tree (not `TelemetrySnapshot`) so it doesn't
//! need `Deserialize` on the domain types and keeps working unmodified as the
//! wire contract grows in later phases. New PII-bearing fields (hostnames,
//! IPs, MACs, SMBIOS/SMART serials, event-log text) introduced by later
//! collectors must get a rule added here before they can appear in a fixture.
//!
//! Usage: `system-pulse-probe --json | sanitize > fixtures/replay/name.ndjson`
//! (defaults to stdin/stdout; `--input`/`--output` name files instead).

use std::collections::HashMap;
use std::io::{self, Read, Write};

use serde_json::Value;

/// Process names left untouched: generic OS/runtime names that don't reveal
/// which specific tools or projects the capturing user runs.
const PROCESS_ALLOWLIST: &[&str] = &[
    "system",
    "systemd",
    "init",
    "bash",
    "sh",
    "zsh",
    "dash",
    "sleep",
    "cat",
    "env",
    "node",
    "python",
    "python3",
    "cargo",
    "rustc",
    "rustfmt",
    "clippy-driver",
    "dbus-daemon",
    "gdbus",
    "explorer.exe",
    "svchost.exe",
    "dwm.exe",
    "chrome.exe",
    "firefox.exe",
    "msedge.exe",
    "msedgewebview2.exe",
];

/// Reaches into a Phase 1A `Sampled<Vec<T>>` field — `{"value": [...] |
/// null, "availability": ..., ...}` — and returns the array, if present.
/// Every array-shaped section field (`processes`, `networks`, `disks`,
/// `gpu`) is wrapped this way now; a field that's a bare array (the old,
/// pre-Phase-1A wire shape) is deliberately NOT matched here, so a
/// regression back to the flat shape fails loudly (nothing gets redacted)
/// rather than silently working by accident.
fn sampled_array_mut<'a>(value: &'a mut Value, field: &str) -> Option<&'a mut Vec<Value>> {
    value.get_mut(field)?.get_mut("value")?.as_array_mut()
}

struct Sanitizer {
    process_names: HashMap<String, String>,
    network_names: HashMap<String, String>,
    next_proc: u32,
    next_net: u32,
}

impl Sanitizer {
    fn new() -> Self {
        Self {
            process_names: HashMap::new(),
            network_names: HashMap::new(),
            next_proc: 0,
            next_net: 0,
        }
    }

    fn sanitize_line(&mut self, value: &mut Value) {
        if let Some(processes) = sampled_array_mut(value, "processes") {
            for p in processes {
                self.sanitize_process(p);
            }
        }
        if let Some(networks) = sampled_array_mut(value, "networks") {
            for n in networks {
                self.sanitize_network(n);
            }
        }
        if let Some(disks) = sampled_array_mut(value, "disks") {
            for d in disks {
                if let Some(mp) = d.get("mountPoint").and_then(Value::as_str) {
                    let redacted = redact_home_dir(mp);
                    d["mountPoint"] = Value::String(redacted);
                }
            }
        }
    }

    fn sanitize_process(&mut self, p: &mut Value) {
        if let Some(name) = p.get("name").and_then(Value::as_str) {
            if !PROCESS_ALLOWLIST.contains(&name) {
                let key = name.to_string();
                let next_proc = &mut self.next_proc;
                let replacement = self
                    .process_names
                    .entry(key)
                    .or_insert_with(|| {
                        let id = *next_proc;
                        *next_proc += 1;
                        format!("proc-{id}")
                    })
                    .clone();
                p["name"] = Value::String(replacement);
            }
        }
        if let Some(exe) = p.get("exe").and_then(Value::as_str) {
            let redacted = redact_home_dir(exe);
            p["exe"] = Value::String(redacted);
        }
        if p.get("user").map(|u| !u.is_null()).unwrap_or(false) {
            p["user"] = Value::String("USER".to_string());
        }
    }

    fn sanitize_network(&mut self, n: &mut Value) {
        let Some(name) = n.get("name").and_then(Value::as_str).map(str::to_string) else {
            return;
        };
        let lower = name.to_ascii_lowercase();
        let replacement = if lower == "lo" || lower.contains("loopback") {
            "lo".to_string()
        } else {
            let next_net = &mut self.next_net;
            self.network_names
                .entry(name)
                .or_insert_with(|| {
                    let id = *next_net;
                    *next_net += 1;
                    format!("eth{id}")
                })
                .clone()
        };
        n["name"] = Value::String(replacement);
    }
}

/// Replaces a `/home/<user>` or `C:\Users\<user>` (or `/Users/<user>` on
/// macOS) path segment with a fixed placeholder, leaving the rest of the path
/// (and any non-home path entirely) intact — the directory structure below
/// the home directory is useful replay context, the username is not.
fn redact_home_dir(path: &str) -> String {
    for prefix in ["/home/", "/Users/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            if let Some(slash) = rest.find('/') {
                return format!("{prefix}USER{}", &rest[slash..]);
            }
            return format!("{prefix}USER");
        }
    }
    if let Some(idx) = path.to_ascii_lowercase().find(r"c:\users\") {
        let after = idx + r"c:\users\".len();
        let rest = &path[after..];
        if let Some(slash) = rest.find('\\') {
            return format!("{}USER{}", &path[..after], &rest[slash..]);
        }
        return format!("{}USER", &path[..after]);
    }
    path.to_string()
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let input_path = arg_value(&args, "--input");
    let output_path = arg_value(&args, "--output");

    let mut input = String::new();
    match input_path {
        Some(path) => {
            input = std::fs::read_to_string(path)?;
        }
        None => {
            io::stdin().read_to_string(&mut input)?;
        }
    }

    // Buffered: an unbuffered writer means one write() syscall per line,
    // which is fine on a local filesystem but pathologically slow on a 9p
    // mount (e.g. /mnt/c under WSL2) — seconds per line rather than
    // effectively instant.
    let mut out: Box<dyn Write> = match output_path {
        Some(path) => Box::new(io::BufWriter::new(std::fs::File::create(path)?)),
        None => Box::new(io::BufWriter::new(io::stdout())),
    };

    let mut sanitizer = Sanitizer::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        sanitizer.sanitize_line(&mut value);
        writeln!(out, "{value}")?;
    }
    out.flush()
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_linux_home_dir() {
        assert_eq!(
            redact_home_dir("/home/helbawi/.codex/bin/codex"),
            "/home/USER/.codex/bin/codex"
        );
        assert_eq!(redact_home_dir("/home/helbawi"), "/home/USER");
    }

    #[test]
    fn redacts_windows_home_dir() {
        assert_eq!(
            redact_home_dir(r"C:\Users\ahmed\AppData\Local\foo.exe"),
            r"C:\Users\USER\AppData\Local\foo.exe"
        );
    }

    #[test]
    fn leaves_non_home_paths_untouched() {
        assert_eq!(
            redact_home_dir("/usr/libexec/xdg-document-portal"),
            "/usr/libexec/xdg-document-portal"
        );
    }

    #[test]
    fn allowlisted_process_name_is_untouched_but_others_are_mapped() {
        let mut s = Sanitizer::new();
        let mut node = serde_json::json!({"name": "node", "exe": null, "user": null});
        s.sanitize_process(&mut node);
        assert_eq!(node["name"], "node");

        let mut custom = serde_json::json!({"name": "Bun Pool 0", "exe": null, "user": null});
        s.sanitize_process(&mut custom);
        assert_eq!(custom["name"], "proc-0");

        // Same original name maps to the same replacement, deterministically.
        let mut custom_again = serde_json::json!({"name": "Bun Pool 0", "exe": null, "user": null});
        s.sanitize_process(&mut custom_again);
        assert_eq!(custom_again["name"], "proc-0");
    }

    #[test]
    fn non_null_user_becomes_generic() {
        let mut s = Sanitizer::new();
        let mut p = serde_json::json!({"name": "node", "exe": null, "user": "ahmed"});
        s.sanitize_process(&mut p);
        assert_eq!(p["user"], "USER");
    }

    #[test]
    fn loopback_network_is_normalized_and_others_are_sequential() {
        let mut s = Sanitizer::new();
        let mut lo = serde_json::json!({"name": "lo"});
        s.sanitize_network(&mut lo);
        assert_eq!(lo["name"], "lo");

        let mut eth = serde_json::json!({"name": "enp0s31f6"});
        s.sanitize_network(&mut eth);
        assert_eq!(eth["name"], "eth0");
    }

    #[test]
    fn full_line_round_trips_through_json() {
        // Mirrors the real Phase 1A wire shape: array sections live under
        // `<field>.value`, not directly on the field (see `Sampled<T>`).
        let mut s = Sanitizer::new();
        let mut line = serde_json::json!({
            "timestampMs": 1,
            "processes": {"value": [{"pid": 1, "name": "claude", "cpuPercent": 0.0, "memory": 1, "gpuMem": null, "exe": "/home/helbawi/x", "user": null}], "availability": {"state": "ok"}},
            "networks": {"value": [{"name": "eth0", "downloadRate": 0.0, "uploadRate": 0.0, "totalRx": 0, "totalTx": 0}], "availability": {"state": "ok"}},
            "disks": {"value": [{"mountPoint": "/home/helbawi/project"}], "availability": {"state": "ok"}}
        });
        s.sanitize_line(&mut line);
        assert_eq!(line["processes"]["value"][0]["exe"], "/home/USER/x");
        assert_eq!(
            line["disks"]["value"][0]["mountPoint"],
            "/home/USER/project"
        );
        // timestampMs and numeric fields are untouched.
        assert_eq!(line["timestampMs"], 1);
    }

    #[test]
    fn sections_with_no_value_are_left_alone_not_panicked_on() {
        // An unavailable section (`value: null`, e.g. Failed/Unsupported)
        // must not crash the sanitizer.
        let mut s = Sanitizer::new();
        let mut line = serde_json::json!({
            "timestampMs": 1,
            "processes": {"value": null, "availability": {"state": "failed", "code": "timeout", "detail": null}},
        });
        s.sanitize_line(&mut line);
        assert_eq!(line["processes"]["value"], serde_json::Value::Null);
    }
}
