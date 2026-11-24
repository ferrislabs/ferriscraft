use std::collections::{BTreeMap, BTreeSet};

use ferriscraft_protocol::{CosmeticId, CosmeticSlot, GameKind, MatchOutcome, PlayerRank};
use uuid::Uuid;

use crate::amount::{Coins, Xp};
use crate::catalog::{Catalog, CatalogEntry, Unlock, rank_satisfies};
use crate::ledger::{CoinDelta, CoinMovement, CoinReason};
use crate::level::{Level, LevelCurve, LevelProgress};
use crate::reward::{Reward, reward};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GameRecord {
    pub xp: Xp,
    pub played: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

impl GameRecord {
    fn record(&mut self, outcome: MatchOutcome, xp: Xp) {
        self.xp = self.xp.saturating_add(xp);
        self.played = self.played.saturating_add(1);
        let counter = match outcome {
            MatchOutcome::Win => &mut self.wins,
            MatchOutcome::Loss => &mut self.losses,
            MatchOutcome::Draw => &mut self.draws,
        };
        *counter = counter.saturating_add(1);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRecord {
    pub uuid: Uuid,
    pub name: String,
    pub rank: PlayerRank,
    pub coins: Coins,
    pub games: BTreeMap<GameKind, GameRecord>,
    pub purchased: BTreeSet<CosmeticId>,
    pub equipped: BTreeMap<CosmeticSlot, CosmeticId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnlockError {
    #[error("already unlocked")]
    AlreadyUnlocked,
    #[error("{missing} missing")]
    InsufficientCoins { missing: Coins },
    #[error("rank {0:?} required")]
    RankRequired(PlayerRank),
    #[error("network level {0} required")]
    LevelRequired(Level),
    #[error("unknown cosmetic")]
    UnknownCosmetic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EquipError {
    #[error("not unlocked")]
    NotUnlocked { requires: Unlock },
    #[error("this cosmetic does not fit this slot")]
    WrongSlot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCredit {
    pub game: GameKind,
    pub reward: Reward,
    pub game_before: LevelProgress,
    pub game_after: LevelProgress,
    pub network_before: LevelProgress,
    pub network_after: LevelProgress,
    pub movement: CoinMovement,
}

impl MatchCredit {
    #[must_use]
    pub fn game_level_up(&self) -> bool {
        self.game_after.level > self.game_before.level
    }

    #[must_use]
    pub fn network_level_up(&self) -> bool {
        self.network_after.level > self.network_before.level
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerProgress {
    record: PlayerRecord,
}

impl PlayerProgress {
    #[must_use]
    pub fn new(uuid: Uuid) -> Self {
        Self {
            record: PlayerRecord {
                uuid,
                name: String::new(),
                rank: PlayerRank::Guest,
                coins: Coins::ZERO,
                games: BTreeMap::new(),
                purchased: BTreeSet::new(),
                equipped: BTreeMap::new(),
            },
        }
    }

    #[must_use]
    pub fn record(&self) -> &PlayerRecord {
        &self.record
    }

    #[must_use]
    pub fn into_record(self) -> PlayerRecord {
        self.record
    }

    #[must_use]
    pub fn uuid(&self) -> Uuid {
        self.record.uuid
    }

    #[must_use]
    pub fn rank(&self) -> PlayerRank {
        self.record.rank
    }

    #[must_use]
    pub fn coins(&self) -> Coins {
        self.record.coins
    }

    pub fn set_identity(&mut self, name: &str, rank: PlayerRank) {
        if self.record.name != name {
            name.clone_into(&mut self.record.name);
        }
        self.record.rank = rank;
    }

    #[must_use]
    pub fn network_xp(&self) -> Xp {
        self.record
            .games
            .values()
            .fold(Xp::ZERO, |total, game| total.saturating_add(game.xp))
    }

    #[must_use]
    pub fn network_progress(&self) -> LevelProgress {
        LevelCurve::NETWORK.progress(self.network_xp())
    }

    #[must_use]
    pub fn game_progress(&self, game: GameKind) -> LevelProgress {
        let xp = self
            .record
            .games
            .get(&game)
            .map_or(Xp::ZERO, |record| record.xp);
        LevelCurve::GAME.progress(xp)
    }

    pub fn record_match(
        &mut self,
        match_id: Uuid,
        game: GameKind,
        outcome: MatchOutcome,
    ) -> MatchCredit {
        let reward = reward(outcome, self.record.rank);
        let game_before = self.game_progress(game);
        let network_before = self.network_progress();
        self.record
            .games
            .entry(game)
            .or_default()
            .record(outcome, reward.xp);
        self.record.coins = self.record.coins.saturating_add(reward.coins);
        MatchCredit {
            game,
            reward,
            game_before,
            game_after: self.game_progress(game),
            network_before,
            network_after: self.network_progress(),
            movement: CoinMovement {
                delta: CoinDelta::Credit(reward.coins),
                reason: CoinReason::Match(match_id),
            },
        }
    }

    pub fn grant(&mut self, amount: Coins, admin: &str) -> CoinMovement {
        self.record.coins = self.record.coins.saturating_add(amount);
        CoinMovement {
            delta: CoinDelta::Credit(amount),
            reason: CoinReason::Admin(admin.to_owned()),
        }
    }

    #[must_use]
    pub fn missing_requirement(&self, entry: &CatalogEntry) -> Option<Unlock> {
        let unlocked = match entry.unlock {
            Unlock::Free => true,
            Unlock::Price(_) => self.record.purchased.contains(&entry.id),
            Unlock::Rank(required) => rank_satisfies(self.record.rank, required),
            Unlock::NetworkLevel(required) => self.network_progress().level >= required,
        };
        (!unlocked).then_some(entry.unlock)
    }

    #[must_use]
    pub fn is_unlocked(&self, id: CosmeticId) -> bool {
        Catalog::entry(id).is_some_and(|entry| self.missing_requirement(&entry).is_none())
    }

    pub fn unlocked(&self) -> impl Iterator<Item = CosmeticId> + '_ {
        Catalog::ALL.into_iter().filter(|&id| self.is_unlocked(id))
    }

    pub fn equipped(&self) -> impl Iterator<Item = (CosmeticSlot, CosmeticId)> + '_ {
        self.record
            .equipped
            .iter()
            .map(|(&slot, &id)| (slot, id))
            .filter(|&(slot, id)| {
                Catalog::entry(id).is_some_and(|entry| entry.slot == slot) && self.is_unlocked(id)
            })
    }

    pub fn purchase(&mut self, id: CosmeticId) -> Result<CoinMovement, UnlockError> {
        let entry = Catalog::entry(id).ok_or(UnlockError::UnknownCosmetic)?;
        let price = match self.missing_requirement(&entry) {
            None => return Err(UnlockError::AlreadyUnlocked),
            Some(Unlock::Free) => return Err(UnlockError::AlreadyUnlocked),
            Some(Unlock::Rank(rank)) => return Err(UnlockError::RankRequired(rank)),
            Some(Unlock::NetworkLevel(level)) => return Err(UnlockError::LevelRequired(level)),
            Some(Unlock::Price(price)) => price,
        };
        self.record.coins = self.record.coins.checked_sub(price).map_err(|short| {
            UnlockError::InsufficientCoins {
                missing: short.missing,
            }
        })?;
        self.record.purchased.insert(id);
        Ok(CoinMovement {
            delta: CoinDelta::Debit(price),
            reason: CoinReason::Cosmetic(id),
        })
    }

    pub fn equip(&mut self, slot: CosmeticSlot, id: Option<CosmeticId>) -> Result<(), EquipError> {
        let Some(id) = id else {
            self.record.equipped.remove(&slot);
            return Ok(());
        };
        let entry = Catalog::entry(id)
            .filter(|entry| entry.slot == slot)
            .ok_or(EquipError::WrongSlot)?;
        if let Some(requires) = self.missing_requirement(&entry) {
            return Err(EquipError::NotUnlocked { requires });
        }
        self.record.equipped.insert(slot, id);
        Ok(())
    }
}

impl From<PlayerRecord> for PlayerProgress {
    fn from(record: PlayerRecord) -> Self {
        Self { record }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const RED: Uuid = Uuid::from_u128(1);
    const BLUE: Uuid = Uuid::from_u128(2);
    const MATCH: Uuid = Uuid::from_u128(99);

    fn player(rank: PlayerRank, coins: u64) -> PlayerProgress {
        let mut progress = PlayerProgress::new(RED);
        progress.set_identity("Alex", rank);
        progress.record.coins = Coins(coins);
        progress
    }

    fn with_sheepwars_xp(progress: &mut PlayerProgress, xp: u64) {
        progress.record.games.insert(
            GameKind::SheepWars,
            GameRecord {
                xp: Xp(xp),
                ..GameRecord::default()
            },
        );
    }

    #[test]
    fn spec_prg_1_win_and_loss_credit_game_and_network_xp() {
        let mut a = PlayerProgress::new(RED);
        let mut b = PlayerProgress::new(BLUE);
        let win = a.record_match(MATCH, GameKind::SheepWars, MatchOutcome::Win);
        let loss = b.record_match(MATCH, GameKind::SheepWars, MatchOutcome::Loss);

        assert_eq!(a.game_progress(GameKind::SheepWars).total, Xp(150));
        assert_eq!(a.network_xp(), Xp(150));
        assert_eq!(a.coins(), Coins(30));
        assert_eq!(b.game_progress(GameKind::SheepWars).total, Xp(50));
        assert_eq!(b.network_xp(), Xp(50));
        assert_eq!(b.coins(), Coins(10));
        assert_eq!(
            win.movement,
            CoinMovement {
                delta: CoinDelta::Credit(Coins(30)),
                reason: CoinReason::Match(MATCH)
            }
        );
        assert_eq!(loss.reward.xp, Xp(50));

        let stats = a.record().games[&GameKind::SheepWars];
        assert_eq!((stats.played, stats.wins, stats.losses), (1, 1, 0));
    }

    #[test]
    fn a_towerwars_win_earns_xp_on_its_own_game_curve_and_the_network_curve() {
        let mut a = PlayerProgress::new(RED);
        let credit = a.record_match(MATCH, GameKind::TowerWars, MatchOutcome::Win);

        assert_eq!(credit.game, GameKind::TowerWars);
        assert_eq!(a.game_progress(GameKind::TowerWars).total, Xp(150));
        assert_eq!(a.game_progress(GameKind::SheepWars).total, Xp::ZERO);
        assert_eq!(a.network_xp(), Xp(150));
        assert_eq!(a.coins(), Coins(30));

        let _ = a.record_match(MATCH, GameKind::SheepWars, MatchOutcome::Loss);
        assert_eq!(a.network_xp(), Xp(200));
        let stats = a.record().games[&GameKind::TowerWars];
        assert_eq!((stats.played, stats.wins, stats.losses), (1, 1, 0));
    }

    #[test]
    fn spec_prg_6_crossing_a_threshold_reports_a_level_up() {
        let mut a = player(PlayerRank::Member, 0);
        with_sheepwars_xp(&mut a, 240);
        let credit = a.record_match(MATCH, GameKind::SheepWars, MatchOutcome::Loss);
        assert_eq!(credit.game_after.level.get(), 2);
        assert!(credit.game_level_up());
        assert!(!credit.network_level_up());
    }

    #[test]
    fn spec_prg_7_purchase_debits_and_unlocks() {
        let mut a = player(PlayerRank::Member, 600);
        let movement = a.purchase(CosmeticId::SheepTrailFlame).unwrap();
        assert_eq!(a.coins(), Coins(100));
        assert!(a.is_unlocked(CosmeticId::SheepTrailFlame));
        assert_eq!(
            movement,
            CoinMovement {
                delta: CoinDelta::Debit(Coins(500)),
                reason: CoinReason::Cosmetic(CosmeticId::SheepTrailFlame)
            }
        );
    }

    #[test]
    fn spec_prg_8_purchase_without_enough_coins_changes_nothing() {
        let mut a = player(PlayerRank::Member, 400);
        assert_eq!(
            a.purchase(CosmeticId::SheepTrailFlame),
            Err(UnlockError::InsufficientCoins {
                missing: Coins(100)
            })
        );
        assert_eq!(a.coins(), Coins(400));
        assert!(!a.is_unlocked(CosmeticId::SheepTrailFlame));
    }

    #[test]
    fn spec_prg_9_second_purchase_cannot_overdraw() {
        let mut a = player(PlayerRank::Member, 600);
        assert!(a.purchase(CosmeticId::SheepTrailFlame).is_ok());
        assert_eq!(
            a.purchase(CosmeticId::SheepTrailHearts),
            Err(UnlockError::InsufficientCoins {
                missing: Coins(400)
            })
        );
        assert_eq!(a.coins(), Coins(100));
    }

    #[test]
    fn buying_twice_is_rejected() {
        let mut a = player(PlayerRank::Member, 2_000);
        assert!(a.purchase(CosmeticId::TrailFlame).is_ok());
        assert_eq!(
            a.purchase(CosmeticId::TrailFlame),
            Err(UnlockError::AlreadyUnlocked)
        );
        assert_eq!(a.coins(), Coins(1_250));
    }

    #[test]
    fn free_rank_and_level_cosmetics_cannot_be_bought() {
        let mut member = player(PlayerRank::Member, 10_000);
        assert_eq!(
            member.purchase(CosmeticId::GadgetFeatherDash),
            Err(UnlockError::AlreadyUnlocked)
        );
        assert_eq!(
            member.purchase(CosmeticId::TrailEmerald),
            Err(UnlockError::RankRequired(PlayerRank::Vip))
        );
        assert_eq!(
            member.purchase(CosmeticId::TrailMagic),
            Err(UnlockError::LevelRequired(Level::new(10).unwrap()))
        );
        assert_eq!(
            member.purchase(CosmeticId::Unknown),
            Err(UnlockError::UnknownCosmetic)
        );
        assert_eq!(member.coins(), Coins(10_000));
    }

    #[test]
    fn spec_prg_11_rank_cosmetic_needs_the_rank() {
        let mut vip = player(PlayerRank::Vip, 0);
        let mut member = player(PlayerRank::Member, 0);
        assert_eq!(
            vip.equip(CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald)),
            Ok(())
        );
        assert_eq!(
            member.equip(CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald)),
            Err(EquipError::NotUnlocked {
                requires: Unlock::Rank(PlayerRank::Vip)
            })
        );
    }

    #[test]
    fn a_rank_cosmetic_is_lost_with_the_rank() {
        let mut a = player(PlayerRank::Vip, 0);
        a.equip(CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald))
            .unwrap();
        a.set_identity("Alex", PlayerRank::Member);
        assert!(!a.is_unlocked(CosmeticId::TrailEmerald));
        assert_eq!(a.equipped().count(), 0);
    }

    #[test]
    fn a_purchased_cosmetic_is_kept_without_rank() {
        let mut a = player(PlayerRank::Vip, 500);
        let _ = a.purchase(CosmeticId::SheepTrailHearts).unwrap();
        a.set_identity("Alex", PlayerRank::Guest);
        assert!(a.is_unlocked(CosmeticId::SheepTrailHearts));
    }

    #[test]
    fn level_cosmetic_unlocks_at_network_level_ten() {
        let mut a = player(PlayerRank::Member, 0);
        with_sheepwars_xp(&mut a, 44_999);
        assert!(!a.is_unlocked(CosmeticId::TrailMagic));
        with_sheepwars_xp(&mut a, 45_000);
        assert!(a.is_unlocked(CosmeticId::TrailMagic));
    }

    #[test]
    fn equipping_in_the_wrong_slot_is_rejected() {
        let mut a = player(PlayerRank::Admin, 0);
        assert_eq!(
            a.equip(
                CosmeticSlot::SheepTrail,
                Some(CosmeticId::GadgetFeatherDash)
            ),
            Err(EquipError::WrongSlot)
        );
        assert_eq!(
            a.equip(CosmeticSlot::SheepTrail, Some(CosmeticId::Unknown)),
            Err(EquipError::WrongSlot)
        );
    }

    #[test]
    fn equipping_none_clears_the_slot() {
        let mut a = player(PlayerRank::Member, 0);
        a.equip(
            CosmeticSlot::LobbyGadget,
            Some(CosmeticId::GadgetFeatherDash),
        )
        .unwrap();
        a.equip(CosmeticSlot::LobbyGadget, None).unwrap();
        assert_eq!(a.equipped().count(), 0);
    }

    #[test]
    fn spec_prg_17_admin_grant_is_logged_with_the_admin_name() {
        let mut a = player(PlayerRank::Member, 0);
        let movement = a.grant(Coins(200), "X");
        assert_eq!(a.coins(), Coins(200));
        assert_eq!(
            movement,
            CoinMovement {
                delta: CoinDelta::Credit(Coins(200)),
                reason: CoinReason::Admin("X".into())
            }
        );
    }

    #[derive(Debug, Clone)]
    enum Op {
        Match(MatchOutcome),
        Buy(usize),
        Equip(usize),
        Grant(u64),
        Rank(PlayerRank),
    }

    fn op() -> impl Strategy<Value = Op> {
        let outcome = prop_oneof![
            Just(MatchOutcome::Win),
            Just(MatchOutcome::Loss),
            Just(MatchOutcome::Draw)
        ];
        let rank = prop_oneof![
            Just(PlayerRank::Admin),
            Just(PlayerRank::Vip),
            Just(PlayerRank::Member),
            Just(PlayerRank::Guest),
        ];
        prop_oneof![
            outcome.prop_map(Op::Match),
            (0..Catalog::ALL.len()).prop_map(Op::Buy),
            (0..Catalog::ALL.len()).prop_map(Op::Equip),
            (0u64..5_000).prop_map(Op::Grant),
            rank.prop_map(Op::Rank),
        ]
    }

    proptest! {
        #[test]
        fn ledger_always_sums_to_the_balance(ops in prop::collection::vec(op(), 0..200)) {
            let mut a = PlayerProgress::new(RED);
            let mut ledger = Vec::new();
            for op in &ops {
                match *op {
                    Op::Match(outcome) => {
                        ledger.push(a.record_match(MATCH, GameKind::SheepWars, outcome).movement);
                    }
                    Op::Buy(index) => {
                        if let Ok(movement) = a.purchase(Catalog::ALL[index]) {
                            ledger.push(movement);
                        }
                    }
                    Op::Equip(index) => {
                        let id = Catalog::ALL[index];
                        let slot = Catalog::entry(id).unwrap().slot;
                        let _ = a.equip(slot, Some(id));
                    }
                    Op::Grant(amount) => ledger.push(a.grant(Coins(amount), "X")),
                    Op::Rank(rank) => a.set_identity("Alex", rank),
                }
                let sum: i128 = ledger.iter().map(|movement| movement.delta.signed()).sum();
                prop_assert_eq!(sum, i128::from(a.coins().get()));
                for (slot, id) in a.equipped() {
                    prop_assert!(a.is_unlocked(id));
                    prop_assert_eq!(Catalog::entry(id).unwrap().slot, slot);
                }
            }
        }

        #[test]
        fn network_xp_is_the_sum_of_game_xp(outcomes in prop::collection::vec(0u8..3, 0..100)) {
            let mut a = PlayerProgress::new(RED);
            for outcome in outcomes {
                let outcome = match outcome {
                    0 => MatchOutcome::Win,
                    1 => MatchOutcome::Loss,
                    _ => MatchOutcome::Draw,
                };
                let _ = a.record_match(MATCH, GameKind::SheepWars, outcome);
            }
            prop_assert_eq!(a.network_xp(), a.game_progress(GameKind::SheepWars).total);
        }
    }
}
