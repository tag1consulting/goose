use num_format::ToFormattedStr;
use std::fmt::{Debug, Display, Formatter, Write};

pub trait DeltaValue: Copy + Debug + Display {
    type Delta: Copy + Display;

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
            if delta > 9223372036854775808
            /* the absolute value of isize::MIN as usize */
            {
                // ... which is too big to fix into the negative space of isize, so we limit to isize::MIN
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

#[derive(Copy, Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Value<T: DeltaValue> {
    Plain(T),
    Delta { value: T, delta: T::Delta },
}

impl<T: DeltaValue> From<T> for Value<T> {
    fn from(value: T) -> Self {
        Self::Plain(value)
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

/// Build a delta to a baseline
pub trait DeltaTo {
    fn delta_to(&mut self, other: &Self);
}

pub struct Formatted<T>(pub T);

impl<T: ToFormattedStr> Display for Formatted<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use num_format::{Locale, ToFormattedString};

        f.write_str(&self.0.to_formatted_string(&Locale::en))?;

        Ok(())
    }
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
}
