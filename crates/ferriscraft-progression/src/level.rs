use std::fmt;
use std::num::NonZeroU32;

use crate::amount::Xp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(NonZeroU32);

impl Level {
    pub const ONE: Self = Self(NonZeroU32::MIN);

    #[must_use]
    pub const fn new(level: u32) -> Option<Self> {
        match NonZeroU32::new(level) {
            Some(level) => Some(Self(level)),
            None => None,
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    const fn saturating_add(self, levels: u32) -> Self {
        Self(self.0.saturating_add(levels))
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelProgress {
    pub level: Level,
    pub xp_into_level: Xp,
    pub xp_for_next: Xp,
    pub total: Xp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelCurve {
    step: u64,
    cap: u64,
}

impl LevelCurve {
    pub const GAME: Self = Self {
        step: 250,
        cap: 2_500,
    };
    pub const NETWORK: Self = Self {
        step: 1_000,
        cap: 10_000,
    };

    #[must_use]
    pub const fn xp_to_next(self, level: Level) -> Xp {
        let need = self.step.saturating_mul(level.get() as u64);
        Xp(if need < self.cap { need } else { self.cap })
    }

    #[must_use]
    pub fn progress(self, total: Xp) -> LevelProgress {
        let mut level = Level::ONE;
        let mut remaining = total.get();
        loop {
            let need = self.xp_to_next(level).get();
            if need >= self.cap {
                break;
            }
            if remaining < need {
                return LevelProgress {
                    level,
                    xp_into_level: Xp(remaining),
                    xp_for_next: Xp(need),
                    total,
                };
            }
            remaining -= need;
            level = level.saturating_add(1);
        }
        let extra_levels = u32::try_from(remaining / self.cap).unwrap_or(u32::MAX);
        LevelProgress {
            level: level.saturating_add(extra_levels),
            xp_into_level: Xp(remaining % self.cap),
            xp_for_next: Xp(self.cap),
            total,
        }
    }

    #[must_use]
    pub fn xp_to_reach(self, target: Level) -> Xp {
        let mut level = Level::ONE;
        let mut total = 0u64;
        while level < target {
            let need = self.xp_to_next(level).get();
            if need >= self.cap {
                let flat_levels = u64::from(target.get() - level.get());
                return Xp(total.saturating_add(flat_levels.saturating_mul(self.cap)));
            }
            total += need;
            level = level.saturating_add(1);
        }
        Xp(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn level(n: u32) -> Level {
        Level::new(n).unwrap()
    }

    #[test]
    fn a_new_player_is_level_one() {
        let progress = LevelCurve::GAME.progress(Xp::ZERO);
        assert_eq!(progress.level, Level::ONE);
        assert_eq!(progress.xp_into_level, Xp(0));
        assert_eq!(progress.xp_for_next, Xp(250));
    }

    #[test]
    fn game_curve_matches_the_spec() {
        assert_eq!(LevelCurve::GAME.progress(Xp(240)).level, level(1));
        assert_eq!(LevelCurve::GAME.progress(Xp(250)).level, level(2));
        assert_eq!(LevelCurve::GAME.progress(Xp(250)).xp_for_next, Xp(500));
        assert_eq!(LevelCurve::GAME.xp_to_next(level(10)), Xp(2_500));
        assert_eq!(LevelCurve::GAME.xp_to_next(level(50)), Xp(2_500));
        assert_eq!(LevelCurve::GAME.xp_to_reach(level(30)), Xp(61_250));
    }

    #[test]
    fn network_level_ten_needs_45000_xp() {
        assert_eq!(LevelCurve::NETWORK.xp_to_reach(level(10)), Xp(45_000));
        assert_eq!(LevelCurve::NETWORK.progress(Xp(44_999)).level, level(9));
        assert_eq!(LevelCurve::NETWORK.progress(Xp(45_000)).level, level(10));
    }

    #[test]
    fn huge_xp_saturates_instead_of_overflowing() {
        let progress = LevelCurve::GAME.progress(Xp(u64::MAX));
        assert_eq!(progress.level.get(), u32::MAX);
        assert!(progress.xp_into_level < progress.xp_for_next);
    }

    fn curves() -> impl Strategy<Value = LevelCurve> {
        prop_oneof![Just(LevelCurve::GAME), Just(LevelCurve::NETWORK)]
    }

    proptest! {
        #[test]
        fn level_never_decreases_when_xp_grows(curve in curves(), xp in 0u64..10_000_000, gain in 0u64..100_000) {
            let before = curve.progress(Xp(xp)).level;
            let after = curve.progress(Xp(xp + gain)).level;
            prop_assert!(after >= before, "xp={xp} gain={gain} before={before} after={after}");
        }

        #[test]
        fn progress_inside_a_level_is_below_the_next_threshold(curve in curves(), xp in any::<u64>()) {
            let progress = curve.progress(Xp(xp));
            prop_assert!(progress.xp_into_level < progress.xp_for_next);
            prop_assert_eq!(progress.xp_for_next, curve.xp_to_next(progress.level));
        }

        #[test]
        fn progress_and_threshold_agree(curve in curves(), xp in 0u64..1_000_000_000) {
            let progress = curve.progress(Xp(xp));
            let floor = curve.xp_to_reach(progress.level).get();
            prop_assert_eq!(floor + progress.xp_into_level.get(), xp);
        }

        #[test]
        fn reaching_a_threshold_gives_that_level(curve in curves(), n in 1u32..100_000) {
            let target = level(n);
            prop_assert_eq!(curve.progress(curve.xp_to_reach(target)).level, target);
        }
    }
}
