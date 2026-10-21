//! Response time buckets, in microseconds.
//!
//! Every time below 128 µs has a bucket of its own. From 128 µs each power
//! of two is split into 64 buckets of equal width, so a bucket is never
//! wider than 1/64 of its lowest time. A bucket reports the time at its
//! middle, at most 1/128 (0.78%) away from any time it holds. From 1 µs to
//! the default 60 second `--timeout` there are 1,330 buckets; a timing data
//! keeps only those that hold a time.

/// Each power of two from [`EXACT`] on is split into `1 << SUB_BITS`
/// buckets.
const SUB_BITS: u32 = 6;

/// Buckets per power of two.
const SUB_BUCKETS: u64 = 1 << SUB_BITS;

/// Every time below this has a bucket of its own.
const EXACT: u64 = SUB_BUCKETS << 1;

/// How many bits of `us` a bucket's width spans: 0 below [`EXACT`].
fn width_bits(us: u64) -> u32 {
    if us < EXACT {
        0
    } else {
        63 - us.leading_zeros() - SUB_BITS
    }
}

/// The lowest time in the bucket that holds `us`, which keys the bucket.
pub(crate) fn bucket_floor(us: u64) -> u64 {
    let bits = width_bits(us);
    (us >> bits) << bits
}

/// The first time past the bucket that holds `us`.
pub(crate) fn bucket_end(us: u64) -> u64 {
    bucket_floor(us).saturating_add(1 << width_bits(us))
}

/// The time the bucket that holds `us` reports: its middle.
pub(crate) fn bucket_value(us: u64) -> u64 {
    bucket_floor(us) + ((1 << width_bits(us)) >> 1)
}

/// The bucket that holds `us`, counted from 0 µs. Never decreases as `us`
/// grows.
pub(crate) fn bucket_index(us: u64) -> usize {
    let bits = width_bits(us);
    if bits == 0 {
        us as usize
    } else {
        (EXACT + (bits as u64 - 1) * SUB_BUCKETS + ((us >> bits) - SUB_BUCKETS)) as usize
    }
}

/// The lowest time in bucket `index`, as [`bucket_index`] counts them.
pub(crate) fn bucket_at(index: usize) -> u64 {
    let index = index as u64;
    if index < EXACT {
        index
    } else {
        let bits = (index - EXACT) / SUB_BUCKETS + 1;
        ((index - EXACT) % SUB_BUCKETS + SUB_BUCKETS) << bits
    }
}

/// The time `percent` of the counted times finished within: the value of the
/// bucket that holds the time at that rank, kept between the fastest and
/// slowest time measured, and those exactly at the first and last rank.
/// `buckets` are `(value, count)`, fastest first, and `total` their counts'
/// sum; 0 without times.
pub(crate) fn percentile(
    buckets: impl IntoIterator<Item = (u64, usize)>,
    total: usize,
    minimum: u64,
    maximum: u64,
    percent: f32,
) -> u64 {
    let rank = (total as f32 * percent).round() as usize;
    match total {
        0 => 0,
        _ if rank >= total => maximum,
        _ if rank <= 1 => minimum,
        _ => super::response_time_percentile(buckets, total, minimum, maximum, percent),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn exact_below_128_microseconds() {
        for us in 0..EXACT {
            assert_eq!(bucket_floor(us), us);
            assert_eq!(bucket_value(us), us);
            assert_eq!(bucket_index(us), us as usize);
            assert_eq!(bucket_end(us), us + 1);
        }
        assert_eq!(bucket_floor(128), 128);
        assert_eq!(bucket_end(128), 130);
        assert_eq!(bucket_value(129), 129);
        assert_eq!(bucket_index(128), 128);
        assert_eq!(bucket_index(256), 192);
    }

    #[test]
    fn bucket_at_inverts_bucket_index() {
        for index in 0..2_000 {
            let floor = bucket_at(index);
            assert_eq!(bucket_index(floor), index);
            assert_eq!(bucket_floor(floor), floor);
        }
    }

    #[test]
    fn bucket_count_to_the_default_timeout() {
        assert_eq!(bucket_index(60_000_000) - bucket_index(1) + 1, 1_330);
        assert_eq!(bucket_end(u64::MAX), u64::MAX);
    }
}
