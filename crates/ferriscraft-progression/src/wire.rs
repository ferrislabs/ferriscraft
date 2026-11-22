use ferriscraft_protocol as protocol;
use uuid::Uuid;

use crate::level::LevelProgress;
use crate::player::{MatchCredit, PlayerProgress};

impl From<LevelProgress> for protocol::LevelProgress {
    fn from(progress: LevelProgress) -> Self {
        Self {
            level: progress.level.get(),
            xp_into_level: progress.xp_into_level.get(),
            xp_for_next: progress.xp_for_next.get(),
            total_xp: progress.total.get(),
        }
    }
}

impl MatchCredit {
    #[must_use]
    pub fn to_player_reward(&self, uuid: Uuid) -> protocol::PlayerReward {
        protocol::PlayerReward {
            uuid,
            xp: self.reward.xp.get(),
            coins: self.reward.coins.get(),
            game: self.game_after.into(),
            game_level_up: self.game_level_up(),
            network: self.network_after.into(),
            network_level_up: self.network_level_up(),
        }
    }
}

impl From<&PlayerProgress> for protocol::PlayerProfile {
    fn from(player: &PlayerProgress) -> Self {
        let record = player.record();
        Self {
            uuid: record.uuid,
            name: record.name.clone(),
            rank: record.rank,
            coins: record.coins.get(),
            network: player.network_progress().into(),
            games: record
                .games
                .iter()
                .map(|(&game, stats)| protocol::GameStats {
                    game,
                    progress: player.game_progress(game).into(),
                    played: stats.played,
                    wins: stats.wins,
                    losses: stats.losses,
                    draws: stats.draws,
                })
                .collect(),
            unlocked: player.unlocked().collect(),
            equipped: player.equipped().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ferriscraft_protocol::{CosmeticId, CosmeticSlot, GameKind, MatchOutcome, PlayerRank};

    use super::*;

    #[test]
    fn unknown_player_has_an_empty_profile_with_free_cosmetics() {
        let profile = protocol::PlayerProfile::from(&PlayerProgress::new(Uuid::nil()));
        assert_eq!(profile.coins, 0);
        assert_eq!(profile.network.level, 1);
        assert!(profile.games.is_empty());
        assert_eq!(
            profile.unlocked,
            [
                CosmeticId::GadgetFeatherDash,
                CosmeticId::GadgetFireworkRocket
            ]
        );
        assert!(profile.equipped.is_empty());
    }

    #[test]
    fn profile_reports_stats_rank_cosmetics_and_equipment() {
        let mut player = PlayerProgress::new(Uuid::nil());
        player.set_identity("Alex", PlayerRank::Vip);
        let _ = player.record_match(Uuid::nil(), GameKind::SheepWars, MatchOutcome::Win);
        player
            .equip(CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald))
            .unwrap();

        let profile = protocol::PlayerProfile::from(&player);
        assert_eq!(profile.name, "Alex");
        assert_eq!(profile.coins, 45);
        assert_eq!(profile.network.total_xp, 150);
        assert_eq!(profile.games.len(), 1);
        assert_eq!(profile.games[0].wins, 1);
        assert_eq!(profile.games[0].progress.xp_for_next, 250);
        assert!(profile.unlocked.contains(&CosmeticId::TrailEmerald));
        assert_eq!(
            profile.equipped.get(&CosmeticSlot::LobbyTrail),
            Some(&CosmeticId::TrailEmerald)
        );
    }

    #[test]
    fn reward_carries_new_levels_and_level_up_flags() {
        let mut player = PlayerProgress::new(Uuid::nil());
        let _ = player.record_match(Uuid::nil(), GameKind::SheepWars, MatchOutcome::Win);
        let credit = player.record_match(Uuid::nil(), GameKind::SheepWars, MatchOutcome::Win);
        let reward = credit.to_player_reward(Uuid::nil());
        assert_eq!((reward.xp, reward.coins), (150, 30));
        assert_eq!(reward.game.level, 2);
        assert!(reward.game_level_up);
        assert!(!reward.network_level_up);
    }
}
