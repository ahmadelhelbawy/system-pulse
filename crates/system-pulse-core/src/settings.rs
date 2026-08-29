//! User settings and hotkey parsing/validation.
//!
//! The hotkey grammar is intentionally small and deterministic so that both
//! the Rust backend (authoritative) and the TypeScript frontend (display +
//! recorder) can agree on a single textual representation.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// Default global hotkey: Ctrl+Alt+0.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+0";

pub const MIN_REFRESH_INTERVAL_MS: u64 = 250;
pub const MAX_REFRESH_INTERVAL_MS: u64 = 10_000;
pub const DEFAULT_REFRESH_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    /// Canonical hotkey string, e.g. `Ctrl+Alt+0`.
    pub hotkey: String,
    pub launch_at_startup: bool,
    pub compact_mode: bool,
    /// Cheap sampling interval (ms) for CPU/memory and the snapshot cadence.
    pub refresh_interval_ms: u64,
    /// Hide to the tray instead of quitting when the window is closed.
    pub hide_to_tray_on_close: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            launch_at_startup: false,
            compact_mode: false,
            refresh_interval_ms: DEFAULT_REFRESH_INTERVAL_MS,
            hide_to_tray_on_close: true,
        }
    }
}

impl Settings {
    /// Validate and normalize the settings object in place.
    pub fn sanitize(&mut self) {
        self.refresh_interval_ms = self
            .refresh_interval_ms
            .clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);
        match Hotkey::parse(&self.hotkey) {
            Ok(hk) => self.hotkey = hk.to_display(),
            Err(_) => self.hotkey = DEFAULT_HOTKEY.to_string(),
        }
    }

    pub fn parsed_hotkey(&self) -> Result<Hotkey, HotkeyError> {
        Hotkey::parse(&self.hotkey)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// Normalized key: a single A-Z / 0-9 char, `F1`..`F24`, or a named key.
    pub key: String,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("hotkey is empty")]
    Empty,
    #[error("hotkey must include at least one modifier (Ctrl, Alt, Shift, or Win)")]
    MissingModifier,
    #[error("unsupported key: {0}")]
    UnsupportedKey(String),
    #[error("hotkey has too many parts")]
    TooManyParts,
}

const NAMED_KEYS: &[(&str, &str)] = &[
    ("SPACE", "Space"),
    ("BACKSPACE", "Backspace"),
    ("TAB", "Tab"),
    ("ENTER", "Enter"),
    ("RETURN", "Enter"),
    ("ESCAPE", "Escape"),
    ("ESC", "Escape"),
    ("DELETE", "Delete"),
    ("DEL", "Delete"),
    ("INSERT", "Insert"),
    ("INS", "Insert"),
    ("HOME", "Home"),
    ("END", "End"),
    ("PAGEUP", "PageUp"),
    ("PGUP", "PageUp"),
    ("PAGEDOWN", "PageDown"),
    ("PGDN", "PageDown"),
    ("UP", "Up"),
    ("DOWN", "Down"),
    ("LEFT", "Left"),
    ("RIGHT", "Right"),
    ("PRINTSCREEN", "PrintScreen"),
    ("PRTSC", "PrintScreen"),
];

impl Default for Hotkey {
    fn default() -> Self {
        Self::parse(DEFAULT_HOTKEY).expect("default hotkey must parse")
    }
}

impl Hotkey {
    pub fn parse(input: &str) -> Result<Self, HotkeyError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(HotkeyError::Empty);
        }

        let mut hotkey = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key: String::new(),
        };

        for raw_part in trimmed.split('+') {
            let part = raw_part.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hotkey.ctrl = true,
                "alt" | "option" => hotkey.alt = true,
                "shift" => hotkey.shift = true,
                "win" | "windows" | "super" | "meta" | "cmd" | "command" => hotkey.meta = true,
                _ => {
                    if !hotkey.key.is_empty() {
                        return Err(HotkeyError::TooManyParts);
                    }
                    hotkey.key = normalize_key(part)?;
                }
            }
        }

        if hotkey.key.is_empty() {
            return Err(HotkeyError::UnsupportedKey("no key specified".to_string()));
        }
        if !hotkey.ctrl && !hotkey.alt && !hotkey.shift && !hotkey.meta {
            return Err(HotkeyError::MissingModifier);
        }
        Ok(hotkey)
    }

    /// Canonical human display form, e.g. `Ctrl+Alt+0`.
    pub fn to_display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.meta {
            parts.push("Win");
        }
        parts.push(self.key.as_str());
        parts.join("+")
    }

    /// Lowercase form accepted by `tauri-plugin-global-shortcut`.
    pub fn to_shortcut(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.meta {
            parts.push("super");
        }
        let key = self.plugin_key();
        parts.push(key.as_str());
        parts.join("+")
    }

    fn plugin_key(&self) -> String {
        if let Some(rest) = self.key.strip_prefix('F') {
            if rest.chars().all(|c| c.is_ascii_digit()) {
                return format!("f{rest}");
            }
        }
        match self.key.as_str() {
            "Space" => "space".into(),
            "Backspace" => "backspace".into(),
            "Tab" => "tab".into(),
            "Enter" => "enter".into(),
            "Escape" => "escape".into(),
            "Delete" => "delete".into(),
            "Insert" => "insert".into(),
            "Home" => "home".into(),
            "End" => "end".into(),
            "PageUp" => "pageup".into(),
            "PageDown" => "pagedown".into(),
            "Up" => "up".into(),
            "Down" => "down".into(),
            "Left" => "left".into(),
            "Right" => "right".into(),
            other => other.to_ascii_lowercase(),
        }
    }
}

fn normalize_key(part: &str) -> Result<String, HotkeyError> {
    let upper = part.to_ascii_uppercase();

    // Single character key: letter or digit.
    if upper.len() == 1 {
        let c = upper.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Ok(upper);
        }
    }

    // Function keys.
    if let Some(rest) = upper.strip_prefix('F') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            let n: u32 = rest.parse().unwrap_or(0);
            if (1..=24).contains(&n) {
                return Ok(format!("F{n}"));
            }
        }
    }

    // Named keys.
    for (alias, canonical) in NAMED_KEYS {
        if upper == *alias {
            return Ok((*canonical).to_string());
        }
    }

    Err(HotkeyError::UnsupportedKey(part.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses() {
        let hk = Hotkey::parse(DEFAULT_HOTKEY).unwrap();
        assert_eq!(hk.to_display(), "Ctrl+Alt+0");
        assert_eq!(hk.to_shortcut(), "ctrl+alt+0");
    }

    #[test]
    fn modifiers_and_key_are_normalized() {
        let hk = Hotkey::parse("  control + alt + shift + A ").unwrap();
        assert!(hk.ctrl && hk.alt && hk.shift && !hk.meta);
        assert_eq!(hk.key, "A");
        assert_eq!(hk.to_display(), "Ctrl+Alt+Shift+A");
    }

    #[test]
    fn meta_maps_to_win_and_super() {
        let hk = Hotkey::parse("win+f12").unwrap();
        assert!(hk.meta);
        assert_eq!(hk.key, "F12");
        assert_eq!(hk.to_display(), "Win+F12");
        assert_eq!(hk.to_shortcut(), "super+f12");
    }

    #[test]
    fn named_keys_are_canonicalized() {
        assert_eq!(Hotkey::parse("ctrl+esc").unwrap().key, "Escape");
        assert_eq!(Hotkey::parse("ctrl+del").unwrap().key, "Delete");
        assert_eq!(Hotkey::parse("ctrl+pgup").unwrap().key, "PageUp");
        assert_eq!(Hotkey::parse("ctrl+space").unwrap().key, "Space");
    }

    #[test]
    fn rejects_missing_modifier() {
        assert_eq!(Hotkey::parse("a"), Err(HotkeyError::MissingModifier));
        assert_eq!(Hotkey::parse(""), Err(HotkeyError::Empty));
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(matches!(
            Hotkey::parse("ctrl+pancake"),
            Err(HotkeyError::UnsupportedKey(_))
        ));
    }

    #[test]
    fn settings_sanitize_fixes_bad_values() {
        let mut s = Settings {
            hotkey: "garbage".into(),
            refresh_interval_ms: 1,
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert_eq!(s.refresh_interval_ms, MIN_REFRESH_INTERVAL_MS);
    }
}
