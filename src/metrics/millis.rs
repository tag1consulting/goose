use super::delta::{DeltaValue, Value};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};

/// A response time kept in microseconds and shown in milliseconds: to the
/// hundredth under 10 ms, where whole milliseconds hide the difference, and
/// whole above, as Goose always showed them.
///
/// It serializes as whole milliseconds, truncated as Goose always took them,
/// so a JSON report reads as it did and loads as a baseline in any Goose;
/// it deserializes from whole milliseconds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Millis(pub u64);

impl Millis {
    /// From whole milliseconds.
    pub(crate) fn from_ms(ms: usize) -> Self {
        Millis((ms as u64).saturating_mul(1000))
    }
}

/// The microseconds `us` shows as: rounded half up to the hundredth of a
/// millisecond under 10 ms, and to whole milliseconds from there.
fn shown_us(us: u64) -> u64 {
    let hundredths = us.saturating_add(5) / 10;
    if hundredths < 1_000 {
        hundredths * 10
    } else {
        us.saturating_add(500) / 1000 * 1000
    }
}

/// A multiple of 10 µs as milliseconds, with trailing zeros dropped (`0.4`,
/// `2.35`, `37`). `separators` adds thousands separators.
fn hundredths_text(us: u64, separators: bool) -> String {
    let whole = us / 1000;
    let whole = if separators {
        whole.to_formatted_string(&Locale::en)
    } else {
        whole.to_string()
    };
    match us % 1000 / 10 {
        0 => whole,
        hundredths => format!("{whole}.{hundredths:02}")
            .trim_end_matches('0')
            .to_string(),
    }
}

/// `us` microseconds as milliseconds, as Goose shows every time: to the
/// hundredth under 10 ms (`0.4`, `2.35`), whole from there (`38`), rounded
/// half up. `separators` adds thousands separators.
pub(crate) fn format_micros(us: u64, separators: bool) -> String {
    hundredths_text(shown_us(us), separators)
}

/// A difference of shown values, in microseconds, as milliseconds: `-`
/// when negative, never `-0`.
fn delta_text(us: i64, separators: bool) -> String {
    let text = hundredths_text(us.unsigned_abs(), separators);
    if us < 0 {
        format!("-{text}")
    } else {
        text
    }
}

impl Display for Millis {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.pad(&format_micros(self.0, false))
    }
}

impl PartialEq<usize> for Millis {
    /// Equal to whole milliseconds with nothing below them.
    fn eq(&self, other: &usize) -> bool {
        self.0 == (*other as u64).saturating_mul(1000)
    }
}

impl Serialize for Millis {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.0 / 1000).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Millis {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let ms = u64::deserialize(deserializer)?;
        Ok(Millis(ms.saturating_mul(1000)))
    }
}

impl From<usize> for Value<Millis> {
    /// From whole milliseconds.
    fn from(ms: usize) -> Self {
        Value::Plain(Millis::from_ms(ms))
    }
}

/// The difference between two [`Millis`] as shown, in microseconds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct MillisDelta(pub i64);

impl MillisDelta {
    /// With thousands separators.
    pub(crate) fn formatted(self) -> String {
        delta_text(self.0, true)
    }
}

impl Display for MillisDelta {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.pad(&delta_text(self.0, false))
    }
}

impl DeltaValue for Millis {
    type Delta = MillisDelta;

    /// The difference of the values shown, so the delta reads as the
    /// difference of what is printed.
    fn delta(self, value: Self) -> Self::Delta {
        let difference = shown_us(self.0) as i128 - shown_us(value.0) as i128;
        MillisDelta(difference.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
    }

    fn is_delta_non_negative(value: Self::Delta) -> bool {
        value.0 >= 0
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn formats_to_the_hundredth_under_ten_milliseconds() {
        assert_eq!(Millis(400).to_string(), "0.4");
        assert_eq!(Millis(420).to_string(), "0.42");
        assert_eq!(Millis(429).to_string(), "0.43");
        assert_eq!(Millis(4).to_string(), "0");
        assert_eq!(Millis(5).to_string(), "0.01");
        assert_eq!(Millis(2_350).to_string(), "2.35");
        assert_eq!(Millis(3_000).to_string(), "3");
        assert_eq!(Millis(9_994).to_string(), "9.99");
        assert_eq!(Millis(9_995).to_string(), "10");
        assert_eq!(Millis(37_000).to_string(), "37");
        assert_eq!(Millis(37_499).to_string(), "37");
        assert_eq!(Millis(37_999).to_string(), "38");
        assert_eq!(format_micros(1_234_000, true), "1,234");
        assert_eq!(format!("{:>6}", Millis(420)), "  0.42");
    }

    #[test]
    fn serializes_as_whole_milliseconds() {
        assert_eq!(serde_json::to_string(&Millis(37_999)).unwrap(), "37");
        assert_eq!(serde_json::to_string(&Millis(420)).unwrap(), "0");
        let back: Millis = serde_json::from_str("37").unwrap();
        assert_eq!(back, Millis(37_000));
        assert_eq!(back, 37usize);
    }

    #[test]
    fn deltas() {
        assert_eq!(Millis(420).delta(Millis(400)).to_string(), "0.02");
        assert_eq!(Millis(400).delta(Millis(420)).to_string(), "-0.02");
        assert_eq!(Millis(50_000).delta(Millis(40_000)).to_string(), "10");
        // 11.7 ms prints as 12, so its delta to 8 ms is 4, not 3.7.
        assert_eq!(Millis(11_700).delta(Millis(8_000)).to_string(), "4");
        assert_eq!(Millis(8_000).delta(Millis(11_700)).to_string(), "-4");
        assert!(Millis::is_delta_non_negative(MillisDelta(0)));
        // The difference of the values shown: 10,000 and 9,999 µs both show
        // 10, 2,351 and 2,349 µs both 2.35, and 2,341 and 2,344 µs both 2.34.
        let mut shown = Value::Plain(Millis(10_000));
        shown.diff(Millis(9_999));
        assert_eq!(shown.to_string(), "10 (+0)");
        assert_eq!(Millis(2_351).delta(Millis(2_349)).to_string(), "0");
        assert_eq!(Millis(2_341).delta(Millis(2_344)).to_string(), "0");
    }
}
