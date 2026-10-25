use serde::{Deserialize, Deserializer, Serialize, de};
use ts_rs::TS;

pub const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// An integer that JSON clients can represent without rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(type = "number")]
pub struct SafeInteger(i64);

impl TryFrom<i64> for SafeInteger {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err("integer is outside the JavaScript safe integer range")
        }
    }
}

impl From<SafeInteger> for i64 {
    fn from(value: SafeInteger) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for SafeInteger {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(i64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(type = "number")]
pub struct Id(SafeInteger);

impl TryFrom<i64> for Id {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value <= 0 {
            return Err("ID must be positive");
        }
        SafeInteger::try_from(value).map(Self)
    }
}

impl From<Id> for i64 {
    fn from(value: Id) -> Self {
        value.0.into()
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(i64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
