//! Utility functions used by Goose, and available when writing load tests.

use regex::Regex;
use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::time;
use url::Url;

use crate::{GooseError, CANCELED};

/// Parse a string representing a time span and return the number of seconds.
///
/// Can be specified as an integer, indicating seconds. Or can use integers
/// together with one or more of "h", "m", and "s", in that order, indicating
/// "hours", "minutes", and "seconds".
///
/// Valid formats include: 20, 20s, 3m, 2h, 1h20m, 3h30m10s, etc.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // 1 hour 2 minutes and 3 seconds is 3,723 seconds.
/// assert_eq!(util::parse_timespan("1h2m3s"), 3_723);
///
/// // 45 seconds is 45 seconds.
/// assert_eq!(util::parse_timespan("45"), 45);
///
/// // Invalid value is 0 seconds.
/// assert_eq!(util::parse_timespan("foo"), 0);
/// ```
pub fn parse_timespan(time_str: &str) -> usize {
    match usize::from_str(time_str) {
        // If an integer is passed in, assume it's seconds
        Ok(t) => {
            trace!("{} is integer: {} seconds", time_str, t);
            t
        }
        // Otherwise use a regex to extract hours, minutes and seconds from string.
        Err(_) => {
            let re = Regex::new(r"((?P<hours>\d+?)h)?((?P<minutes>\d+?)m)?((?P<seconds>\d+?)s)?")
                .unwrap();
            let time_matches = re.captures(time_str).unwrap();
            let hours = match time_matches.name("hours") {
                Some(_) => usize::from_str(&time_matches["hours"]).unwrap(),
                None => 0,
            };
            let minutes = match time_matches.name("minutes") {
                Some(_) => usize::from_str(&time_matches["minutes"]).unwrap(),
                None => 0,
            };
            let seconds = match time_matches.name("seconds") {
                Some(_) => usize::from_str(&time_matches["seconds"]).unwrap(),
                None => 0,
            };
            let total = hours * 60 * 60 + minutes * 60 + seconds;
            trace!(
                "{} hours {} minutes {} seconds: {} seconds",
                hours,
                minutes,
                seconds,
                total
            );
            total
        }
    }
}

/// Sleep for a specified duration, minus the time spent doing other things.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// async fn loop_with_delay() {
///     loop {
///         // Start drift timer.
///         let mut drift_timer = tokio::time::Instant::now();
///
///         // Do other stuff, in this case sleep 250 milliseconds. This is
///         // the "drift" that will be subtracted from the sleep time later.
///         tokio::time::sleep(std::time::Duration::from_millis(250));
///
///         // Sleep for 1 second minus the time spent doing other stuff.
///         drift_timer = util::sleep_minus_drift(
///             std::time::Duration::from_secs(1),
///             drift_timer,
///         ).await;
///
///         // Normally the loop would continue, and the amount of time doing
///         // other things would vary each time, but the total time to complete
///         // the loop would remain the same.
///         break;
///     }
/// }
/// ```
pub async fn sleep_minus_drift(
    duration: std::time::Duration,
    drift: tokio::time::Instant,
) -> tokio::time::Instant {
    match duration.checked_sub(drift.elapsed()) {
        Some(delay) if delay.as_nanos() > 0 => tokio::time::sleep(delay).await,
        _ => info!("sleep_minus_drift: drift was greater than or equal to duration, not sleeping"),
    };
    tokio::time::Instant::now()
}

/// Calculate the greatest common divisor of two integers using binary GCD (or Stein's) algorithm.
///
/// More detail on [Wikipedia](https://en.wikipedia.org/wiki/Binary_GCD_algorithm).
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // 1 and any other integer are only divisible by 1.
/// assert_eq!(util::gcd(1, 100), 1);
///
/// // 9 and 103 are both divisible by 3.
/// assert_eq!(util::gcd(9, 102), 3);
///
/// // 12345 and 67890 are both divisible by 15.
/// assert_eq!(util::gcd(12345, 67890), 15);
///
/// // 2 and 5 are both divisible by 1.
/// assert_eq!(util::gcd(2, 5), 1);
/// ```
pub fn gcd(u: usize, v: usize) -> usize {
    match ((u, v), (u & 1, v & 1)) {
        ((x, y), _) if x == y => x,
        ((x, y), (0, 1)) | ((y, x), (1, 0)) => gcd(x >> 1, y),
        ((x, y), (0, 0)) => gcd(x >> 1, y >> 1) << 1,
        ((x, y), (1, 1)) => {
            let (x, y) = (min(x, y), max(x, y));
            gcd((y - x) >> 1, x)
        }
        _ => unreachable!(),
    }
}

/// Calculate the standard deviation between two f32 numbers.
///
/// Standard deviation is the average variability between numbers. It indicates, on average,
/// how far from the mean each value is. A high standard deviation suggests that values are
/// generally far from the mean, while a low standard deviation suggests that values are
/// close to the mean.
///
/// Standard deviation is calculated with the following steps:
///  1) determine the average of the two numbers
///  2) subtract the mean from each number, calculating two values (one positive, one negative)
///  3) square each value and add them together (this is the "variance")
///  4) return the square root of this value (this is the "standard deviation")
pub fn standard_deviation(raw_average: f32, co_average: f32) -> f32 {
    // Determine the mean (average) between the two numbers.
    let mean = (raw_average + co_average) / 2.0;
    // Get the difference between the mean and each number.
    let raw_difference = raw_average - mean;
    let co_difference = co_average - mean;
    // Add together the square of both differences to get the variance.
    let variance = raw_difference * raw_difference + co_difference * co_difference;
    // Finally, calculate the standard deviation, which is the square root of the variance.
    variance.sqrt()
}

/// Calculate median for a BTreeMap of usizes.
///
/// The Median is the "middle" of a sorted list of numbers. In this case, the list is
/// comprised of two parts: the integer value on the left, and the number of occurrences
/// of the integer on the right. For example (5, 1) indicates that the integer "5" is
/// included 1 time.
///
/// The function requires three parameters that Goose already has while building the
/// BTreeMap: the total occurences of all integers, the smallest integer, and the largest
/// integer in the list: while this could be calculate by the function, the goal is to make
/// this function as fast as possible as it runs during load tests.
///
/// NOTE: Once [`first_entry`](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html#method.first_entry)
/// and [`last_entry`](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html#method.last_entry)
/// land in Stable Rust ([rust-lang issue #62924](https://github.com/rust-lang/rust/issues/62924))
/// we can efficiently derive the last two parameters and simplify the calling of this
/// function a little.
///
/// # Example
/// ```rust
/// use std::collections::BTreeMap;
/// use goose::util;
///
/// // In this first example, we add one instance of three different integers.
/// let mut btree: BTreeMap<usize, usize> = BTreeMap::new();
/// btree.insert(1, 1);
/// btree.insert(99, 1);
/// btree.insert(100, 1);
///
/// // Median (middle) value in this list of 3 integers is 99.
/// assert_eq!(util::median(&btree, 3, 1, 100), 99);
///
/// // In this next example, we add multiple instance of five different integers.
/// let mut btree: BTreeMap<usize, usize> = BTreeMap::new();
/// btree.insert(7, 5);
/// btree.insert(8, 1);
/// btree.insert(13, 21);
/// btree.insert(19, 44);
/// btree.insert(21, 5);
///
/// // Median (middle) value in this list of 76 integers is 19.
/// assert_eq!(util::median(&btree, 76, 7, 21), 19);
/// ```
pub fn median(
    btree: &BTreeMap<usize, usize>,
    total_elements: usize,
    min: usize,
    max: usize,
) -> usize {
    let mut total_count: usize = 0;
    let half_elements: usize = (total_elements as f64 / 2.0).round() as usize;
    for (value, counter) in btree {
        total_count += counter;
        if total_count >= half_elements {
            // We're working with rounded values, it's possible the mean is greater than the
            // max response time, or smaller than the min response time -- in these cases
            // return the actual values;
            if *value > max {
                return max;
            } else if *value < min {
                return min;
            } else {
                return *value;
            }
        }
    }
    0
}

/// Truncate strings when they're too long to display.
///
/// If a string is longer than the specified max length, this function removes extra
/// the characters and replaces the last two with a double-period ellipsis.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // All but 7 characters are truncated, with ".." appended.
/// assert_eq!(util::truncate_string("this is a long string", 9), "this is..");
///
/// // All characters are returned as the string is less than 15 characters long.
/// assert_eq!(util::truncate_string("shorter string", 15), "shorter string");
/// ```
pub fn truncate_string(str_to_truncate: &str, max_length: usize) -> String {
    if str_to_truncate.char_indices().count() > max_length {
        match str_to_truncate.char_indices().nth(max_length - 2) {
            None => str_to_truncate.to_string(),
            Some((idx, _)) => format!("{}..", &str_to_truncate[..idx]),
        }
    } else {
        str_to_truncate.to_string()
    }
}

/// Determine if a timer expired, with second granularity.
///
/// If the timer was started more than `run_time` seconds ago return `true`, otherwise
/// return `false`.
///
/// This function accepts started as a
/// [`std::time::Instant`](https://doc.rust-lang.org/std/time/struct.Instant.html). It
/// expects `run_time` in seconds.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// let started = std::time::Instant::now();
/// let mut counter = 0;
/// loop {
///     // Track how many times this loop runs.
///     counter += 1;
///
///     // Sleep for a quarter of a second.
///     std::thread::sleep(std::time::Duration::from_millis(250));
///
///     // Do stuff ...
///
///     // Loop until the timer expires, then break.
///     if util::timer_expired(started, 1) {
///         break
///     }
/// }
///
/// // It took 4 loops for the timer to expire.
/// assert_eq!(counter, 4);
/// ```
pub fn timer_expired(started: time::Instant, run_time: usize) -> bool {
    run_time > 0 && started.elapsed().as_secs() >= run_time as u64
}

/// Determine if a timer expired, with millisecond granularity.
///
/// If the timer was started more than `run_time` milliseconds ago return `true`,
/// otherwise return `false`.
///
/// This function accepts started as a
/// [`std::time::Instant`](https://doc.rust-lang.org/std/time/struct.Instant.html). It
/// expects `run_time` in milliseconds.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// let started = std::time::Instant::now();
/// let mut counter = 0;
/// loop {
///     // Track how many times this loop runs.
///     counter += 1;
///
///     // Sleep for a quarter of a second.
///     std::thread::sleep(std::time::Duration::from_millis(100));
///
///     // Do stuff ...
///
///     // Loop until the timer expires, then break.
///     if util::ms_timer_expired(started, 750) {
///         break
///     }
/// }
///
/// // It took 8 loops for the timer to expire. (Total time in the loop was 800 ms).
/// assert_eq!(counter, 8);
/// ```
pub fn ms_timer_expired(started: time::Instant, elapsed: usize) -> bool {
    elapsed > 0 && started.elapsed().as_millis() >= elapsed as u128
}

/// Convert optional string to f32, otherwise defaulting to 1.0.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // No decimal returns a proper float.
/// assert_eq!(util::get_hatch_rate(Some("1".to_string())), 1.0);
///
/// // Leading decimal returns a proper float.
/// assert_eq!(util::get_hatch_rate(Some(".1".to_string())), 0.1);
///
/// // Valid float string returns a proper float.
/// assert_eq!(util::get_hatch_rate(Some("1.1".to_string())), 1.1);
///
/// // Invalid number with too many decimals returns the defaut of 1.0.
/// assert_eq!(util::get_hatch_rate(Some("1.1.1".to_string())), 1.0);
///
/// // No number returns the defaut of 1.0.
/// assert_eq!(util::get_hatch_rate(None), 1.0);
/// ```
pub fn get_hatch_rate(hatch_rate: Option<String>) -> f32 {
    if let Some(value) = get_float_from_string(hatch_rate) {
        value
    } else {
        1.0
    }
}

/// Convert optional string to f32, otherwise return None.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // No decimal returns a proper float.
/// assert_eq!(util::get_float_from_string(Some("1".to_string())), Some(1.0));
///
/// // Leading decimal returns a proper float.
/// assert_eq!(util::get_float_from_string(Some(".1".to_string())), Some(0.1));
///
/// // Valid float string returns a proper float.
/// assert_eq!(util::get_float_from_string(Some("1.1".to_string())), Some(1.1));
///
/// // Invalid number with too many decimals returns None.
/// assert_eq!(util::get_float_from_string(Some("1.1.1".to_string())), None);
///
/// // No number returns None.
/// assert_eq!(util::get_float_from_string(None), None);
/// ```
pub fn get_float_from_string(string: Option<String>) -> Option<f32> {
    match string {
        Some(s) => match s.parse::<f32>() {
            Ok(value) => Some(value),
            Err(e) => {
                warn!("failed to convert {} to float: {}", s, e);
                None
            }
        },
        None => None,
    }
}

/// Helper function to determine if a host can be parsed.
///
/// # Example
/// ```rust
/// use goose::util;
///
/// // Hostname is a valid URL.
/// assert_eq!(util::is_valid_host("http://localhost/").is_ok(), true);
///
/// // IP is a valid URL.
/// assert_eq!(util::is_valid_host("http://127.0.0.1").is_ok(), true);
///
/// // URL with path is a valid URL.
/// assert_eq!(util::is_valid_host("https://example.com/foo").is_ok(), true);
///
/// // Protocol is required
/// assert_eq!(util::is_valid_host("example.com/").is_ok(), false);
/// ```
pub fn is_valid_host(host: &str) -> Result<bool, GooseError> {
    Url::parse(host).map_err(|parse_error| GooseError::InvalidHost {
        host: host.to_string(),
        detail: "Invalid host.".to_string(),
        parse_error,
    })?;
    Ok(true)
}

// Internal helper to configure the control-c handler. Shutdown cleanly on the first
// ctrl-c. Exit abruptly on the second ctrl-c.
pub(crate) fn setup_ctrlc_handler() {
    match ctrlc::set_handler(move || {
        // We've caught a ctrl-c, determine if it's the first time or an additional time.
        if *CANCELED.read().unwrap() {
            warn!("caught another ctrl-c, exiting immediately...");
            std::process::exit(1);
        } else {
            warn!("caught ctrl-c, stopping...");
            let mut canceled = CANCELED.write().unwrap();
            *canceled = true;
        }
    }) {
        Ok(_) => (),
        Err(e) => {
            // When running in tests, reset CANCELED with each new test allowing testing
            // of the ctrl-c handler.
            let mut canceled = CANCELED.write().unwrap();
            *canceled = false;
            info!("reset ctrl-c handler: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timespan() {
        assert_eq!(parse_timespan("0"), 0);
        assert_eq!(parse_timespan("foo"), 0);
        assert_eq!(parse_timespan("1"), 1);
        assert_eq!(parse_timespan("1s"), 1);
        assert_eq!(parse_timespan("1m"), 60);
        assert_eq!(parse_timespan("61"), 61);
        assert_eq!(parse_timespan("1m1s"), 61);
        assert_eq!(parse_timespan("10m"), 600);
        assert_eq!(parse_timespan("10m5s"), 605);
        assert_eq!(parse_timespan("15mins"), 900);
        assert_eq!(parse_timespan("60m"), 3600);
        assert_eq!(parse_timespan("1h"), 3600);
        assert_eq!(parse_timespan("1h15s"), 3615);
        assert_eq!(parse_timespan("1h5m"), 3900);
        assert_eq!(parse_timespan("1h5m13s"), 3913);
        assert_eq!(parse_timespan("2h3min"), 7380);
        assert_eq!(parse_timespan("3h3m"), 10980);
        assert_eq!(parse_timespan("3h3m5s"), 10985);
        assert_eq!(parse_timespan("5hours"), 18000);
        assert_eq!(parse_timespan("450m"), 27000);
        assert_eq!(parse_timespan("24h"), 86400);
        assert_eq!(parse_timespan("88h88m88s"), 322168);
        assert_eq!(parse_timespan("100hourblah"), 360000);
    }

    #[test]
    fn greatest_common_divisor() {
        assert_eq!(gcd(2, 4), 2);
        assert_eq!(gcd(1, 4), 1);
        assert_eq!(gcd(9, 102), 3);
        assert_eq!(gcd(12345, 98765), 5);
        assert_eq!(gcd(2, 99), 1);
        // More complicated two-part GCD
        assert_eq!(gcd(gcd(30, 90), 60), 30);
        assert_eq!(gcd(gcd(25, 7425), gcd(15, 9025)), 5);
    }

    #[test]
    fn median_test() {
        // Simple median test - add 3 numbers and pick the middle one.
        let mut btree: BTreeMap<usize, usize> = BTreeMap::new();
        btree.insert(1, 1);
        btree.insert(2, 1);
        btree.insert(3, 1);
        // 1: 1, 2: 1, 3: 1
        assert_eq!(median(&btree, 3, 1, 3), 2);
        assert_eq!(median(&btree, 3, 1, 1), 1);
        assert_eq!(median(&btree, 3, 3, 3), 3);
        btree.insert(1, 2);
        // 1: 2, 2: 1, 3: 1
        // We don't do a true median, we find the first value that is positioned >= 1/2 way
        // into the total btree size
        assert_eq!(median(&btree, 3, 1, 3), 1);
        btree.insert(4, 1);
        btree.insert(5, 1);
        // 1: 2, 2: 1, 3: 1, 4: 1, 5: 1
        assert_eq!(median(&btree, 6, 1, 5), 2);
        btree.insert(6, 1);
        btree.insert(7, 2);
        btree.insert(8, 1);
        btree.insert(9, 2);
        // 1: 2, 2: 1, 3: 1, 4: 1, 5: 1, 6: 1, 7: 2, 8: 1, 9: 2
        assert_eq!(median(&btree, 12, 1, 9), 5);

        // Confirm we're counting and not just returning the key.
        let mut btree: BTreeMap<usize, usize> = BTreeMap::new();
        btree.insert(2, 1);
        btree.insert(5, 1);
        btree.insert(25, 1);
        // 2: 1, 5: 1, 25: 1
        assert_eq!(median(&btree, 3, 2, 25), 5);
        btree.insert(5, 3);
        // 2: 1, 5: 3, 25: 1
        assert_eq!(median(&btree, 4, 2, 25), 5);
        btree.insert(25, 10);
        // 2: 1, 5: 3, 25: 10
        assert_eq!(median(&btree, 14, 2, 25), 25);
        btree.insert(100, 5);
        // 2: 1, 5: 3, 25: 10, 100: 5
        assert_eq!(median(&btree, 19, 2, 100), 25);
        btree.insert(100, 20);
        // 2: 1, 5: 3, 25: 20, 100: 5
        assert_eq!(median(&btree, 29, 2, 100), 100);

        // We round response times, be sure we return min or max when appropriate.
        let mut btree: BTreeMap<usize, usize> = BTreeMap::new();
        btree.insert(100, 3);
        btree.insert(210, 1);
        btree.insert(240, 1);
        // 100: 3, 210: 1, 240: 1
        // Minimum is more than median, use minimum.
        assert_eq!(median(&btree, 5, 101, 243), 101);
        btree.insert(240, 1);
        // 100: 3, 210: 1, 240: 5
        // Maximum is less than median, use maximum.
        assert_eq!(median(&btree, 9, 101, 239), 239);
    }

    #[test]
    fn truncate() {
        assert_eq!(
            truncate_string("the quick brown fox", 25),
            "the quick brown fox"
        );
        assert_eq!(truncate_string("the quick brown fox", 10), "the quic..");
        assert_eq!(truncate_string("abcde", 5), "abcde");
        assert_eq!(truncate_string("abcde", 4), "ab..");
        assert_eq!(truncate_string("abcde", 3), "a..");
        assert_eq!(truncate_string("abcde", 2), "..");
        assert_eq!(truncate_string("これはテストだ", 10), "これはテストだ");
        assert_eq!(truncate_string("これはテストだ", 3), "こ..");
        assert_eq!(truncate_string("这是一个测试。", 10), "这是一个测试。");
        assert_eq!(truncate_string("这是一个测试。", 3), "这..");
        assert_eq!(
            truncate_string("이것은 테스트입니다.", 15),
            "이것은 테스트입니다."
        );
        assert_eq!(truncate_string("이것은 테스트입니다.", 3), "이..");
    }

    #[tokio::test]
    async fn timer() {
        let started = time::Instant::now();

        // 60 second timer has not expired.
        assert!(!timer_expired(started, 60));

        // Timer is disabled.
        assert!(!timer_expired(started, 0));

        let sleep_duration = time::Duration::from_secs(1);
        tokio::time::sleep(sleep_duration).await;

        // Timer is now expired.
        assert!(timer_expired(started, 1));
    }

    #[test]
    fn hatch_rate() {
        //  https://rust-lang.github.io/rust-clippy/master/index.html#float_cmp
        assert!((get_hatch_rate(Some("1".to_string())) - 1.0).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some("1.0".to_string())) - 1.0).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some(".5".to_string())) - 0.5).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some("0.5".to_string())) - 0.5).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some(".12345".to_string())) - 0.12345).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some("12.345".to_string())) - 12.345).abs() < f32::EPSILON);
        // Defaults to 1.0.
        assert!((get_hatch_rate(None) - 1.0).abs() < f32::EPSILON);
        // Also on invalid input, defaults to 1.0.
        assert!((get_hatch_rate(Some("g".to_string())) - 1.0).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some("2.1f".to_string())) - 1.0).abs() < f32::EPSILON);
        assert!((get_hatch_rate(Some("1.1.1".to_string())) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn valid_host() {
        assert!(is_valid_host("http://example.com").is_ok());
        assert!(is_valid_host("example.com").is_err());
        assert!(is_valid_host("http://example.com/").is_ok());
        assert!(is_valid_host("example.com/").is_err());
        assert!(is_valid_host("https://www.example.com/and/with/path").is_ok());
        assert!(is_valid_host("www.example.com/and/with/path").is_err());
        assert!(is_valid_host("foo://example.com").is_ok());
        assert!(is_valid_host("file:///path/to/file").is_ok());
        assert!(is_valid_host("/path/to/file").is_err());
        assert!(is_valid_host("http://").is_err());
        assert!(is_valid_host("http://foo").is_ok());
        assert!(is_valid_host("http:///example.com").is_ok());
        assert!(is_valid_host("http:// example.com").is_err());
    }
}
