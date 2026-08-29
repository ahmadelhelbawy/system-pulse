//! Pure bucket-boundary math shared by `store`'s rollup SQL — kept out of
//! the SQL itself (rather than computed with SQLite's own integer division)
//! so it's unit-testable without a database and reusable identically by
//! `query`'s range-to-granularity selection.

/// The start timestamp of the bucket containing `ts_ms`, for a bucket
/// width of `bucket_ms`. `div_euclid`/`rem_euclid` (rather than plain `/`)
/// keep this correct for a hypothetical negative timestamp instead of
/// rounding toward zero — defensive, since `ts_ms` is always a real
/// post-1970 wall-clock value in practice. Only `store` (feature
/// `history`) calls this.
#[cfg_attr(not(feature = "history"), allow(dead_code))]
pub fn bucket_start(ts_ms: i64, bucket_ms: i64) -> i64 {
    ts_ms.div_euclid(bucket_ms) * bucket_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_to_the_bucket_boundary() {
        assert_eq!(bucket_start(12_345, 10_000), 10_000);
        assert_eq!(bucket_start(9_999, 10_000), 0);
        assert_eq!(bucket_start(10_000, 10_000), 10_000);
    }

    #[test]
    fn handles_larger_buckets() {
        assert_eq!(bucket_start(125_000, 60_000), 120_000);
    }
}
