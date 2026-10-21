//! Response time buckets, in microseconds.
//!
//! One log linear scheme for every time: each power of two is split into 64
//! buckets of equal width, so every time below 128 µs has a bucket of its
//! own (the powers of two below 64 µs are narrower than 64 buckets, and
//! from 64 µs each bucket is 1 µs wide) and a bucket is never wider than
//! 1/64 of its lowest time. A bucket reports the time at its middle, at
//! most 1/128 (0.78%) away from any time it holds. From 1 µs to the default
//! 60 second `--timeout` there are 1,330 buckets; a timing data keeps only
//! those that hold a time.

/// Each power of two is split into `1 << SUB_BITS` buckets.
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

/// How many times fell in each bucket, keyed by the bucket's fastest time in
/// microseconds, fastest first. Held as a sorted list
/// rather than a tree: a user's batch fills one with a few dozen buckets a
/// second and the metrics processor merges it into the whole run's, and a
/// sorted list does both without allocating a node per few buckets. It
/// serializes as a JSON object of bucket to count, as a `BTreeMap` would.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeBuckets(Vec<(u64, usize)>);

impl TimeBuckets {
    /// No buckets.
    pub fn new() -> Self {
        TimeBuckets(Vec::new())
    }

    /// Add `count` to the bucket that starts at `bucket` microseconds.
    pub(crate) fn add(&mut self, bucket: u64, count: usize) {
        // Times usually arrive near the slowest bucket so far.
        if let Some(last) = self.0.last_mut() {
            if last.0 == bucket {
                last.1 += count;
                return;
            }
            if last.0 < bucket {
                self.0.push((bucket, count));
                return;
            }
        }
        match self.0.binary_search_by_key(&bucket, |&(key, _)| key) {
            Ok(at) => self.0[at].1 += count,
            Err(at) => self.0.insert(at, (bucket, count)),
        }
    }

    /// Add `count` to the bucket that starts at `bucket` microseconds,
    /// leaving the list out of order until [`Self::finish`]: a user's batch
    /// records times this way and is finished once, before it is sent.
    pub(crate) fn push(&mut self, bucket: u64, count: usize) {
        if self.0.capacity() == 0 {
            // Room for a batch's buckets without growing a few times over.
            self.0.reserve(32);
        }
        self.0.push((bucket, count));
    }

    /// Sort what [`Self::push`] added and join the pieces of each bucket.
    pub(crate) fn finish(&mut self) {
        if self.0.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return;
        }
        self.0.sort_unstable_by_key(|&(bucket, _)| bucket);
        let mut joined: usize = 0;
        for at in 1..self.0.len() {
            if self.0[at].0 == self.0[joined].0 {
                self.0[joined].1 += self.0[at].1;
            } else {
                joined += 1;
                self.0[joined] = self.0[at];
            }
        }
        self.0.truncate(joined + 1);
    }

    /// Add every bucket of `other`, handing each, as its fastest time and
    /// count, to `also`.
    pub(crate) fn merge(&mut self, other: &TimeBuckets, mut also: impl FnMut(u64, usize)) {
        for &(bucket, count) in &other.0 {
            also(bucket, count);
        }
        if self.0.is_empty() {
            self.0.extend_from_slice(&other.0);
        } else if other.0.len() > self.0.len() {
            // More buckets than held: one pass over both.
            let mut merged = Vec::with_capacity(self.0.len() + other.0.len());
            let (mut a, mut b) = (self.0.iter().peekable(), other.0.iter().peekable());
            while let (Some(&&x), Some(&&y)) = (a.peek(), b.peek()) {
                match x.0.cmp(&y.0) {
                    std::cmp::Ordering::Less => {
                        merged.push(x);
                        a.next();
                    }
                    std::cmp::Ordering::Greater => {
                        merged.push(y);
                        b.next();
                    }
                    std::cmp::Ordering::Equal => {
                        merged.push((x.0, x.1 + y.1));
                        a.next();
                        b.next();
                    }
                }
            }
            merged.extend(a);
            merged.extend(b);
            self.0 = merged;
        } else {
            // Fewer: each is found from where the last one was, and once a
            // run's buckets are all seen, none is inserted.
            let mut from = 0;
            for &(bucket, count) in &other.0 {
                match self.0[from..].binary_search_by_key(&bucket, |&(key, _)| key) {
                    Ok(at) => {
                        self.0[from + at].1 += count;
                        from += at + 1;
                    }
                    Err(at) => {
                        self.0.insert(from + at, (bucket, count));
                        from += at + 1;
                    }
                }
            }
        }
    }

    /// Set the count of the bucket that starts at `bucket` microseconds.
    pub fn insert(&mut self, bucket: u64, count: usize) {
        match self.0.binary_search_by_key(&bucket, |&(key, _)| key) {
            Ok(at) => self.0[at].1 = count,
            Err(at) => self.0.insert(at, (bucket, count)),
        }
    }

    /// The count of the bucket that starts at `bucket` microseconds.
    pub fn get(&self, bucket: &u64) -> Option<&usize> {
        self.0
            .binary_search_by_key(bucket, |&(key, _)| key)
            .ok()
            .map(|at| &self.0[at].1)
    }

    /// How many buckets hold a time.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no bucket holds a time.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each bucket's fastest time and its count, fastest first.
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &usize)> + '_ {
        self.0.iter().map(|(bucket, count)| (bucket, count))
    }

    /// Each bucket's fastest time, fastest first.
    pub fn keys(&self) -> impl Iterator<Item = &u64> + '_ {
        self.0.iter().map(|(bucket, _)| bucket)
    }
}

impl<'a> IntoIterator for &'a TimeBuckets {
    type Item = (&'a u64, &'a usize);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (u64, usize)>,
        fn(&'a (u64, usize)) -> (&'a u64, &'a usize),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(bucket, count)| (bucket, count))
    }
}

impl std::ops::Index<&u64> for TimeBuckets {
    type Output = usize;

    fn index(&self, bucket: &u64) -> &usize {
        self.get(bucket).expect("no such bucket")
    }
}

impl std::iter::FromIterator<(u64, usize)> for TimeBuckets {
    fn from_iter<I: IntoIterator<Item = (u64, usize)>>(buckets: I) -> Self {
        let mut times = TimeBuckets::new();
        for (bucket, count) in buckets {
            times.add(bucket, count);
        }
        times
    }
}

impl serde::Serialize for TimeBuckets {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

impl<'de> serde::Deserialize<'de> for TimeBuckets {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = std::collections::BTreeMap::<u64, usize>::deserialize(deserializer)?;
        Ok(TimeBuckets(map.into_iter().collect()))
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
    fn time_buckets_add_and_merge_as_a_map() {
        let mut rng: u64 = 7;
        let mut next = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bucket_floor((rng >> 33) % 5_000_000)
        };
        let mut reference = std::collections::BTreeMap::new();
        let mut merged = TimeBuckets::new();
        for size in [0, 1, 3, 40, 400, 2_000, 5] {
            let mut batch = TimeBuckets::new();
            for _ in 0..size {
                let bucket = next();
                batch.add(bucket, 1);
                *reference.entry(bucket).or_insert(0) += 1;
            }
            let mut seen = 0;
            merged.merge(&batch, |_, count| seen += count);
            assert_eq!(seen, size);
            let flat: Vec<(u64, usize)> = merged.iter().map(|(&b, &c)| (b, c)).collect();
            let expected: Vec<(u64, usize)> = reference.iter().map(|(&b, &c)| (b, c)).collect();
            assert_eq!(flat, expected);
        }
        // Pushed out of order and finished: the same buckets.
        let mut pushed = TimeBuckets::new();
        for &(bucket, count) in merged.0.iter().rev() {
            pushed.push(bucket, count / 2);
            pushed.push(bucket, count - count / 2);
        }
        pushed.finish();
        assert_eq!(pushed, merged);
        let json = serde_json::to_string(&merged).unwrap();
        assert_eq!(serde_json::from_str::<TimeBuckets>(&json).unwrap(), merged);
        assert_eq!(json, serde_json::to_string(&reference).unwrap());
    }

    #[test]
    fn bucket_count_to_the_default_timeout() {
        assert_eq!(bucket_index(60_000_000) - bucket_index(1) + 1, 1_330);
        assert_eq!(bucket_end(u64::MAX), u64::MAX);
    }
}
