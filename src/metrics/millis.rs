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

/// `us` microseconds as milliseconds: to the hundredth, truncated and with
/// trailing zeros dropped, under 10 ms (`0.4`, `2.35`), whole above (`37`).
/// `separators` adds thousands separators to whole milliseconds.
pub(crate) fn format_micros(us: u64, separators: bool) -> String {
    if us < 10_000 && !us.is_multiple_of(1000) {
        let hundredths = us / 10;
        let text = format!("{}.{:02}", hundredths / 100, hundredths % 100);
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else if separators {
        (us / 1000).to_formatted_string(&Locale::en)
    } else {
        (us / 1000).to_string()
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

/// The difference between two [`Millis`], in microseconds, shown like them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct MillisDelta(pub i64);

impl Display for MillisDelta {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let text = format_micros(self.0.unsigned_abs(), false);
        if self.0 < 0 {
            f.pad(&format!("-{text}"))
        } else {
            f.pad(&text)
        }
    }
}

impl DeltaValue for Millis {
    type Delta = MillisDelta;

    /// To the microsecond when both are shown to the hundredth, otherwise
    /// between the whole milliseconds shown, so the delta reads as the
    /// difference of what is printed.
    fn delta(self, value: Self) -> Self::Delta {
        let (a, b) = if self.0 < 10_000 && value.0 < 10_000 {
            (self.0 as i128, value.0 as i128)
        } else {
            (
                (self.0 / 1000 * 1000) as i128,
                (value.0 / 1000 * 1000) as i128,
            )
        };
        MillisDelta((a - b).clamp(i64::MIN as i128, i64::MAX as i128) as i64)
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
        assert_eq!(Millis(429).to_string(), "0.42");
        assert_eq!(Millis(5).to_string(), "0");
        assert_eq!(Millis(2_350).to_string(), "2.35");
        assert_eq!(Millis(3_000).to_string(), "3");
        assert_eq!(Millis(9_999).to_string(), "9.99");
        assert_eq!(Millis(37_000).to_string(), "37");
        assert_eq!(Millis(37_999).to_string(), "37");
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
        // 11.7 ms prints as 11, so its delta to 8 ms is 3, not 3.7.
        assert_eq!(Millis(11_700).delta(Millis(8_000)).to_string(), "3");
        assert_eq!(Millis(8_000).delta(Millis(11_700)).to_string(), "-3");
        assert!(Millis::is_delta_non_negative(MillisDelta(0)));
    }
}
