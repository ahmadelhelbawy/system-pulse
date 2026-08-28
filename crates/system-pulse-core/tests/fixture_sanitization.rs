//! Guards `fixtures/replay/*.ndjson` against accidentally committing
//! machine- or user-identifying data. Runs as part of `cargo test -p
//! system-pulse-core`, standing in for the "CI PII scan" called for in the
//! System Pulse 2.0 plan's fixture policy until real CI exists in this repo.
//!
//! This is a pattern denylist, not a guarantee of full anonymity — it exists
//! to catch the specific leaks the sanitizer (`src/bin/sanitize.rs`) is
//! responsible for removing, so a regression there fails the build instead
//! of landing quietly in a committed fixture.
//!
//! Each line is parsed as JSON and every *decoded* string value is checked
//! (not the raw file bytes), so this isn't fooled by JSON's own backslash
//! escaping of Windows paths.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

/// The actual home-directory username seen in this repo's Phase 0 capture,
/// checked by exact name in addition to the general patterns below so a
/// leak of *this specific* machine's identity is always caught even if the
/// general patterns are narrowed later.
const KNOWN_LEAKED_USERNAME: &str = "helbawi";

fn replay_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("replay")
}

#[test]
fn replay_fixtures_contain_no_known_pii_patterns() {
    let dir = replay_fixtures_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        // No fixtures committed yet is fine; nothing to scan.
        return;
    };

    let mut checked = 0usize;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ndjson") {
            continue;
        }
        checked += 1;
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));

        for (line_no, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("{}:{}: not valid JSON: {e}", path.display(), line_no + 1)
            });
            let mut strings = Vec::new();
            collect_strings(&value, &mut strings);

            for s in &strings {
                assert!(
                    !s.contains(KNOWN_LEAKED_USERNAME),
                    "{}:{}: leaked known username {KNOWN_LEAKED_USERNAME:?} in {s:?}",
                    path.display(),
                    line_no + 1
                );
                assert!(
                    !has_home_username(s, "/home/", '/'),
                    "{}:{}: leaked a /home/<name>/ path with a real username: {s:?} (expected /home/USER/)",
                    path.display(),
                    line_no + 1
                );
                assert!(
                    !has_home_username_case_insensitive(s, r"C:\Users\", '\\'),
                    "{}:{}: leaked a C:\\Users\\<name>\\ path with a real username: {s:?} (expected \\USER\\)",
                    path.display(),
                    line_no + 1
                );
            }
        }
    }

    // Sanity check that this test is actually exercising something once
    // Phase 0's baseline fixture is committed.
    assert!(
        checked > 0 || !dir.exists(),
        "fixtures/replay/ exists but contains no .ndjson files"
    );
}

/// Recursively collects every string leaf value in a JSON tree (object keys
/// are not values people fabricate paths into, so only values are checked).
fn collect_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(s) => out.push(s),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

/// True if `s` contains `prefix` followed by a path segment that isn't the
/// sanitizer's `USER` placeholder — i.e. a real username slipped through.
fn has_home_username(s: &str, prefix: &str, separator: char) -> bool {
    let mut rest = s;
    while let Some(idx) = rest.find(prefix) {
        let after = &rest[idx + prefix.len()..];
        let segment_end = after.find(separator).unwrap_or(after.len());
        let segment = &after[..segment_end];
        if segment != "USER" && !segment.is_empty() {
            return true;
        }
        rest = &after[segment_end..];
    }
    false
}

fn has_home_username_case_insensitive(s: &str, prefix: &str, separator: char) -> bool {
    let lower = s.to_ascii_lowercase();
    let Some(idx) = lower.find(&prefix.to_ascii_lowercase()) else {
        return false;
    };
    has_home_username(&s[idx..], prefix, separator)
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn detects_real_linux_username_but_not_placeholder() {
        assert!(has_home_username("/home/helbawi/.codex/bin", "/home/", '/'));
        assert!(!has_home_username("/home/USER/.codex/bin", "/home/", '/'));
        assert!(!has_home_username(
            "/usr/libexec/xdg-document-portal",
            "/home/",
            '/'
        ));
    }

    #[test]
    fn detects_real_windows_username_but_not_placeholder() {
        assert!(has_home_username_case_insensitive(
            r"C:\Users\ahmed\AppData\Local\foo.exe",
            r"C:\Users\",
            '\\'
        ));
        assert!(!has_home_username_case_insensitive(
            r"C:\Users\USER\AppData\Local\foo.exe",
            r"C:\Users\",
            '\\'
        ));
    }

    #[test]
    fn collects_nested_strings() {
        let v = serde_json::json!({
            "a": "leak-me",
            "b": [{"c": "also-leak-me"}, 1, null],
        });
        let mut out = Vec::new();
        collect_strings(&v, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&"leak-me"));
        assert!(out.contains(&"also-leak-me"));
    }
}
