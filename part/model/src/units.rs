// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Processing units: cores' worth of time, in steps of 0.01. 1.00 unit
//! is one whole core. Kept as hundredths, written as a decimal number.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

/// Processing units, in hundredths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Units(pub u32);

impl Units {
    /// One whole core.
    pub const ONE: Units = Units(100);
    /// The least a virtual processor may be given.
    pub const LEAST_PER_VIRTUAL: Units = Units(5);

    /// `cores` whole cores.
    pub fn cores(cores: u32) -> Self {
        Units(cores.saturating_mul(100))
    }

    /// The hundredths.
    pub fn hundredths(self) -> u32 {
        self.0
    }

    /// From a decimal number; `None` when it is negative, too large or
    /// not a step of 0.01.
    pub fn from_number(number: f64) -> Option<Self> {
        let scaled = number * 100.0;
        let rounded = scaled.round();
        if !(0.0..=1e8).contains(&rounded) || (scaled - rounded).abs() > 1e-6 {
            return None;
        }
        // In range and whole: checked just above.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(Units(rounded as u32))
    }
}

impl fmt::Display for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, self.0 % 100)
    }
}

impl ipc::Wire for Units {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }

    fn get(d: &mut ipc::Dec<'_>) -> Result<Self, ipc::WireError> {
        u32::get(d).map(Units)
    }
}

impl Serialize for Units {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(f64::from(self.0) / 100.0)
    }
}

struct UnitsVisitor;

impl Visitor<'_> for UnitsVisitor {
    type Value = Units;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("processing units in steps of 0.01")
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Units, E> {
        Units::from_number(v).ok_or_else(|| E::custom(format!("{v} is not a step of 0.01")))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Units, E> {
        u32::try_from(v)
            .ok()
            .and_then(|whole| whole.checked_mul(100))
            .map(Units)
            .ok_or_else(|| E::custom(format!("{v} is not a number of processing units")))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Units, E> {
        i64::try_from(v)
            .map_err(|_| E::custom(format!("{v} is not a number of processing units")))
            .and_then(|v| self.visit_i64(v))
    }
}

impl<'de> Deserialize<'de> for Units {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UnitsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Doc {
        units: Units,
    }

    #[test]
    fn units_are_written_as_a_decimal_number_and_read_in_steps_of_a_hundredth() {
        assert_eq!(Units(50).to_string(), "0.50");
        assert_eq!(Units(205).to_string(), "2.05");
        assert_eq!(Units::cores(3), Units(300));
        let text = toml::to_string(&Doc { units: Units(50) }).unwrap();
        assert_eq!(text.trim(), "units = 0.5");
        for (text, units) in [
            ("units = 0.5", 50),
            ("units = 0.05", 5),
            ("units = 1.10", 110),
            ("units = 2", 200),
            ("units = 0.07", 7),
            ("units = 0.29", 29),
        ] {
            assert_eq!(toml::from_str::<Doc>(text).unwrap().units, Units(units));
        }
        for text in [
            "units = 0.505",
            "units = -1.0",
            "units = -1",
            "units = 1e12",
        ] {
            assert!(toml::from_str::<Doc>(text).is_err(), "{text}");
        }
    }
}
