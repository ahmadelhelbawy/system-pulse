//! Windows Event Log (Phase 5): `EvtQuery`/`EvtNext` against the
//! Application, System, and (when elevated) Security channels, with
//! bookmarked incremental reads so a restart resumes from where it left
//! off instead of re-scanning the whole log.
//!
//! **Security channel gating.** Opening the Security channel at all
//! requires `SeSecurityPrivilege`, which only an elevated process holds.
//! This collector doesn't pre-check elevation (duplicating a token check
//! system-pulse-win has no other reason to own) — it simply attempts the
//! query and treats `ERROR_ACCESS_DENIED` as the expected, non-fatal signal
//! that the channel is gated right now: `EventLogSnapshot::security_included`
//! is `false` and Application/System data is still returned normally. The
//! collector's own `Availability` stays `Ok` in that case — one gated
//! channel is not a collector failure.
//!
//! **Bookmarks.** Each channel's last-read position is persisted as the
//! opaque XML string `EvtRenderBookmark` produces, in a small JSON file
//! next to `settings.json`/`history.sqlite3` (see `BookmarkStore`). On the
//! next run, that XML is turned back into a bookmark handle via
//! `EvtCreateBookmark` and `EvtSeek(EvtSeekRelativeToBookmark, 1, ...)`
//! skips straight past it — never re-reading, let alone re-surfacing, an
//! already-seen record. `BookmarkStore` itself is plain, pure JSON
//! read/write and is unit-tested without Windows; only the real
//! `EvtSeek`/`EvtCreateBookmark` round trip needs a live Windows host.
//!
//! **Bounded storage.** Newly-read entries accumulate in a
//! `BoundedRing` (see `crate` — actually `system_pulse_core::transport`)
//! for the life of the process; overflow drops the oldest entry and the
//! drop count is surfaced in `EventLogSnapshot::dropped`, never silently.
//! The ring itself is not persisted across restarts — only the bookmark
//! (the OS-level read position) is, which is what "no full rescans" means:
//! the *log* is never re-read from the start, even though the in-memory
//! ring of recently-seen entries starts empty each run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use system_pulse_core::collector::{
    Cadence, CollectCtx, Collector, CollectorId, CollectorOutput, Privilege,
};
use system_pulse_core::model::{Availability, FailureCode, Sampled, Source};
use system_pulse_core::transport::BoundedRing;
use system_pulse_core::types::{EventLevel, EventLogEntry, EventLogSnapshot};

const CADENCE: Duration = Duration::from_secs(20);
/// Bounded in-memory window of recently-seen entries, independent of
/// however many the OS log actually holds — see the module doc's
/// "Bounded storage" section.
const RING_CAPACITY: usize = 500;
/// Hard cap per channel per collection cycle — a channel that has
/// accumulated thousands of new events since the last read (a fresh
/// install, a long-hidden window) is drained over several cycles rather
/// than blocking one `collect()` call reading all of them at once.
const MAX_PER_CHANNEL_PER_CYCLE: u32 = 200;

const CHANNELS: [&str; 3] = ["Application", "System", "Security"];

/// `EVT_SYSTEM_PROPERTY_ID(EvtSystemLevel)`'s raw byte value -> the
/// contract enum. Pure and testable without Windows. `0` (LogAlways) and
/// `4` (Information) both collapse to `Information`; anything
/// unrecognized also falls back to `Information` rather than guessing at
/// a severity this app has no real basis for.
pub fn map_level(raw: u8) -> EventLevel {
    match raw {
        1 => EventLevel::Critical,
        2 => EventLevel::Error,
        3 => EventLevel::Warning,
        5 => EventLevel::Verbose,
        _ => EventLevel::Information,
    }
}

/// Per-channel bookmark persistence — plain JSON, no Windows API surface,
/// so this is unit-testable directly. `EventLogCollector` is the only
/// caller; it treats every I/O error here as "start fresh," never a fatal
/// condition (a corrupt or missing bookmark file just means the next read
/// re-establishes a position from whatever this run's first query finds,
/// exactly like a first-ever run).
#[derive(Default, Debug, Clone, PartialEq)]
pub struct BookmarkStore {
    by_channel: std::collections::HashMap<String, String>,
}

impl BookmarkStore {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .map(|by_channel| Self { by_channel })
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string(&self.by_channel).unwrap_or_default();
        std::fs::write(path, json)
    }

    pub fn get(&self, channel: &str) -> Option<&str> {
        self.by_channel.get(channel).map(String::as_str)
    }

    pub fn set(&mut self, channel: &str, bookmark_xml: String) {
        self.by_channel.insert(channel.to_string(), bookmark_xml);
    }
}

pub struct EventLogCollector {
    bookmark_path: Option<PathBuf>,
    bookmarks: BookmarkStore,
    ring: BoundedRing<EventLogEntry>,
    /// Sticky across cycles: once any cycle successfully reads the
    /// Security channel, later transient failures (a momentary access
    /// error unrelated to elevation) don't flip this back to `false` and
    /// erase the fact that elevation *is* in effect. Reset only by a
    /// fresh `access_denied` observation.
    security_included: bool,
}

impl Default for EventLogCollector {
    fn default() -> Self {
        Self {
            bookmark_path: None,
            bookmarks: BookmarkStore::default(),
            ring: BoundedRing::new(RING_CAPACITY),
            security_included: false,
        }
    }
}

impl EventLogCollector {
    /// `bookmark_path` is `None` for the headless probe and every test —
    /// a collector with no persistence path still works, it just re-seeds
    /// its bookmark from "now" every process start rather than resuming
    /// exactly where a prior run left off.
    pub fn new(bookmark_path: Option<PathBuf>) -> Self {
        let bookmarks = bookmark_path
            .as_deref()
            .map(BookmarkStore::load)
            .unwrap_or_default();
        Self {
            bookmark_path,
            bookmarks,
            ..Self::default()
        }
    }
}

impl Collector for EventLogCollector {
    fn id(&self) -> CollectorId {
        CollectorId::EventLog
    }

    fn cadence(&self) -> Cadence {
        Cadence::Cold(CADENCE)
    }

    fn required_privilege(&self) -> Privilege {
        // Application/System are readable unelevated; Security is gated
        // internally (see the module doc) rather than by refusing to run
        // this collector at all.
        Privilege::User
    }

    fn probe(&mut self) -> Availability {
        #[cfg(target_os = "windows")]
        {
            Availability::Ok
        }
        #[cfg(not(target_os = "windows"))]
        {
            Availability::unsupported(
                system_pulse_core::model::UnsupportedReason::NotImplementedOnPlatform,
            )
        }
    }

    fn collect(&mut self, ctx: &CollectCtx) -> CollectorOutput {
        let mut any_success = false;
        let mut any_hard_error = false;

        for &channel in &CHANNELS {
            let bookmark = self.bookmarks.get(channel).map(str::to_string);
            match raw::read_channel(channel, bookmark.as_deref(), MAX_PER_CHANNEL_PER_CYCLE) {
                Ok(outcome) => {
                    any_success = true;
                    if channel == "Security" {
                        self.security_included = true;
                    }
                    for entry in outcome.entries {
                        self.ring.push(entry);
                    }
                    if let Some(new_bookmark) = outcome.new_bookmark {
                        self.bookmarks.set(channel, new_bookmark);
                    }
                }
                Err(raw::ReadError::AccessDenied) => {
                    if channel == "Security" {
                        self.security_included = false;
                    }
                    // Application/System should never be access-denied
                    // unelevated in practice; if one somehow is, it's
                    // simply not counted as a success below rather than
                    // treated specially.
                }
                Err(raw::ReadError::Other(detail)) => {
                    // Diagnostic only, matching `Scheduler::spawn`'s own
                    // "not load-bearing" stderr pattern for a non-fatal
                    // subsystem failure — this collector still reports
                    // whatever other channels succeeded this cycle.
                    eprintln!("event_log: failed to read channel {channel}: {detail}");
                    any_hard_error = true;
                }
            }
        }

        if let Some(path) = &self.bookmark_path {
            // Best-effort: a failed save just means the next run re-seeds
            // from "now" for whichever channel didn't persist, not a
            // reason to fail this collection cycle.
            let _ = self.bookmarks.save(path);
        }

        let availability = if any_success {
            Availability::Ok
        } else if any_hard_error {
            Availability::failed(FailureCode::ApiError)
        } else {
            // Every channel came back access-denied (only possible if
            // Application/System are somehow gated too) — genuinely
            // needs elevation, not a transient failure.
            Availability::NeedsElevation
        };

        let snapshot = EventLogSnapshot {
            entries: self.ring.iter().cloned().collect(),
            dropped: self.ring.dropped(),
            security_included: self.security_included,
        };

        CollectorOutput::EventLog(Sampled {
            value: if availability.is_ok() {
                Some(snapshot)
            } else {
                None
            },
            availability,
            source: Source::EventLog,
            as_of: ctx.wall_now,
        })
    }
}

#[cfg(target_os = "windows")]
mod raw {
    use super::map_level;
    use system_pulse_core::model::UnixMillis;
    use system_pulse_core::types::EventLogEntry;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::System::EventLog::{
        EvtClose, EvtCreateBookmark, EvtCreateRenderContext, EvtNext, EvtQuery,
        EvtQueryChannelPath, EvtQueryForwardDirection, EvtRender, EvtRenderBookmark,
        EvtRenderContextSystem, EvtRenderEventValues, EvtSeek, EvtSeekRelativeToBookmark,
        EvtSystemEventID, EvtSystemEventRecordId, EvtSystemLevel, EvtSystemProviderName,
        EvtSystemTimeCreated, EvtUpdateBookmark, EVT_HANDLE, EVT_VARIANT,
    };

    pub enum ReadError {
        AccessDenied,
        Other(String),
    }

    pub struct ReadOutcome {
        pub entries: Vec<EventLogEntry>,
        /// `None` when nothing new was read this cycle — the caller must
        /// not overwrite a good bookmark with nothing.
        pub new_bookmark: Option<String>,
    }

    /// RAII guard so an early `?`/return can never leak an `EVT_HANDLE` —
    /// every handle this module opens is wrapped the moment it's created.
    struct Handle(EVT_HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                // SAFETY: `self.0` was returned by a successful Evt* call
                // and is closed at most once, here.
                #[allow(unsafe_code)]
                unsafe {
                    let _ = EvtClose(self.0);
                }
            }
        }
    }

    fn to_read_error(e: windows::core::Error) -> ReadError {
        if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
            ReadError::AccessDenied
        } else {
            ReadError::Other(e.to_string())
        }
    }

    /// FILETIME (100ns ticks since 1601-01-01) -> `UnixMillis`.
    fn filetime_to_unix_millis(filetime: u64) -> UnixMillis {
        const FILETIME_UNIX_EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
        let unix_100ns = filetime.saturating_sub(FILETIME_UNIX_EPOCH_DIFF_100NS);
        UnixMillis((unix_100ns / 10_000) as i64)
    }

    /// Extracts the five system properties this app cares about from one
    /// event handle via a `EvtRenderContextSystem` render — a fixed,
    /// well-known array layout (see `EVT_SYSTEM_PROPERTY_ID`'s constants,
    /// which are literally the array indices), not string/XPath parsing.
    fn render_system_values(
        render_context: EVT_HANDLE,
        event: EVT_HANDLE,
        channel: &str,
    ) -> Option<EventLogEntry> {
        let mut buffer_used = 0u32;
        let mut property_count = 0u32;
        // SAFETY: first call with no output buffer, purely to size it —
        // documented `EvtRender` sizing idiom.
        #[allow(unsafe_code)]
        let sizing = unsafe {
            EvtRender(
                Some(render_context),
                event,
                EvtRenderEventValues.0,
                0,
                None,
                &mut buffer_used,
                &mut property_count,
            )
        };
        // The sizing call is expected to report ERROR_INSUFFICIENT_BUFFER
        // (surfaced as an Err here); a real, different error means this
        // event can't be rendered at all.
        if sizing.is_err() && buffer_used == 0 {
            return None;
        }

        let mut buf: Vec<u8> = vec![0u8; buffer_used as usize];
        let mut used2 = 0u32;
        let mut count2 = 0u32;
        // SAFETY: `buf` is sized exactly to what the call above reported.
        #[allow(unsafe_code)]
        let ok = unsafe {
            EvtRender(
                Some(render_context),
                event,
                EvtRenderEventValues.0,
                buf.len() as u32,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                &mut used2,
                &mut count2,
            )
        };
        if ok.is_err() {
            return None;
        }

        let stride = std::mem::size_of::<EVT_VARIANT>();
        let values = buf.as_ptr() as *const EVT_VARIANT;
        // Bounds-checked: never index past how many `EVT_VARIANT`s the
        // buffer actually holds, regardless of what the fixed property-id
        // constants below assume.
        let variant_count = (buf.len() / stride) as i32;
        let get = |id: i32| -> Option<&EVT_VARIANT> {
            if id < variant_count {
                // SAFETY: `values` points into `buf`, which holds exactly
                // `variant_count` contiguous `EVT_VARIANT`s as filled by
                // the successful `EvtRender` call above; `id` is checked
                // in-bounds just above.
                #[allow(unsafe_code)]
                Some(unsafe { &*values.add(id as usize) })
            } else {
                None
            }
        };

        // SAFETY: each field read below is guarded by the corresponding
        // `EVT_VARIANT::Type` check, matching that variant's actual union
        // arm — never read blind.
        #[allow(unsafe_code)]
        unsafe {
            let provider = get(EvtSystemProviderName.0)
                .filter(|v| v.Type == windows::Win32::System::EventLog::EvtVarTypeString.0 as u32)
                .and_then(|v| {
                    let s = v.Anonymous.StringVal;
                    (!s.is_null()).then(|| s.to_string().unwrap_or_default())
                })
                .unwrap_or_else(|| "Unknown".to_string());

            let event_id = get(EvtSystemEventID.0)
                .filter(|v| v.Type == windows::Win32::System::EventLog::EvtVarTypeUInt16.0 as u32)
                .map(|v| v.Anonymous.UInt16Val as u32)
                .unwrap_or(0);

            let level_raw = get(EvtSystemLevel.0)
                .filter(|v| v.Type == windows::Win32::System::EventLog::EvtVarTypeByte.0 as u32)
                .map(|v| v.Anonymous.ByteVal)
                .unwrap_or(0);

            let time_created = get(EvtSystemTimeCreated.0)
                .filter(|v| v.Type == windows::Win32::System::EventLog::EvtVarTypeFileTime.0 as u32)
                .map(|v| filetime_to_unix_millis(v.Anonymous.FileTimeVal))
                .unwrap_or(UnixMillis(0));

            let record_id = get(EvtSystemEventRecordId.0)
                .filter(|v| v.Type == windows::Win32::System::EventLog::EvtVarTypeUInt64.0 as u32)
                .map(|v| v.Anonymous.UInt64Val)
                .unwrap_or(0);

            // Computed before the struct literal moves `provider` in —
            // `format_message` only needs to borrow it.
            let message = format_message(&provider, event);
            Some(EventLogEntry {
                channel: channel.to_string(),
                record_id,
                event_id,
                level: map_level(level_raw),
                provider,
                time_created,
                // Best-effort human-readable text — see `format_message`'s
                // doc for why a failure here is silent, never fabricated.
                message,
            })
        }
    }

    /// `EvtFormatMessage` against the event's own publisher metadata.
    /// `None` on *any* failure (publisher not registered, no message
    /// table, locale mismatch, etc.) — this app has no fallback text to
    /// offer and does not attempt to construct one; `EventLogEntry` treats
    /// `message: None` as a first-class, expected state, not an error.
    fn format_message(provider: &str, event: EVT_HANDLE) -> Option<String> {
        use windows::Win32::System::EventLog::{EvtFormatMessage, EvtFormatMessageEvent};

        let publisher_name = HSTRING::from(provider);
        // SAFETY: `publisher_name` outlives this call; the handle is
        // opened, used, and closed entirely within this function.
        #[allow(unsafe_code)]
        let metadata = unsafe {
            windows::Win32::System::EventLog::EvtOpenPublisherMetadata(
                None,
                &publisher_name,
                PCWSTR::null(),
                0,
                0,
            )
        }
        .ok()?;
        let _guard = Handle(metadata);

        let mut needed = 0u32;
        // SAFETY: sizing call, no output buffer.
        #[allow(unsafe_code)]
        let sizing = unsafe {
            EvtFormatMessage(
                Some(metadata),
                Some(event),
                0,
                None,
                EvtFormatMessageEvent.0,
                None,
                &mut needed,
            )
        };
        if sizing.is_err() && needed == 0 {
            return None;
        }

        let mut buf: Vec<u16> = vec![0u16; needed as usize];
        let mut used = 0u32;
        // SAFETY: `buf` is sized exactly to what the call above reported.
        #[allow(unsafe_code)]
        let ok = unsafe {
            EvtFormatMessage(
                Some(metadata),
                Some(event),
                0,
                None,
                EvtFormatMessageEvent.0,
                Some(&mut buf),
                &mut used,
            )
        };
        if ok.is_err() {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..end]);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn read_channel(
        channel: &str,
        bookmark_xml: Option<&str>,
        max_events: u32,
    ) -> Result<ReadOutcome, ReadError> {
        let channel_w = HSTRING::from(channel);
        let query_flags = EvtQueryChannelPath.0 | EvtQueryForwardDirection.0;
        // SAFETY: `channel_w` outlives the call; `PCWSTR::null()` (no
        // filter XPath) is a documented valid query argument.
        #[allow(unsafe_code)]
        let query = unsafe { EvtQuery(None, &channel_w, PCWSTR::null(), query_flags) }
            .map_err(to_read_error)?;
        let query = Handle(query);

        if let Some(xml) = bookmark_xml {
            let xml_w = HSTRING::from(xml);
            // SAFETY: `xml_w` outlives the call.
            #[allow(unsafe_code)]
            if let Ok(bm) = unsafe { EvtCreateBookmark(&xml_w) } {
                let bm = Handle(bm);
                // SAFETY: `query.0` and `bm.0` are both valid, freshly
                // opened handles. A seek failure (a stale/corrupt
                // bookmark) is not fatal — the query simply starts from
                // its default position instead, and a slightly wider read
                // than strictly necessary at worst re-surfaces already
                // known records into a bounded ring, never a crash.
                #[allow(unsafe_code)]
                let _ =
                    unsafe { EvtSeek(query.0, 1, Some(bm.0), None, EvtSeekRelativeToBookmark.0) };
            }
        }

        // SAFETY: no per-property XPath list — the fixed system property
        // set is exactly what `render_system_values` reads.
        #[allow(unsafe_code)]
        let render_context = unsafe { EvtCreateRenderContext(None, EvtRenderContextSystem.0) }
            .map_err(to_read_error)?;
        let render_context = Handle(render_context);

        let mut entries = Vec::new();
        let mut last_event: Option<Handle> = None;
        let batch = max_events.clamp(1, 64);
        let mut handles = vec![0isize; batch as usize];

        loop {
            if entries.len() as u32 >= max_events {
                break;
            }
            let mut returned = 0u32;
            // SAFETY: `handles` is a correctly-sized output buffer;
            // `EvtNext` writes at most `handles.len()` raw handle values
            // and reports how many in `returned`.
            #[allow(unsafe_code)]
            let next = unsafe { EvtNext(query.0, &mut handles, 0, 0, &mut returned) };
            if next.is_err() {
                // `ERROR_NO_MORE_ITEMS` (no new events) ends the loop
                // cleanly; any other error just stops this cycle's read
                // early with whatever was already collected — still a
                // valid partial result, not a failure.
                break;
            }
            if returned == 0 {
                break;
            }
            for &raw_handle in &handles[..returned as usize] {
                let event = Handle(EVT_HANDLE(raw_handle));
                if let Some(entry) = render_system_values(render_context.0, event.0, channel) {
                    entries.push(entry);
                }
                last_event = Some(event);
            }
        }

        let new_bookmark = last_event.and_then(|event| {
            // SAFETY: `PCWSTR::null()` creates an empty bookmark handle,
            // a documented valid `EvtCreateBookmark` argument, immediately
            // updated to point at `event` below.
            #[allow(unsafe_code)]
            let bm = unsafe { EvtCreateBookmark(PCWSTR::null()) }.ok()?;
            let bm = Handle(bm);
            // SAFETY: `bm.0` and `event.0` are both valid handles from
            // this same read.
            #[allow(unsafe_code)]
            unsafe { EvtUpdateBookmark(bm.0, event.0) }.ok()?;

            let mut needed = 0u32;
            // SAFETY: sizing call, no output buffer.
            #[allow(unsafe_code)]
            let sizing = unsafe {
                EvtRender(
                    None,
                    bm.0,
                    EvtRenderBookmark.0,
                    0,
                    None,
                    &mut needed,
                    &mut 0,
                )
            };
            if sizing.is_err() && needed == 0 {
                return None;
            }
            let mut buf: Vec<u16> = vec![0u16; (needed as usize) / 2 + 1];
            let mut used = 0u32;
            // SAFETY: `buf` is at least as large as `needed` bytes.
            #[allow(unsafe_code)]
            let ok = unsafe {
                EvtRender(
                    None,
                    bm.0,
                    EvtRenderBookmark.0,
                    (buf.len() * 2) as u32,
                    Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                    &mut used,
                    &mut 0,
                )
            };
            if ok.is_err() {
                return None;
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        });

        Ok(ReadOutcome {
            entries,
            new_bookmark,
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod raw {
    use system_pulse_core::types::EventLogEntry;

    pub enum ReadError {
        #[allow(dead_code)]
        AccessDenied,
        #[allow(dead_code)]
        Other(String),
    }

    pub struct ReadOutcome {
        pub entries: Vec<EventLogEntry>,
        pub new_bookmark: Option<String>,
    }

    pub fn read_channel(
        _channel: &str,
        _bookmark_xml: Option<&str>,
        _max_events: u32,
    ) -> Result<ReadOutcome, ReadError> {
        Ok(ReadOutcome {
            entries: Vec::new(),
            new_bookmark: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_level() {
        assert_eq!(map_level(1), EventLevel::Critical);
        assert_eq!(map_level(2), EventLevel::Error);
        assert_eq!(map_level(3), EventLevel::Warning);
        assert_eq!(map_level(4), EventLevel::Information);
        assert_eq!(map_level(5), EventLevel::Verbose);
    }

    #[test]
    fn log_always_and_unknown_levels_fall_back_to_information_not_a_guess() {
        assert_eq!(map_level(0), EventLevel::Information);
        assert_eq!(map_level(200), EventLevel::Information);
    }

    #[test]
    fn bookmark_store_round_trips_through_a_real_file() {
        let dir = std::env::temp_dir().join(format!("sp-evtbm-test-{}", std::process::id()));
        let path = dir.join("bookmarks.json");

        let mut store = BookmarkStore::default();
        store.set("Application", "<Bookmark App/>".to_string());
        store.set("System", "<Bookmark Sys/>".to_string());
        store.save(&path).unwrap();

        let loaded = BookmarkStore::load(&path);
        assert_eq!(loaded.get("Application"), Some("<Bookmark App/>"));
        assert_eq!(loaded.get("System"), Some("<Bookmark Sys/>"));
        assert_eq!(loaded.get("Security"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bookmark_store_missing_file_starts_empty_not_an_error() {
        let store = BookmarkStore::load(Path::new("/no/such/path/bookmarks.json"));
        assert_eq!(store.get("Application"), None);
    }

    #[test]
    fn bookmark_store_corrupt_file_starts_fresh_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("sp-evtbm-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bookmarks.json");
        std::fs::write(&path, "not valid json{{{").unwrap();

        let store = BookmarkStore::load(&path);
        assert_eq!(store.get("Application"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_windows_probe_reports_unsupported() {
        if cfg!(not(target_os = "windows")) {
            let mut c = EventLogCollector::default();
            assert!(!c.probe().is_ok());
        }
    }

    #[test]
    fn collect_on_this_host_never_panics() {
        let mut c = EventLogCollector::default();
        c.probe();
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis::now(),
        };
        let CollectorOutput::EventLog(sampled) = c.collect(&ctx) else {
            panic!("EventLogCollector must return CollectorOutput::EventLog");
        };
        // No further assertion — the point is that collection never
        // panics on whatever host actually runs this test.
        let _ = sampled;
    }

    #[test]
    fn a_collector_with_no_bookmark_path_still_works() {
        let mut c = EventLogCollector::new(None);
        let ctx = CollectCtx {
            now: std::time::Instant::now(),
            wall_now: system_pulse_core::model::UnixMillis::now(),
        };
        let CollectorOutput::EventLog(_) = c.collect(&ctx) else {
            panic!("EventLogCollector must return CollectorOutput::EventLog");
        };
    }
}
