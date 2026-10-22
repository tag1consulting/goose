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
    // In f64, which holds every count exactly (an f32 rounds counts past
    // 2^24), from the percent as written: 0.95 rather than the f32's
    // 0.94999999, so 10 times it is 9.5 and rounds up.
    let rank = (total as f64 * super::percent_as_written(percent)).round() as usize;
    match total {
        0 => 0,
        _ if rank >= total => maximum,
        _ if rank <= 1 => minimum,
        _ => super::response_time_percentile(buckets, total, minimum, maximum, percent),
    }
}

/// A list this long finds a bucket by its index rather than by searching.
const PLACED_FROM: usize = 32;

/// How many times fell in each bucket, keyed by the bucket's fastest time in
/// microseconds. Every time under 128 µs has a bucket of its own, and each
/// power of two above is split into 64; a bucket reports the time at its
/// middle, at most 1/128 (0.78%) off any time it holds. Held as a list rather
/// than a tree: a user's batch fills one with a few dozen buckets a second
/// and the metrics processor merges it into the whole run's, and a list does
/// both without allocating a node per few buckets. A batch's list keeps its
/// times in the order they came, which every merge takes; every other list is
/// sorted, fastest first, and once it is long it also keeps where each bucket
/// sits, so a merge finds a bucket without searching. It serializes as a JSON
/// object of bucket to count, as a `BTreeMap` would.
#[derive(Debug, Clone, Default)]
pub struct TimeBuckets {
    list: Vec<(u64, usize)>,
    /// Once the list has [`PLACED_FROM`] buckets: for each bucket index, as
    /// [`bucket_index`] counts them, its place in the list plus one, or 0
    /// when the list does not hold it.
    places: Vec<u32>,
}

impl PartialEq for TimeBuckets {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
    }
}

impl Eq for TimeBuckets {}

impl TimeBuckets {
    /// No buckets.
    pub fn new() -> Self {
        TimeBuckets::default()
    }

    /// Add `count` to the bucket that starts at `bucket` microseconds.
    pub(crate) fn add(&mut self, bucket: u64, count: usize) {
        if !self.places.is_empty() {
            return self.add_placed(bucket, count);
        }
        // Times usually arrive near the slowest bucket so far.
        if let Some(last) = self.list.last_mut() {
            if last.0 == bucket {
                last.1 += count;
                return;
            }
            if last.0 < bucket {
                self.list.push((bucket, count));
                return self.place_if_long();
            }
        }
        match self.list.binary_search_by_key(&bucket, |&(key, _)| key) {
            Ok(at) => self.list[at].1 += count,
            Err(at) => {
                self.list.insert(at, (bucket, count));
                self.place_if_long();
            }
        }
    }

    /// Add `count` to the bucket that starts at `bucket` microseconds, by
    /// its place.
    fn add_placed(&mut self, bucket: u64, count: usize) {
        let index = bucket_index(bucket);
        if index >= self.places.len() {
            self.places.resize(index + 1, 0);
        }
        match self.places[index] {
            0 => {
                let at = self
                    .list
                    .binary_search_by_key(&bucket, |&(key, _)| key)
                    .unwrap_err();
                self.list.insert(at, (bucket, count));
                self.place_from(at);
            }
            place => self.list[place as usize - 1].1 += count,
        }
    }

    /// Start keeping each bucket's place once the list is long.
    fn place_if_long(&mut self) {
        if self.list.len() >= PLACED_FROM {
            self.place_from(0);
        }
    }

    /// Note the place of every bucket from `at` in the list on.
    fn place_from(&mut self, at: usize) {
        if let Some(&(last, _)) = self.list.last() {
            let size = bucket_index(last) + 1;
            if self.places.len() < size {
                self.places.resize(size, 0);
            }
        }
        for (place, &(bucket, _)) in self.list.iter().enumerate().skip(at) {
            self.places[bucket_index(bucket)] = place as u32 + 1;
        }
    }

    /// Add `count` to the bucket that starts at `bucket` microseconds,
    /// leaving the list in the order its times came: a user's batch records
    /// times this way, and a merge takes them in any order.
    pub(crate) fn push(&mut self, bucket: u64, count: usize) {
        if self.list.capacity() == 0 {
            // Room for a batch's times without growing a few times over.
            self.list.reserve(32);
        }
        self.list.push((bucket, count));
    }

    /// Add every bucket of `other`, in whatever order it holds them, handing
    /// each, as its fastest time and count, to `also`.
    pub(crate) fn merge(&mut self, other: &TimeBuckets, mut also: impl FnMut(u64, usize)) {
        for &(bucket, count) in &other.list {
            also(bucket, count);
            self.add(bucket, count);
        }
    }

    /// Set the count of the bucket that starts at `bucket` microseconds.
    pub fn insert(&mut self, bucket: u64, count: usize) {
        match self.list.binary_search_by_key(&bucket, |&(key, _)| key) {
            Ok(at) => self.list[at].1 = count,
            Err(_) => self.add(bucket, count),
        }
    }

    /// The count of the bucket that starts at `bucket` microseconds.
    pub fn get(&self, bucket: &u64) -> Option<&usize> {
        self.list
            .binary_search_by_key(bucket, |&(key, _)| key)
            .ok()
            .map(|at| &self.list[at].1)
    }

    /// How many buckets hold a time.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether no bucket holds a time.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Each bucket's fastest time and its count: fastest first, each once,
    /// except in a user's batch, which keeps the order its times came and
    /// may hold a bucket more than once.
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &usize)> + '_ {
        self.list.iter().map(|(bucket, count)| (bucket, count))
    }
}

impl<'a> IntoIterator for &'a TimeBuckets {
    type Item = (&'a u64, &'a usize);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (u64, usize)>,
        fn(&'a (u64, usize)) -> (&'a u64, &'a usize),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter().map(|(bucket, count)| (bucket, count))
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
    /// Each bucket once, fastest first: a batch, which may hold a bucket
    /// more than once and out of order, is combined and sorted first.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.list.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            serializer.collect_map(self.iter())
        } else {
            let combined: TimeBuckets = self.list.iter().copied().collect();
            serializer.collect_map(combined.iter())
        }
    }
}

impl<'de> serde::Deserialize<'de> for TimeBuckets {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = std::collections::BTreeMap::<u64, usize>::deserialize(deserializer)?;
        Ok(map.into_iter().collect())
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
        // Pushed out of order, each bucket in two pieces, and merged: the
        // same buckets.
        let mut pushed = TimeBuckets::new();
        for &(bucket, count) in merged.list.iter().rev() {
            pushed.push(bucket, count / 2);
            pushed.push(bucket, count - count / 2);
        }
        let mut from_pushed = TimeBuckets::new();
        from_pushed.merge(&pushed, |_, _| {});
        assert_eq!(from_pushed, merged);
        let json = serde_json::to_string(&merged).unwrap();
        assert_eq!(serde_json::from_str::<TimeBuckets>(&json).unwrap(), merged);
        assert_eq!(json, serde_json::to_string(&reference).unwrap());
    }

    /// A user's batch, pushed out of order with a bucket twice, serializes
    /// each bucket once with its whole count.
    #[test]
    fn a_pushed_batch_serializes_each_bucket_once() {
        let mut batch = TimeBuckets::new();
        batch.push(2_048, 1);
        batch.push(128, 2);
        batch.push(2_048, 3);
        let json = serde_json::to_string(&batch).unwrap();
        assert_eq!(json, r#"{"128":2,"2048":4}"#);
        let back: TimeBuckets = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get(&128), Some(&2));
        assert_eq!(back.get(&2_048), Some(&4));
    }

    #[test]
    fn bucket_count_to_the_default_timeout() {
        assert_eq!(bucket_index(60_000_000) - bucket_index(1) + 1, 1_330);
        assert_eq!(bucket_end(u64::MAX), u64::MAX);
    }
}
