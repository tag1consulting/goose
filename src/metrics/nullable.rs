use serde::Deserializer;
use std::fmt::{Display, Formatter};
use std::ops::{Deref, DerefMut};

/// An `f64` which can deserialize from `null` as `NaN`. An `f64`, not an
/// `f32`, so a mean of thousands of milliseconds still prints exactly to the
/// hundredth.
///
/// Also see: <https://github.com/serde-rs/json/issues/202>
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, serde::Serialize)]
pub struct NullableFloat(pub f64);

impl Deref for NullableFloat {
    type Target = f64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NullableFloat {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<f64> for NullableFloat {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl From<f32> for NullableFloat {
    /// The `f64` with the digits the `f32` prints with, so a value still
    /// computed as an `f32` reports exactly as it did (`0.1`, not
    /// `0.10000000149011612`).
    fn from(value: f32) -> Self {
        Self(value.to_string().parse().unwrap_or(f64::NAN))
    }
}

impl<'de> serde::Deserialize<'de> for NullableFloat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<f64>::deserialize(deserializer)?;
        Ok(Self(value.unwrap_or(f64::NAN)))
    }
}

impl Display for NullableFloat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl crate::metrics::delta::DeltaValue for NullableFloat {
    type Delta = f64;

    fn delta(self, value: Self) -> Self::Delta {
        self.0 - value.0
    }

    fn is_delta_non_negative(value: Self::Delta) -> bool {
        value >= 0.0
    }
}
