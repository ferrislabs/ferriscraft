use std::collections::HashMap;

use crate::domain::PlayerId;

pub const LANDING_PROTECTION_TICKS: u64 = 20;
pub const MIN_AIRBORNE_TICKS: u64 = 4;
pub const MAX_AIRBORNE_TICKS: u64 = 20 * 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guard {
    Airborne { since: u64 },
    Landed { until: u64 },
}

#[derive(Debug, Default)]
pub struct FallGuards {
    entries: HashMap<PlayerId, Guard>,
}

impl FallGuards {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn board(&mut self, player: PlayerId, now: u64) {
        self.entries.insert(player, Guard::Airborne { since: now });
    }

    pub fn observe(&mut self, player: PlayerId, now: u64, on_ground: bool, dead: bool) {
        let Some(guard) = self.entries.get(&player).copied() else {
            return;
        };
        if dead {
            self.entries.remove(&player);
            return;
        }
        if let Guard::Airborne { since } = guard {
            let age = now.saturating_sub(since);
            if (on_ground && age >= MIN_AIRBORNE_TICKS) || age >= MAX_AIRBORNE_TICKS {
                self.entries.insert(
                    player,
                    Guard::Landed {
                        until: now + LANDING_PROTECTION_TICKS,
                    },
                );
            }
        }
    }

    pub fn protects(&self, player: PlayerId, now: u64) -> bool {
        match self.entries.get(&player) {
            Some(Guard::Airborne { .. }) => true,
            Some(Guard::Landed { until }) => now <= *until,
            None => false,
        }
    }

    pub fn airborne(&self) -> Vec<PlayerId> {
        self.entries
            .iter()
            .filter(|(_, guard)| matches!(guard, Guard::Airborne { .. }))
            .map(|(player, _)| *player)
            .collect()
    }

    pub fn prune(&mut self, now: u64) {
        self.entries.retain(|_, guard| match guard {
            Guard::Airborne { .. } => true,
            Guard::Landed { until } => now <= *until,
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: PlayerId = PlayerId(1);
    const BOB: PlayerId = PlayerId(2);

    #[test]
    fn an_unknown_player_is_never_protected() {
        assert!(!FallGuards::new().protects(ALICE, 0));
    }

    #[test]
    fn a_boarded_player_stays_protected_for_as_long_as_they_are_airborne() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 100);
        for now in [100, 150, 400, 600] {
            guards.observe(ALICE, now, false, false);
            assert!(guards.protects(ALICE, now), "tick {now}");
        }
    }

    #[test]
    fn protection_ends_one_second_after_touching_the_ground() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 100);
        guards.observe(ALICE, 140, true, false);
        assert!(guards.protects(ALICE, 140));
        assert!(guards.protects(ALICE, 140 + LANDING_PROTECTION_TICKS));
        assert!(!guards.protects(ALICE, 140 + LANDING_PROTECTION_TICKS + 1));
        assert_eq!(LANDING_PROTECTION_TICKS, 20);
    }

    #[test]
    fn touching_the_ground_right_after_boarding_does_not_count_as_landing() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 100);
        guards.observe(ALICE, 100 + MIN_AIRBORNE_TICKS - 1, true, false);
        assert!(guards.protects(ALICE, 100 + 300));
        assert_eq!(guards.airborne(), vec![ALICE]);
        guards.observe(ALICE, 100 + MIN_AIRBORNE_TICKS, true, false);
        assert!(guards.airborne().is_empty());
    }

    #[test]
    fn a_dead_player_loses_the_protection() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 0);
        guards.observe(ALICE, 10, false, true);
        assert!(!guards.protects(ALICE, 10));
    }

    #[test]
    fn airborne_protection_is_capped() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 0);
        guards.observe(ALICE, MAX_AIRBORNE_TICKS, false, false);
        assert!(guards.protects(ALICE, MAX_AIRBORNE_TICKS));
        assert!(!guards.protects(ALICE, MAX_AIRBORNE_TICKS + LANDING_PROTECTION_TICKS + 1));
    }

    #[test]
    fn guards_are_tracked_per_player_and_can_be_cleared() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 0);
        assert!(guards.protects(ALICE, 5));
        assert!(!guards.protects(BOB, 5));
        guards.clear();
        assert!(!guards.protects(ALICE, 5));
    }

    #[test]
    fn prune_drops_expired_landings_only() {
        let mut guards = FallGuards::new();
        guards.board(ALICE, 0);
        guards.board(BOB, 0);
        guards.observe(BOB, 10, true, false);
        guards.prune(10 + LANDING_PROTECTION_TICKS + 1);
        assert!(guards.protects(ALICE, 500));
        assert!(!guards.protects(BOB, 500));
    }
}
