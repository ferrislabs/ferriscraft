use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Xp(pub u64);

impl Xp {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl fmt::Display for Xp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} XP", self.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coins(pub u64);

impl Coins {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    pub const fn checked_sub(self, price: Self) -> Result<Self, InsufficientCoins> {
        match self.0.checked_sub(price.0) {
            Some(rest) => Ok(Self(rest)),
            None => Err(InsufficientCoins {
                missing: Self(price.0 - self.0),
            }),
        }
    }

    #[must_use]
    pub const fn scale_floor(self, numerator: u64, denominator: u64) -> Self {
        Self(self.0.saturating_mul(numerator) / denominator)
    }
}

impl fmt::Display for Coins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} coins", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{missing} missing")]
pub struct InsufficientCoins {
    pub missing: Coins,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn checked_sub_reports_the_missing_amount() {
        assert_eq!(
            Coins(400).checked_sub(Coins(500)),
            Err(InsufficientCoins {
                missing: Coins(100)
            })
        );
        assert_eq!(Coins(600).checked_sub(Coins(500)), Ok(Coins(100)));
    }

    #[test]
    fn scale_floor_rounds_down() {
        assert_eq!(Coins(15).scale_floor(3, 2), Coins(22));
        assert_eq!(Coins(30).scale_floor(3, 2), Coins(45));
    }

    proptest! {
        #[test]
        fn checked_sub_never_goes_below_zero(balance in any::<u64>(), price in any::<u64>()) {
            match Coins(balance).checked_sub(Coins(price)) {
                Ok(rest) => prop_assert_eq!(rest.get() + price, balance),
                Err(InsufficientCoins { missing }) => {
                    prop_assert!(price > balance);
                    prop_assert_eq!(balance + missing.get(), price);
                }
            }
        }
    }
}
