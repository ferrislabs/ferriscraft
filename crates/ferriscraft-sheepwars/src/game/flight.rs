use rspigot_plugin::Material;

use crate::domain::{PlayerId, SheepKind, TeamId};
use crate::sheep::{display_name, sheep_wool};

pub const MAX_CONCURRENT_FLIGHTS: usize = 32;
pub const MAX_FLIGHT_TICKS: u32 = 20 * 15;
pub const LANDING_GRACE_TICKS: u32 = 4;
pub const SHEEP_FUSE_TICKS: u32 = 20 * 3;
pub const EYE_HEIGHT: f64 = 1.62;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flight {
    pub kind: SheepKind,
    pub team: TeamId,
    pub thrower: PlayerId,
    pub age: u32,
}

pub const fn waits_for_ground(kind: SheepKind) -> bool {
    matches!(kind, SheepKind::Healing)
}

pub const fn arms_on_impact(kind: SheepKind) -> bool {
    !matches!(kind, SheepKind::Healing | SheepKind::Boarding)
}

pub fn fuse_label(kind: SheepKind, remaining_ticks: u32) -> String {
    let secs = f64::from(remaining_ticks) / 20.0;
    let color = if remaining_ticks <= 20 { "§c" } else { "§e" };
    format!("{} {color}({secs:.1}s)", display_name(kind))
}

impl Flight {
    pub const fn new(kind: SheepKind, team: TeamId, thrower: PlayerId) -> Self {
        Self {
            kind,
            team,
            thrower,
            age: 0,
        }
    }

    pub const fn should_land(&self, on_ground: bool, wall_hit: bool) -> bool {
        let hit = on_ground || (wall_hit && !waits_for_ground(self.kind));
        (hit && self.age >= LANDING_GRACE_TICKS) || self.age >= MAX_FLIGHT_TICKS
    }
}

pub fn kind_of(material: Material) -> Option<SheepKind> {
    SheepKind::ALL
        .into_iter()
        .find(|kind| sheep_wool(*kind) == material)
}

pub const fn can_launch(active: usize) -> bool {
    active < MAX_CONCURRENT_FLIGHTS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight(kind: SheepKind, age: u32) -> Flight {
        Flight {
            kind,
            team: TeamId::Red,
            thrower: PlayerId(1),
            age,
        }
    }

    #[test]
    fn every_sheep_item_maps_back_to_its_kind() {
        for kind in SheepKind::ALL {
            assert_eq!(kind_of(sheep_wool(kind)), Some(kind));
        }
    }

    #[test]
    fn other_materials_are_not_sheep() {
        assert_eq!(kind_of(Material::Compass), None);
    }

    #[test]
    fn lands_on_the_ground_after_the_grace() {
        assert!(!flight(SheepKind::Lightning, 0).should_land(true, false));
        assert!(!flight(SheepKind::Lightning, LANDING_GRACE_TICKS - 1).should_land(true, false));
        assert!(flight(SheepKind::Lightning, LANDING_GRACE_TICKS).should_land(true, false));
        assert!(!flight(SheepKind::Lightning, LANDING_GRACE_TICKS).should_land(false, false));
    }

    #[test]
    fn wall_impact_detonates_offensive_sheep_but_healing_waits_for_ground() {
        assert!(flight(SheepKind::Incendiary, LANDING_GRACE_TICKS).should_land(false, true));
        assert!(flight(SheepKind::Boarding, LANDING_GRACE_TICKS).should_land(false, true));
        assert!(!flight(SheepKind::Healing, LANDING_GRACE_TICKS).should_land(false, true));
        assert!(flight(SheepKind::Healing, LANDING_GRACE_TICKS).should_land(true, true));
    }

    #[test]
    fn lands_when_the_lifetime_is_over() {
        assert!(flight(SheepKind::Lightning, MAX_FLIGHT_TICKS).should_land(false, false));
    }

    #[test]
    fn flights_are_capped() {
        assert!(can_launch(MAX_CONCURRENT_FLIGHTS - 1));
        assert!(!can_launch(MAX_CONCURRENT_FLIGHTS));
    }

    #[test]
    fn offensive_sheep_arm_on_impact_while_healing_and_boarding_do_not() {
        assert!(!arms_on_impact(SheepKind::Healing));
        assert!(!arms_on_impact(SheepKind::Boarding));
        assert!(arms_on_impact(SheepKind::Incendiary));
        assert!(arms_on_impact(SheepKind::Quake));
        assert!(arms_on_impact(SheepKind::Warp));
        assert!(arms_on_impact(SheepKind::Seeker));
    }

    #[test]
    fn fuse_label_formats_seconds_and_turns_red_under_one_second() {
        assert!(fuse_label(SheepKind::Incendiary, 60).contains("§e(3.0s)"));
        assert!(fuse_label(SheepKind::Incendiary, 20).contains("§c(1.0s)"));
    }
}
