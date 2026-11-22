use ferriscraft_protocol::{MatchOutcome, PlayerRank};

use crate::amount::{Coins, Xp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reward {
    pub xp: Xp,
    pub coins: Coins,
}

#[must_use]
pub const fn xp_reward(outcome: MatchOutcome) -> Xp {
    match outcome {
        MatchOutcome::Win => Xp(150),
        MatchOutcome::Draw => Xp(75),
        MatchOutcome::Loss => Xp(50),
    }
}

#[must_use]
pub const fn base_coin_reward(outcome: MatchOutcome) -> Coins {
    match outcome {
        MatchOutcome::Win => Coins(30),
        MatchOutcome::Draw => Coins(15),
        MatchOutcome::Loss => Coins(10),
    }
}

#[must_use]
pub const fn has_coin_bonus(rank: PlayerRank) -> bool {
    matches!(
        rank,
        PlayerRank::Admin | PlayerRank::Mod | PlayerRank::Dev | PlayerRank::Vip
    )
}

#[must_use]
pub const fn coin_reward(outcome: MatchOutcome, rank: PlayerRank) -> Coins {
    let base = base_coin_reward(outcome);
    if has_coin_bonus(rank) {
        base.scale_floor(3, 2)
    } else {
        base
    }
}

#[must_use]
pub const fn reward(outcome: MatchOutcome, rank: PlayerRank) -> Reward {
    Reward {
        xp: xp_reward(outcome),
        coins: coin_reward(outcome, rank),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn spec_prg_1_win_and_loss_rewards() {
        assert_eq!(
            reward(MatchOutcome::Win, PlayerRank::Member),
            Reward {
                xp: Xp(150),
                coins: Coins(30)
            }
        );
        assert_eq!(
            reward(MatchOutcome::Loss, PlayerRank::Member),
            Reward {
                xp: Xp(50),
                coins: Coins(10)
            }
        );
        assert_eq!(
            reward(MatchOutcome::Draw, PlayerRank::Guest),
            Reward {
                xp: Xp(75),
                coins: Coins(15)
            }
        );
    }

    #[test]
    fn spec_prg_10_vip_bonus_applies_to_coins_only() {
        let vip = reward(MatchOutcome::Win, PlayerRank::Vip);
        let member = reward(MatchOutcome::Win, PlayerRank::Member);
        assert_eq!(vip.xp, Xp(150));
        assert_eq!(member.xp, Xp(150));
        assert_eq!(vip.coins, Coins(45));
        assert_eq!(member.coins, Coins(30));
    }

    #[test]
    fn bonus_rounds_down() {
        assert_eq!(coin_reward(MatchOutcome::Draw, PlayerRank::Vip), Coins(22));
        assert_eq!(
            coin_reward(MatchOutcome::Loss, PlayerRank::Admin),
            Coins(15)
        );
    }

    fn outcomes() -> impl Strategy<Value = MatchOutcome> {
        prop_oneof![
            Just(MatchOutcome::Win),
            Just(MatchOutcome::Loss),
            Just(MatchOutcome::Draw)
        ]
    }

    fn ranks() -> impl Strategy<Value = PlayerRank> {
        prop_oneof![
            Just(PlayerRank::Admin),
            Just(PlayerRank::Mod),
            Just(PlayerRank::Dev),
            Just(PlayerRank::Vip),
            Just(PlayerRank::Member),
            Just(PlayerRank::Guest),
        ]
    }

    proptest! {
        #[test]
        fn rank_never_changes_xp(outcome in outcomes(), a in ranks(), b in ranks()) {
            prop_assert_eq!(reward(outcome, a).xp, reward(outcome, b).xp);
        }

        #[test]
        fn a_rank_never_earns_fewer_coins_than_base(outcome in outcomes(), rank in ranks()) {
            prop_assert!(coin_reward(outcome, rank) >= base_coin_reward(outcome));
        }
    }
}
