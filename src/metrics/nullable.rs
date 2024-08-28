use crate::metrics::DeltaValue;
use serde::Deserializer;
use std::fmt::{Display, Formatter};
use std::ops::{Deref, DerefMut};

/// An `f32` which can deserialize from `null` as `NaN`.
///
/// Also see: <https://github.com/serde-rs/json/issues/202>
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, serde::Serialize)]
pub struct NullableFloat(pub f32);

impl Deref for NullableFloat {
    type Target = f32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NullableFloat {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<f32> for NullableFloat {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl<'de> serde::Deserialize<'de> for NullableFloat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<f32>::deserialize(deserializer)?;
        Ok(Self(value.unwrap_or(f32::NAN)))
    }
}

impl Display for NullableFloat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl DeltaValue for NullableFloat {
    type Delta = NullableFloat;

    fn delta(self, value: Self) -> Self::Delta {
        NullableFloat(self.0.delta(value.0))
    }

    fn is_delta_positive(value: Self::Delta) -> bool {
        f32::is_delta_positive(value.0)
    }
}
