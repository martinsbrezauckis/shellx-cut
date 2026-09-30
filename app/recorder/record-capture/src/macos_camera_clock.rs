//! Exact video ticks and the precision of ffprobe's decimal duration text.

use record_core::{error_codes, RecordError, Result};

#[derive(Debug, Clone, Copy)]
pub(super) struct TimeBase {
    pub(super) num: u64,
    pub(super) den: u64,
}

impl TimeBase {
    pub(super) fn parse(value: &str) -> Result<Self> {
        let (num, den) = value.split_once('/').ok_or_else(invalid)?;
        let clock = Self {
            num: num.parse().map_err(|_| invalid())?,
            den: den.parse().map_err(|_| invalid())?,
        };
        if clock.num == 0 || clock.den == 0 {
            return Err(invalid());
        }
        clock.scaled(1)?;
        Ok(clock)
    }

    // Values share denominator den; division occurs only at the public ns edge.
    pub(super) fn scaled(self, ticks: u64) -> Result<u128> {
        u128::from(ticks)
            .checked_mul(u128::from(self.num))
            .and_then(|v| v.checked_mul(1_000_000_000))
            .ok_or_else(invalid)
    }

    pub(super) fn ns_scaled(self, ns: u64) -> u128 {
        u128::from(ns) * u128::from(self.den)
    }

    pub(super) fn floor_ns(self, ticks: u64) -> Result<u64> {
        u64::try_from(self.scaled(ticks)? / u128::from(self.den)).map_err(|_| invalid())
    }

    pub(super) fn rounded_ms(self, ticks: u64) -> Result<u64> {
        let unit = u128::from(self.den) * 1_000_000;
        u64::try_from(
            self.scaled(ticks)?
                .checked_add(unit / 2)
                .ok_or_else(invalid)?
                / unit,
        )
        .map_err(|_| invalid())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DecimalDuration {
    pub(super) ns: u64,
    pub(super) precision_ns: u64,
}

impl DecimalDuration {
    pub(super) fn parse(value: &str) -> Result<Self> {
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if fraction.len() > 9
            || whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let unit = 10_u64.pow((9 - fraction.len()) as u32);
        let seconds = whole.parse::<u64>().map_err(|_| invalid())?;
        let sub = if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u64>().map_err(|_| invalid())? * unit
        };
        Ok(Self {
            ns: seconds
                .checked_mul(1_000_000_000)
                .and_then(|v| v.checked_add(sub))
                .ok_or_else(invalid)?,
            // ffprobe prints nearest decimal: half its last printed unit.
            // At nine digits, retain the half-ns bound by doubling comparisons.
            precision_ns: unit,
        })
    }

    pub(super) fn matches(self, clock: TimeBase, ticks: u64) -> Result<bool> {
        let delta = clock.scaled(ticks)?.abs_diff(clock.ns_scaled(self.ns));
        Ok(delta.checked_mul(2).ok_or_else(invalid)? <= clock.ns_scaled(self.precision_ns))
    }
}

fn invalid() -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "verify macOS camera movie clock",
        "movie rational clock is invalid or overflowed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_endpoint_has_only_the_precision_that_was_printed() {
        for (base, ticks, text) in [
            ("1/24", 1, "0.041667"),
            ("1/600", 1, "0.001667"),
            ("1/600", 2, "0.003333"),
        ] {
            let clock = TimeBase::parse(base).unwrap();
            assert!(DecimalDuration::parse(text)
                .unwrap()
                .matches(clock, ticks)
                .unwrap());
        }
        let clock = TimeBase::parse("1/600").unwrap();
        assert!(!DecimalDuration::parse("0.003332")
            .unwrap()
            .matches(clock, 2)
            .unwrap());
        assert!(!DecimalDuration::parse("0.003333000")
            .unwrap()
            .matches(clock, 2)
            .unwrap());
        let clock = TimeBase::parse("1/2000000000").unwrap();
        assert!(DecimalDuration::parse("0.000000001")
            .unwrap()
            .matches(clock, 1)
            .unwrap());
        assert!(!DecimalDuration::parse("0.000000002")
            .unwrap()
            .matches(clock, 1)
            .unwrap());
    }

    #[test]
    fn invalid_or_overflowed_clocks_refuse() {
        for base in ["0/600", "1/0", "1", "-1/600", "1/18446744073709551616"] {
            assert!(TimeBase::parse(base).is_err());
        }
        let clock = TimeBase {
            num: u64::MAX,
            den: 1,
        };
        assert!(clock.scaled(u64::MAX).is_err());
        assert!(clock.floor_ns(1).is_err());
        for text in ["", "-1.000", "1.0000000001", "18446744073709551615"] {
            assert!(DecimalDuration::parse(text).is_err());
        }
    }
}
