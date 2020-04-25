use std::cmp::{min, max};
use std::str::FromStr;
use regex::Regex;

/// Parse a string representing a time span and return the number of seconds.
/// Valid formats are: 20, 20s, 3m, 2h, 1h20m, 3h30m10s, etc.
pub fn parse_timespan(time_str: &str) -> usize {
    let time = match usize::from_str(time_str) {
        // If an integer is passed in, assume it's seconds
        Ok(t) => {
            trace!("{} is integer: {} seconds", time_str, t);
            t
        }
        // Otherwise use a regex to extract hours, minutes and seconds from string.
        Err(_) => {
            let re = Regex::new(r"((?P<hours>\d+?)h)?((?P<minutes>\d+?)m)?((?P<seconds>\d+?)s)?").unwrap();
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
            trace!("{} hours {} minutes {} seconds: {} seconds", hours, minutes, seconds, total);
            total
        }
    };
    time
}

/// Calculate the greatest commond divisor using binary GCD (or Stein's) algorithm.
/// More detail: https://en.wikipedia.org/wiki/Binary_GCD_algorithm
pub fn gcd(u: usize, v: usize) -> usize {
    let gcd = match ((u, v), (u & 1, v & 1)) {
        ((x, y), _) if x == y               => x,
        ((x, y), (0, 1)) | ((y, x), (1, 0)) => gcd(x >> 1, y),
        ((x, y), (0, 0))                    => gcd(x >> 1, y >> 1) << 1,
        ((x, y), (1, 1))                    => { let (x, y) = (min(x, y), max(x, y)); 
                                                 gcd((y - x) >> 1, x) 
                                               }
        _                                   => unreachable!(),
    };
    gcd
}

/// Calculate mean for a vector of f32s.
pub fn mean(list: &[usize]) -> usize {
    let sum: usize = Iterator::sum(list.iter());
    usize::from(sum) / (list.len() as usize)
}

/// Calculate median for a vector of f32s.
pub fn median(list: &[usize]) -> usize {
    let len = list.len();
    let mid = len / 2;
    if len % 2 == 0 {
        mean(&list[(mid - 1)..(mid + 1)])
    } else {
        list[mid]
    }
}

/// Truncate strings when they're too long to display.
pub fn truncate_string(str_to_truncate: &str, max_length: u64) -> String {
    let mut string_to_truncate = str_to_truncate.to_string();
    if string_to_truncate.len() as u64 > max_length {
        let truncated_length = max_length - 2;
        string_to_truncate.truncate(truncated_length as usize);
        string_to_truncate = string_to_truncate + "..";
    }
    string_to_truncate
}
