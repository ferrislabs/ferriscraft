use ferriscraft_protocol::{CosmeticId, CosmeticSlot, PlayerRank};

use crate::amount::Coins;
use crate::level::Level;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlock {
    Free,
    Price(Coins),
    Rank(PlayerRank),
    NetworkLevel(Level),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: CosmeticId,
    pub slot: CosmeticSlot,
    pub unlock: Unlock,
}

#[must_use]
pub fn rank_satisfies(rank: PlayerRank, required: PlayerRank) -> bool {
    rank <= required
}

const MAGIC_TRAIL_LEVEL: Level = Level::new(10).expect("10 is not zero");

pub struct Catalog;

impl Catalog {
    pub const ALL: [CosmeticId; 16] = [
        CosmeticId::GadgetFeatherDash,
        CosmeticId::GadgetFireworkRocket,
        CosmeticId::GadgetWindCharge,
        CosmeticId::GadgetSheepCannon,
        CosmeticId::GadgetVortexWand,
        CosmeticId::GadgetEnderGrapple,
        CosmeticId::TrailFlame,
        CosmeticId::TrailHeart,
        CosmeticId::TrailMagic,
        CosmeticId::TrailEmerald,
        CosmeticId::SheepTrailFlame,
        CosmeticId::SheepTrailHearts,
        CosmeticId::SheepTrailNotes,
        CosmeticId::SheepTrailRainbow,
        CosmeticId::VictoryFireworks,
        CosmeticId::VictoryLightning,
    ];

    #[must_use]
    pub const fn entry(id: CosmeticId) -> Option<CatalogEntry> {
        use CosmeticSlot::{LobbyGadget, LobbyTrail, SheepTrail, VictoryEffect};
        let (slot, unlock) = match id {
            CosmeticId::GadgetFeatherDash => (LobbyGadget, Unlock::Free),
            CosmeticId::GadgetFireworkRocket => (LobbyGadget, Unlock::Free),
            CosmeticId::GadgetWindCharge => (LobbyGadget, Unlock::Price(Coins(500))),
            CosmeticId::GadgetSheepCannon => (LobbyGadget, Unlock::Price(Coins(1_000))),
            CosmeticId::GadgetVortexWand => (LobbyGadget, Unlock::Price(Coins(1_500))),
            CosmeticId::GadgetEnderGrapple => (LobbyGadget, Unlock::Rank(PlayerRank::Vip)),
            CosmeticId::TrailFlame => (LobbyTrail, Unlock::Price(Coins(750))),
            CosmeticId::TrailHeart => (LobbyTrail, Unlock::Price(Coins(750))),
            CosmeticId::TrailMagic => (LobbyTrail, Unlock::NetworkLevel(MAGIC_TRAIL_LEVEL)),
            CosmeticId::TrailEmerald => (LobbyTrail, Unlock::Rank(PlayerRank::Vip)),
            CosmeticId::SheepTrailFlame => (SheepTrail, Unlock::Price(Coins(500))),
            CosmeticId::SheepTrailHearts => (SheepTrail, Unlock::Price(Coins(500))),
            CosmeticId::SheepTrailNotes => (SheepTrail, Unlock::Price(Coins(800))),
            CosmeticId::SheepTrailRainbow => (SheepTrail, Unlock::Rank(PlayerRank::Vip)),
            CosmeticId::VictoryFireworks => (VictoryEffect, Unlock::Price(Coins(1_000))),
            CosmeticId::VictoryLightning => (VictoryEffect, Unlock::Price(Coins(1_500))),
            CosmeticId::Unknown => return None,
        };
        Some(CatalogEntry { id, slot, unlock })
    }

    pub fn in_slot(slot: CosmeticSlot) -> impl Iterator<Item = CatalogEntry> {
        Self::ALL
            .into_iter()
            .filter_map(Self::entry)
            .filter(move |entry| entry.slot == slot)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_listed_cosmetic_has_an_entry_and_appears_once() {
        let distinct: HashSet<_> = Catalog::ALL.into_iter().collect();
        assert_eq!(distinct.len(), Catalog::ALL.len());
        for id in Catalog::ALL {
            assert_eq!(Catalog::entry(id).map(|entry| entry.id), Some(id));
        }
    }

    #[test]
    fn unknown_cosmetic_has_no_entry() {
        assert_eq!(Catalog::entry(CosmeticId::Unknown), None);
    }

    #[test]
    fn exactly_two_gadgets_stay_free() {
        let free: Vec<_> = Catalog::in_slot(CosmeticSlot::LobbyGadget)
            .filter(|entry| entry.unlock == Unlock::Free)
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            free,
            [
                CosmeticId::GadgetFeatherDash,
                CosmeticId::GadgetFireworkRocket
            ]
        );
    }

    #[test]
    fn slots_hold_the_expected_number_of_cosmetics() {
        assert_eq!(Catalog::in_slot(CosmeticSlot::LobbyGadget).count(), 6);
        assert_eq!(Catalog::in_slot(CosmeticSlot::LobbyTrail).count(), 4);
        assert_eq!(Catalog::in_slot(CosmeticSlot::SheepTrail).count(), 4);
        assert_eq!(Catalog::in_slot(CosmeticSlot::VictoryEffect).count(), 2);
    }

    #[test]
    fn staff_ranks_satisfy_a_vip_requirement() {
        for rank in [
            PlayerRank::Admin,
            PlayerRank::Mod,
            PlayerRank::Dev,
            PlayerRank::Vip,
        ] {
            assert!(rank_satisfies(rank, PlayerRank::Vip), "{rank:?}");
        }
        assert!(!rank_satisfies(PlayerRank::Member, PlayerRank::Vip));
        assert!(!rank_satisfies(PlayerRank::Guest, PlayerRank::Vip));
    }
}
