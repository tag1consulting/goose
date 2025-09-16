use crate::metrics::NullableFloat;
use std::fmt::{Debug, Display, Formatter, Write};

/// The absolute value of isize::MIN as usize, used for overflow protection in delta calculations.
const ISIZE_MIN_ABS: usize = (isize::MIN as i128).unsigned_abs() as usize;

/// A value that can be used to provide a delta
///
/// As the actual value can be an unsigned type, we require an associated type which defines the
/// type of the delta.
pub trait DeltaValue: Clone + Debug + Display {
    type Delta: Clone + Display;

    /// Generate the delta between this and the provided value
    fn delta(self, value: Self) -> Self::Delta;

    /// It's positive if it's not negative or zero
    fn is_delta_positive(value: Self::Delta) -> bool;
}

impl DeltaValue for usize {
    type Delta = isize;

    fn delta(self, value: Self) -> Self::Delta {
        if self >= value {
            // the result will be positive, so just limit to isize::MAX
            (self - value).min(isize::MAX as usize) as isize
        } else {
            // the result will be negative, we will calculate the absolute value of that...
            let delta = value - self;
            if delta > ISIZE_MIN_ABS {
                // ... which is too big to fit into the negative space of isize, so we limit to isize::MIN
                isize::MIN
            } else {
                // ... which fits, so we return the negative value
                -(delta as isize)
            }
        }
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
        !value.is_sign_negative()
    }
}

impl DeltaValue for String {
    type Delta = String;

    fn delta(self, _value: Self) -> Self::Delta {
        // For strings, we don't calculate meaningful deltas
        // Just return empty string as delta
        String::new()
    }

    fn is_delta_positive(_value: Self::Delta) -> bool {
        // String deltas are not meaningful, so always return false
        false
    }
}

/// A value, being either a plain value of a value with delta to a baseline
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, PartialOrd)]
#[serde(untagged)]
pub enum Value<T: DeltaValue> {
    Plain(T),
    Delta { value: T, delta: T::Delta },
}

impl<T: DeltaValue + Eq> Eq for Value<T> where T::Delta: Eq {}

impl<T: DeltaValue + std::hash::Hash> std::hash::Hash for Value<T>
where
    T::Delta: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Plain(value) => {
                0u8.hash(state); // Discriminant for Plain variant
                value.hash(state);
            }
            Value::Delta { value, delta } => {
                1u8.hash(state); // Discriminant for Delta variant
                value.hash(state);
                delta.hash(state);
            }
        }
    }
}

impl<T: DeltaValue> Value<T> {
    /// Check if the underlying value is empty (for types that support it)
    pub fn is_empty(&self) -> bool
    where
        T: IsEmpty,
    {
        match self {
            Value::Plain(value) => value.is_empty(),
            Value::Delta { value, .. } => value.is_empty(),
        }
    }
}

/// Trait for types that can check if they are empty
pub trait IsEmpty {
    fn is_empty(&self) -> bool;
}

impl IsEmpty for String {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
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
                let current_value = value.clone();
                *self = Self::Delta {
                    value: current_value.clone(),
                    delta: current_value.delta(other),
                };
            }
            Self::Delta { value, delta: _ } => {
                let current_value = value.clone();
                *self = Self::Delta {
                    value: current_value.clone(),
                    delta: current_value.delta(other),
                }
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
            Self::Plain(value) => value.clone(),
            Self::Delta { value, delta: _ } => value.clone(),
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
                if T::is_delta_positive(delta.clone()) {
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

/// Build a delta to a baseline
pub trait DeltaTo {
    fn delta_to(&mut self, other: &Self);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::metrics::Value;

    #[test]
    fn eval_optional() {
        assert_eq!(
            {
                let mut value = Some(Value::Plain(10));
                value.eval(Some(Value::Plain(5)));
                value
            },
            Some(Value::Delta {
                value: 10,
                delta: 5
            })
        );

        assert_eq!(
            {
                let mut value = None;
                value.eval(Some(Value::Plain(5)));
                value
            },
            None
        );
    }

    #[test]
    fn delta_to_string() {
        assert_eq!(format!("{}", 0.delta(10)), "-10");
        assert_eq!(format!("{}", 10.delta(10)), "0");
        assert_eq!(format!("{}", 10.delta(0)), "10");
    }

    #[test]
    fn value_to_string() {
        fn value<T: DeltaValue>(value: T, baseline: T) -> Value<T> {
            let mut result = Value::from(value);
            result.diff(baseline);
            result
        }

        assert_eq!(format!("{}", value(0, 1000)), "0 (-1000)");
        assert_eq!(format!("{}", value(1000, 1000)), "1000 (0)");
        assert_eq!(format!("{}", value(1000, 0)), "1000 (+1000)");
    }
}
