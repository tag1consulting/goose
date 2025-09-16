use crate::metrics::NullableFloat;
use num_format::{Format, ToFormattedString};
use std::fmt::{Debug, Display, Formatter, Write};

/// A value that can be used to provide a delta
///
/// As the actual value can be an unsigned type, we require an associated type which defines the
/// type of the delta.
pub trait DeltaValue: Copy + Debug + Display {
    type Delta: Copy + Display;

    /// Generate the delta between this and the provided value
    fn delta(self, value: Self) -> Self::Delta;

    /// It's positive if it's not negative or zero
    fn is_delta_positive(value: Self::Delta) -> bool;
}

impl DeltaValue for usize {
    type Delta = isize;

    fn delta(self, value: Self) -> Self::Delta {
        let delta = (self as i128) - (value as i128);
        delta.clamp(isize::MIN as i128, isize::MAX as i128) as isize
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        value.is_positive()
    }
}

impl DeltaValue for f32 {
    type Delta = f32;

    fn delta(self, value: Self) -> Self::Delta {
        self - value
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        value > 0.0
    }
}

impl DeltaValue for u64 {
    type Delta = i64;

    fn delta(self, value: Self) -> Self::Delta {
        let delta = (self as i128) - (value as i128);
        delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        value.is_positive()
    }
}

impl DeltaValue for u16 {
    type Delta = i16;

    fn delta(self, value: Self) -> Self::Delta {
        (self as i32 - value as i32) as i16 // No overflow possible
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        value.is_positive()
    }
}

impl DeltaValue for f64 {
    type Delta = f64;

    fn delta(self, value: Self) -> Self::Delta {
        self - value
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        value > 0.0
    }
}

/// A value, being either a plain value of a value with delta to a baseline
#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Value<T: DeltaValue> {
    Plain(T),
    Delta { value: T, delta: T::Delta },
}

impl<T: DeltaValue> From<T> for Value<T> {
    fn from(value: T) -> Self {
        Self::Plain(value)
    }
}

impl From<f32> for Value<NullableFloat> {
    fn from(value: f32) -> Self {
        Self::Plain(NullableFloat(value))
    }
}

impl<T: DeltaValue> Value<T> {
    pub fn diff(&mut self, other: T) {
        match self {
            Self::Plain(value) => {
                *self = Self::Delta {
                    value: *value,
                    delta: value.delta(other),
                };
            }
            Self::Delta { value, delta: _ } => {
                *self = Self::Delta {
                    value: *value,
                    delta: value.delta(other),
                }
            }
        }
    }
}

impl<T> Value<T>
where
    T: DeltaValue + ToFormattedString,
    <T as DeltaValue>::Delta: ToFormattedString,
{
    pub fn formatted_number(&self, format: &impl Format) -> String {
        match self {
            Self::Plain(value) => value.to_formatted_string(format),
            Self::Delta { value, delta } => {
                let s = if T::is_delta_positive(*delta) {
                    "+"
                } else {
                    ""
                };
                format!(
                    "{} ({s}{})",
                    value.to_formatted_string(format),
                    delta.to_formatted_string(format)
                )
            }
        }
    }
}

impl<T: DeltaValue> DeltaEval<T> for Value<T> {
    fn eval(&mut self, other: Self) {
        self.diff(other.value())
    }
}

impl<T: DeltaValue> DeltaEval<T> for Option<Value<T>> {
    fn eval(&mut self, other: Self) {
        if let (Some(value), Some(other)) = (self, other) {
            value.eval(other);
        }
    }
}

pub trait DeltaEval<T: DeltaValue> {
    fn eval(&mut self, other: Self);
}

impl<T: DeltaValue> Value<T> {
    pub fn value(&self) -> T {
        match self {
            Self::Plain(value) => *value,
            Self::Delta { value, delta: _ } => *value,
        }
    }
}

impl<T: DeltaValue> Display for Value<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(value) => Display::fmt(value, f),
            Self::Delta { value, delta } => {
                // we can pass on the actual value
                Display::fmt(value, f)?;

                // format delta as `({delta:+})`, keeping the actual format options
                f.write_str(" (")?;

                // for the delta, we want a plus sign, in the case of a positive value, zero excluded
                if T::is_delta_positive(*delta) {
                    f.write_char('+')?;
                    Display::fmt(delta, f)?;
                } else {
                    Display::fmt(delta, f)?;
                }

                f.write_char(')')?;

                // done
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::metrics::Value;
    use num_format::Locale;

    #[test]
    fn eval_optional() {
        assert_eq!(
            {
                let mut value = Some(Value::Plain(10usize));
                value.eval(Some(Value::Plain(5usize)));
                value
            },
            Some(Value::Delta {
                value: 10usize,
                delta: 5isize
            })
        );

        assert_eq!(
            {
                let mut value: Option<Value<usize>> = None;
                value.eval(Some(Value::Plain(5usize)));
                value
            },
            None
        );
    }

    #[test]
    fn delta_to_string() {
        assert_eq!(format!("{}", 0usize.delta(10usize)), "-10");
        assert_eq!(format!("{}", 10usize.delta(10usize)), "0");
        assert_eq!(format!("{}", 10usize.delta(0usize)), "10");
    }

    #[test]
    fn value_to_string() {
        fn value<T: DeltaValue>(value: T, baseline: T) -> Value<T> {
            let mut result = Value::from(value);
            result.diff(baseline);
            result
        }

        assert_eq!(format!("{}", value(0usize, 1000usize)), "0 (-1000)");
        assert_eq!(format!("{}", value(1000usize, 1000usize)), "1000 (0)");
        assert_eq!(format!("{}", value(1000usize, 0usize)), "1000 (+1000)");
    }

    #[test]
    fn value_with_delta_to_string_num() {
        fn value<T: DeltaValue>(value: T, baseline: T) -> Value<T> {
            let mut result = Value::from(value);
            result.diff(baseline);
            result
        }

        assert_eq!(
            format!("{}", value(0usize, 1000usize).formatted_number(&Locale::en)),
            "0 (-1,000)"
        );
        assert_eq!(
            format!(
                "{}",
                value(1000usize, 1000usize).formatted_number(&Locale::en)
            ),
            "1,000 (0)"
        );
        assert_eq!(
            format!("{}", value(1000usize, 0usize).formatted_number(&Locale::en)),
            "1,000 (+1,000)"
        );
    }

    #[test]
    fn value_to_string_num() {
        assert_eq!(
            format!("{}", Value::from(1000usize).formatted_number(&Locale::en)),
            "1,000"
        );
    }
}
