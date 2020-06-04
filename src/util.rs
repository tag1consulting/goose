use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time;

use regex::Regex;

/// Parse a string representing a time span and return the number of seconds.
/// Valid formats are: 20, 20s, 3m, 2h, 1h20m, 3h30m10s, etc.
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

/// Calculate the greatest commond divisor using binary GCD (or Stein's) algorithm.
/// More detail: https://en.wikipedia.org/wiki/Binary_GCD_algorithm
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

/// Calculate median for a BTreeMap of usizes.
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
pub fn truncate_string(str_to_truncate: &str, max_length: u64) -> String {
    let mut string_to_truncate = str_to_truncate.to_string();
    if string_to_truncate.len() as u64 > max_length {
        let truncated_length = max_length - 2;
        string_to_truncate.truncate(truncated_length as usize);
        string_to_truncate += "..";
    }
    string_to_truncate
}

/// If run_time was specified, detect when it's time to shut down
pub fn timer_expired(started: time::Instant, run_time: usize) -> bool {
    run_time > 0 && started.elapsed().as_secs() >= run_time as u64
}

pub fn setup_ctrlc_handler(canceled: &Arc<AtomicBool>) {
    let caught_ctrlc = canceled.clone();
    match ctrlc::set_handler(move || {
        // We've caught a ctrl-c, determine if it's the first time or an additional time.
        if caught_ctrlc.load(Ordering::SeqCst) {
            warn!("caught another ctrl-c, exiting immediately...");
            std::process::exit(1);
        } else {
            warn!("caught ctrl-c, stopping...");
            caught_ctrlc.store(true, Ordering::SeqCst);
        }
    }) {
        Ok(_) => (),
        Err(e) => {
            info!("failed to set ctrl-c handler: {}", e);
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
    }

    #[test]
    fn timer() {
        use std::thread;

        let started = time::Instant::now();

        // 60 second timer has not expired.
        let expired = timer_expired(started, 60);
        assert_eq!(expired, false);

        // Timer is disabled.
        let expired = timer_expired(started, 0);
        assert_eq!(expired, false);

        let sleep_duration = time::Duration::from_secs(1);
        thread::sleep(sleep_duration);

        // Timer is now expired.
        let expired = timer_expired(started, 1);
        assert_eq!(expired, true);
    }
}
