//! The SQLite schema, migrations, writes, rollups, retention and queries.
//! Pure DB logic with an explicit `now_ms` parameter everywhere retention
//! or rollup cutoffs matter, so it's fully unit-testable against an
//! in-memory database with a synthetic timeline — no real clock, no
//! sleeping, no background thread required to exercise it.

use rusqlite::{params, Connection, OptionalExtension};

use super::rollup::bucket_start;
use super::{retention, HistoryPoint, HistorySample, SeriesId, TimeRange};
use crate::model::UnixMillis;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history database error: {0}")]
    Db(#[from] rusqlite::Error),
}

const SCHEMA_VERSION: i64 = 1;

const COLUMNS: &str = "ts_ms, cpu_percent, mem_used_percent, gpu_percent, \
    disk_read_rate, disk_write_rate, net_download_rate, net_upload_rate";

fn create_table_sql(name: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {name} (
            ts_ms INTEGER PRIMARY KEY,
            cpu_percent REAL,
            mem_used_percent REAL,
            gpu_percent REAL,
            disk_read_rate REAL,
            disk_write_rate REAL,
            net_download_rate REAL,
            net_upload_rate REAL
        )"
    )
}

pub struct HistoryStore {
    conn: Connection,
}

impl HistoryStore {
    /// Opens (creating if absent) the history database at `path`, in WAL
    /// mode (so the writer thread's transactions never block a concurrent
    /// reader — the `query_history` IPC command uses its own read-only
    /// connection to the same file), and runs migrations. Creates `path`'s
    /// parent directory if it doesn't exist yet (best-effort — a fresh
    /// install's app-data directory may not exist until something writes
    /// to it, and nothing else in the app creates it first); if that fails
    /// too, `Connection::open` below surfaces a clear error rather than
    /// this silently reporting success into a directory that was never
    /// actually created.
    pub fn open(path: &std::path::Path) -> Result<Self, HistoryError> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// An in-memory database — used by tests, and available for a future
    /// "history disabled" fallback without changing this type's shape.
    pub fn open_in_memory() -> Result<Self, HistoryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, HistoryError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), HistoryError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS rollup_watermarks (
                 granularity TEXT PRIMARY KEY,
                 watermark_ms INTEGER NOT NULL
             );",
        )?;
        let current: i64 = self
            .conn
            .query_row("SELECT version FROM schema_meta LIMIT 1", [], |r| r.get(0))
            .optional()?
            .unwrap_or(0);

        if current < SCHEMA_VERSION {
            self.conn.execute_batch(&create_table_sql("samples_raw"))?;
            self.conn.execute_batch(&create_table_sql("samples_10s"))?;
            self.conn.execute_batch(&create_table_sql("samples_1m"))?;
            self.conn.execute_batch(&create_table_sql("samples_5m"))?;
            if current == 0 {
                self.conn.execute(
                    "INSERT INTO schema_meta (version) VALUES (?1)",
                    [SCHEMA_VERSION],
                )?;
            } else {
                self.conn
                    .execute("UPDATE schema_meta SET version = ?1", [SCHEMA_VERSION])?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn schema_version(&self) -> i64 {
        self.conn
            .query_row("SELECT version FROM schema_meta LIMIT 1", [], |r| r.get(0))
            .unwrap()
    }

    /// Inserts a batch of raw samples in one transaction — the batching the
    /// writer thread relies on to amortize fsync cost instead of committing
    /// once per tick.
    pub fn insert_raw_batch(&mut self, samples: &[HistorySample]) -> Result<(), HistoryError> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(&format!(
                "INSERT OR REPLACE INTO samples_raw ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ))?;
            for s in samples {
                stmt.execute(params![
                    s.ts_ms.0,
                    s.cpu_percent,
                    s.mem_used_percent,
                    s.gpu_percent,
                    s.disk_read_rate,
                    s.disk_write_rate,
                    s.net_download_rate,
                    s.net_upload_rate,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn watermark(&self, granularity: &str) -> Result<i64, HistoryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT watermark_ms FROM rollup_watermarks WHERE granularity = ?1",
                [granularity],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    fn set_watermark(&self, granularity: &str, watermark_ms: i64) -> Result<(), HistoryError> {
        self.conn.execute(
            "INSERT INTO rollup_watermarks (granularity, watermark_ms) VALUES (?1, ?2)
             ON CONFLICT(granularity) DO UPDATE SET watermark_ms = excluded.watermark_ms",
            params![granularity, watermark_ms],
        )?;
        Ok(())
    }

    /// Aggregates every bucket of `source` that has fully closed (its end
    /// has passed `now_ms`) since the last run into `dest`, using the mean
    /// of whatever's present (SQL `AVG` already skips `NULL`s, so a source
    /// row missing one metric doesn't zero out — it's just excluded from
    /// that column's average). Advances the per-granularity watermark so a
    /// restart resumes rather than re-scanning from the beginning.
    fn rollup_into(
        &self,
        source: &str,
        dest: &str,
        granularity: &str,
        bucket_ms: i64,
        now_ms: i64,
    ) -> Result<(), HistoryError> {
        let from = self.watermark(granularity)?;
        let to = bucket_start(now_ms, bucket_ms); // never touch the still-open current bucket
        if to <= from {
            return Ok(());
        }
        self.conn.execute(
            &format!(
                "INSERT OR REPLACE INTO {dest} ({COLUMNS})
                 SELECT (ts_ms / {bucket_ms}) * {bucket_ms} AS bucket,
                        AVG(cpu_percent), AVG(mem_used_percent), AVG(gpu_percent),
                        AVG(disk_read_rate), AVG(disk_write_rate),
                        AVG(net_download_rate), AVG(net_upload_rate)
                 FROM {source}
                 WHERE ts_ms >= ?1 AND ts_ms < ?2
                 GROUP BY bucket"
            ),
            params![from, to],
        )?;
        self.set_watermark(granularity, to)?;
        Ok(())
    }

    /// Runs all three rollup stages (raw→10s→1m→5m, each sourcing only the
    /// table below it — never re-scanning raw data for the coarser
    /// granularities) as of `now_ms`.
    pub fn rollup(&self, now_ms: UnixMillis) -> Result<(), HistoryError> {
        self.rollup_into(
            "samples_raw",
            "samples_10s",
            "10s",
            retention::BUCKET_10S_MS,
            now_ms.0,
        )?;
        self.rollup_into(
            "samples_10s",
            "samples_1m",
            "1m",
            retention::BUCKET_1M_MS,
            now_ms.0,
        )?;
        self.rollup_into(
            "samples_1m",
            "samples_5m",
            "5m",
            retention::BUCKET_5M_MS,
            now_ms.0,
        )?;
        Ok(())
    }

    fn prune_table(&self, table: &str, cutoff_ms: i64) -> Result<(), HistoryError> {
        self.conn.execute(
            &format!("DELETE FROM {table} WHERE ts_ms < ?1"),
            [cutoff_ms],
        )?;
        Ok(())
    }

    /// Time-based retention for every table, plus a hard row-count backstop
    /// on `samples_raw` (belt-and-suspenders against an unusually fast
    /// refresh interval outrunning the time-based cap between passes).
    pub fn apply_retention(&self, now_ms: UnixMillis) -> Result<(), HistoryError> {
        self.prune_table("samples_raw", now_ms.0 - retention::RAW_MS)?;
        self.prune_table("samples_10s", now_ms.0 - retention::RETENTION_10S_MS)?;
        self.prune_table("samples_1m", now_ms.0 - retention::RETENTION_1M_MS)?;
        self.prune_table("samples_5m", now_ms.0 - retention::RETENTION_5M_MS)?;
        self.conn.execute(
            "DELETE FROM samples_raw WHERE ts_ms NOT IN (
                SELECT ts_ms FROM samples_raw ORDER BY ts_ms DESC LIMIT ?1
            )",
            [retention::RAW_ROW_CAP],
        )?;
        Ok(())
    }

    /// Picks the coarsest table that still covers the full requested range
    /// with a reasonable point budget, so a wide range (e.g. 7 days) stays
    /// fast — the whole reason rollups exist — while a narrow range still
    /// gets raw-resolution data.
    fn table_for_range(range: TimeRange) -> &'static str {
        let span = (range.to_ms.0 - range.from_ms.0).max(0);
        if span <= retention::RAW_MS {
            "samples_raw"
        } else if span <= retention::RETENTION_10S_MS {
            "samples_10s"
        } else if span <= retention::RETENTION_1M_MS {
            "samples_1m"
        } else {
            "samples_5m"
        }
    }

    pub fn query(
        &self,
        range: TimeRange,
        series: SeriesId,
    ) -> Result<Vec<HistoryPoint>, HistoryError> {
        let table = Self::table_for_range(range);
        let col = series.column();
        // See `SeriesId::column`'s doc: `table`/`col` are from a fixed,
        // closed Rust enum, never caller input, so this interpolation
        // carries no injection risk despite not being a bind parameter
        // (SQL has no bind-parameter form for identifiers).
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT ts_ms, {col} FROM {table}
             WHERE ts_ms >= ?1 AND ts_ms <= ?2 AND {col} IS NOT NULL
             ORDER BY ts_ms"
        ))?;
        let rows = stmt.query_map(params![range.from_ms.0, range.to_ms.0], |r| {
            Ok(HistoryPoint {
                ts_ms: UnixMillis(r.get(0)?),
                value: r.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HistoryError::from)
    }

    #[cfg(test)]
    pub(crate) fn row_count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts_ms: i64, cpu: f64) -> HistorySample {
        HistorySample {
            ts_ms: UnixMillis(ts_ms),
            cpu_percent: Some(cpu),
            mem_used_percent: Some(50.0),
            gpu_percent: None,
            disk_read_rate: Some(0.0),
            disk_write_rate: Some(0.0),
            net_download_rate: Some(0.0),
            net_upload_rate: Some(0.0),
        }
    }

    #[test]
    fn fresh_database_migrates_to_current_version() {
        let store = HistoryStore::open_in_memory().unwrap();
        assert_eq!(store.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn open_creates_a_missing_parent_directory() {
        // A fresh install's app-data directory may not exist yet — nothing
        // else in the app is guaranteed to have created it first.
        let dir = std::env::temp_dir().join(format!(
            "system-pulse-history-open-test-{}-{}",
            std::process::id(),
            UnixMillis::now().0
        ));
        assert!(!dir.exists());
        let path = dir.join("nested").join("history.sqlite3");

        let store = HistoryStore::open(&path).unwrap();
        assert_eq!(store.schema_version(), SCHEMA_VERSION);
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrating_an_already_current_database_is_a_no_op() {
        let store = HistoryStore::open_in_memory().unwrap();
        store.migrate().unwrap(); // must not error or double-insert schema_meta
        assert_eq!(store.schema_version(), SCHEMA_VERSION);
    }

    #[test]
    fn inserted_raw_samples_round_trip_through_query() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        store
            .insert_raw_batch(&[sample(1_000, 10.0), sample(2_000, 20.0)])
            .unwrap();
        let points = store
            .query(
                TimeRange {
                    from_ms: UnixMillis(0),
                    to_ms: UnixMillis(3_000),
                },
                SeriesId::CpuPercent,
            )
            .unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].value, 10.0);
        assert_eq!(points[1].value, 20.0);
    }

    #[test]
    fn a_series_with_no_reading_this_tick_is_excluded_not_zero() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        store.insert_raw_batch(&[sample(1_000, 10.0)]).unwrap();
        let points = store
            .query(
                TimeRange {
                    from_ms: UnixMillis(0),
                    to_ms: UnixMillis(3_000),
                },
                SeriesId::GpuPercent,
            )
            .unwrap();
        assert!(
            points.is_empty(),
            "NULL gpu_percent must not surface as 0.0"
        );
    }

    #[test]
    fn rollup_averages_closed_buckets_and_ignores_the_open_one() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        // Two samples in the [0, 10_000) bucket, one in the still-open
        // [10_000, 20_000) bucket.
        store
            .insert_raw_batch(&[
                sample(1_000, 10.0),
                sample(5_000, 30.0),
                sample(11_000, 999.0),
            ])
            .unwrap();
        // now_ms = 12_000: the current bucket (10_000..20_000) has not
        // closed yet, so only the first bucket should roll up.
        store.rollup(UnixMillis(12_000)).unwrap();
        let points = store
            .query(
                TimeRange {
                    from_ms: UnixMillis(0),
                    to_ms: UnixMillis(20_000),
                },
                SeriesId::CpuPercent,
            )
            .unwrap();
        // Range span (20_000ms) is within RAW_MS, so this still reads raw —
        // read the rollup table directly to check what actually landed.
        assert_eq!(store.row_count("samples_10s"), 1);
        assert_eq!(points.len(), 3); // raw is untouched by rollup
    }

    #[test]
    fn rollup_is_idempotent_and_resumable_via_watermark() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        store
            .insert_raw_batch(&[sample(1_000, 10.0), sample(5_000, 30.0)])
            .unwrap();
        store.rollup(UnixMillis(12_000)).unwrap();
        store.rollup(UnixMillis(12_000)).unwrap(); // same instant again
        store.rollup(UnixMillis(15_000)).unwrap(); // still the same open bucket
        assert_eq!(store.row_count("samples_10s"), 1);
    }

    #[test]
    fn retention_prunes_raw_rows_older_than_the_window() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        store
            .insert_raw_batch(&[sample(0, 1.0), sample(retention::RAW_MS + 5_000, 2.0)])
            .unwrap();
        store
            .apply_retention(UnixMillis(retention::RAW_MS + 5_000))
            .unwrap();
        assert_eq!(store.row_count("samples_raw"), 1);
    }

    #[test]
    fn retention_prunes_rollup_tables_too_not_just_raw() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        store
            .insert_raw_batch(&[sample(1_000, 10.0), sample(5_000, 30.0)])
            .unwrap();
        // Roll everything up through 10s -> 1m -> 5m by advancing "now"
        // well past every bucket boundary once.
        store
            .rollup(UnixMillis(retention::BUCKET_5M_MS + 1))
            .unwrap();
        assert_eq!(store.row_count("samples_10s"), 1);
        assert_eq!(store.row_count("samples_1m"), 1);
        assert_eq!(store.row_count("samples_5m"), 1);

        // Advance far enough that every rollup table's own retention
        // window (24h / 7d / 30d) has elapsed since those rows landed.
        let far_future = UnixMillis(retention::RETENTION_5M_MS + retention::RETENTION_5M_MS);
        store.apply_retention(far_future).unwrap();
        assert_eq!(store.row_count("samples_10s"), 0);
        assert_eq!(store.row_count("samples_1m"), 0);
        assert_eq!(store.row_count("samples_5m"), 0);
    }

    #[test]
    fn a_seven_day_rollup_query_is_fast() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        // One row every 5 minutes for 7 days = 2016 rows — the acceptance
        // criterion's own scenario ("7-day rollup query < 100ms").
        let bucket = retention::BUCKET_5M_MS;
        let count = retention::RETENTION_1M_MS / bucket; // ~7 days worth of 5m buckets
        let samples: Vec<HistorySample> =
            (0..count).map(|i| sample(i * bucket, i as f64)).collect();
        for chunk in samples.chunks(500) {
            store.insert_raw_batch(chunk).unwrap();
        }
        // Force everything into 5m via successive rollup stages.
        store
            .rollup(UnixMillis(count * bucket + retention::BUCKET_5M_MS))
            .unwrap();

        let start = std::time::Instant::now();
        let points = store
            .query(
                TimeRange {
                    from_ms: UnixMillis(0),
                    to_ms: UnixMillis(count * bucket),
                },
                SeriesId::CpuPercent,
            )
            .unwrap();
        let elapsed = start.elapsed();
        assert!(!points.is_empty());
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "7-day rollup query took {elapsed:?}, exceeding the 100ms acceptance bound"
        );
    }

    #[test]
    fn retention_enforces_the_hard_row_cap_even_within_the_time_window() {
        let mut store = HistoryStore::open_in_memory().unwrap();
        // 400ms spacing keeps the whole run inside RAW_MS (30 min), so the
        // row cap — not the time-based prune — is what's under test here.
        let spacing_ms = 400;
        let samples: Vec<HistorySample> = (0..retention::RAW_ROW_CAP + 100)
            .map(|i| sample(i * spacing_ms, i as f64))
            .collect();
        assert!(
            samples.last().unwrap().ts_ms.0 < retention::RAW_MS,
            "test fixture must stay within the time-based window"
        );
        store.insert_raw_batch(&samples).unwrap();
        let last_ts = samples.last().unwrap().ts_ms.0;
        store.apply_retention(UnixMillis(last_ts)).unwrap();
        assert_eq!(store.row_count("samples_raw"), retention::RAW_ROW_CAP);
    }

    #[test]
    fn query_selects_the_coarsest_table_that_still_covers_the_range() {
        assert_eq!(
            HistoryStore::table_for_range(TimeRange {
                from_ms: UnixMillis(0),
                to_ms: UnixMillis(retention::RAW_MS - 1)
            }),
            "samples_raw"
        );
        assert_eq!(
            HistoryStore::table_for_range(TimeRange {
                from_ms: UnixMillis(0),
                to_ms: UnixMillis(retention::RAW_MS + 1)
            }),
            "samples_10s"
        );
        assert_eq!(
            HistoryStore::table_for_range(TimeRange {
                from_ms: UnixMillis(0),
                to_ms: UnixMillis(retention::RETENTION_10S_MS + 1)
            }),
            "samples_1m"
        );
        assert_eq!(
            HistoryStore::table_for_range(TimeRange {
                from_ms: UnixMillis(0),
                to_ms: UnixMillis(retention::RETENTION_1M_MS + 1)
            }),
            "samples_5m"
        );
    }
}
