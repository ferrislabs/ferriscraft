use super::launch::DEFAULT_THROW_POWER;
use crate::domain::SheepKind;

pub const INCENDIARY_POWER: f32 = 3.0;
pub const INCENDIARY_BREAKS_BLOCKS: bool = true;
pub const INCENDIARY_FIRE_RADIUS: f64 = 6.0;
pub const INCENDIARY_FIRE_TICKS: i32 = 120;
pub const INCENDIARY_IGNITE_BLOCKS: u32 = 26;
pub const INCENDIARY_FIRE_SPREAD: f64 = 5.5;
pub const INCENDIARY_DAMAGE: f32 = 4.0;
pub const INCENDIARY_DAMAGE_RADIUS: f64 = 5.0;

pub const EXPLOSIVE_POWER: f32 = 7.0;
pub const EXPLOSIVE_BREAKS_BLOCKS: bool = true;
pub const EXPLOSIVE_DAMAGE: f32 = 12.0;
pub const EXPLOSIVE_DAMAGE_RADIUS: f64 = 6.0;

pub const TORPEDO_SPEED: f64 = 1.1;
pub const TORPEDO_CLEAR_RADIUS: f64 = 3.0;
pub const TORPEDO_TRIGGER_RADIUS: f64 = 3.0;
pub const TORPEDO_POWER: f32 = 3.0;
pub const TORPEDO_DAMAGE: f32 = 16.0;
pub const TORPEDO_DAMAGE_RADIUS: f64 = 5.0;
pub const TORPEDO_MAX_FLIGHT_TICKS: u32 = 20 * 6;
pub const TORPEDO_CLEAR_DELAY_TICKS: u32 = 2;

pub const LIGHTNING_STRIKES: u32 = 3;
pub const LIGHTNING_SPREAD: f64 = 2.5;

pub const FROST_ZONE_RADIUS: f64 = 5.0;
pub const FROST_ZONE_DURATION_TICKS: u32 = 160;
pub const FROST_PULSE_TICKS: u32 = 10;
pub const FROST_SLOWNESS_TICKS: i32 = 30;
pub const FROST_SLOWNESS_AMPLIFIER: u8 = 1;
pub const FROST_ICE_RING_TICKS: u32 = 3;
pub const FROST_ICE_LAYERS: [i32; 2] = [-1, -2];

pub const SEEKER_POWER: f32 = 3.5;
pub const SEEKER_BREAKS_BLOCKS: bool = true;
pub const SEEKER_SPEED: f64 = 1.25;
pub const SEEKER_RANGE: f64 = 64.0;
pub const SEEKER_DETONATE_DISTANCE: f64 = 1.5;
pub const SEEKER_RUN_SPEED: f64 = 0.32;
pub const SEEKER_FUSE_TICKS: u32 = 40;
pub const SEEKER_DAMAGE: f32 = 10.0;
pub const SEEKER_DAMAGE_RADIUS: f64 = 4.0;

pub const HEALING_RADIUS: f64 = 6.0;
pub const HEALING_REGENERATION_TICKS: i32 = 60;
pub const HEALING_REGENERATION_AMPLIFIER: u8 = 1;
pub const HEALING_DURATION_TICKS: u32 = 20 * 12;
pub const HEALING_PULSE_TICKS: u32 = 20;
pub const HEALING_PULSE_HEALTH: f32 = 2.0;
pub const HEALING_THROW_POWER: f64 = 1.1;
pub const BOARDING_THROW_POWER: f64 = 1.5;

pub const QUAKE_RADIUS: f64 = 7.0;
pub const QUAKE_LIFT: f64 = 1.25;
pub const QUAKE_PUSH: f64 = 0.85;
pub const QUAKE_SLOWNESS_TICKS: i32 = 60;
pub const QUAKE_SLOWNESS_AMPLIFIER: u8 = 1;
pub const QUAKE_PULSE_TICKS: u32 = 40;
pub const QUAKE_DURATION_TICKS: u32 = 200;
pub const QUAKE_PULSE_LIFT: f64 = 0.84;

pub const DARKNESS_RADIUS: f64 = 8.0;
pub const DARKNESS_BLINDNESS_TICKS: i32 = 160;
pub const DARKNESS_BLINDNESS_AMPLIFIER: u8 = 0;

pub const SPLIT_COUNT: u32 = 4;
pub const SPLIT_HORIZONTAL_SPEED: f64 = 0.85;
pub const SPLIT_VERTICAL_SPEED: f64 = 0.75;
pub const SPLIT_CHILD: SheepKind = SheepKind::Fragment;
pub const SPLIT_PARENT_POWER: f32 = 2.5;
pub const SPLIT_PARENT_DAMAGE: f32 = 6.0;
pub const SPLIT_PARENT_DAMAGE_RADIUS: f64 = 4.0;
pub const SPLIT_PARENT_BREAKS_BLOCKS: bool = true;

pub const FRAGMENT_POWER: f32 = 2.5;
pub const FRAGMENT_BREAKS_BLOCKS: bool = true;
pub const FRAGMENT_DAMAGE: f32 = 6.0;
pub const FRAGMENT_DAMAGE_RADIUS: f64 = 4.0;
pub const FRAGMENT_SCALE: f64 = 0.5;
pub const FRAGMENT_LANDING_GRACE_TICKS: u32 = 8;

pub const WARP_RADIUS: f64 = 8.0;
pub const WARP_PUSH: f64 = 2.0;
pub const WARP_LIFT: f64 = 0.85;
pub const WARP_BLINDNESS_TICKS: i32 = 40;
pub const WARP_BLINDNESS_AMPLIFIER: u8 = 0;
pub const WARP_DELAY_TICKS: u32 = 40;
pub const WARP_BLOCK_PAIRS: u32 = 12;
pub const WARP_BLOCK_RADIUS: f64 = 4.0;

pub const INTERGALACTIC_METEORS: u32 = 8;
pub const INTERGALACTIC_SPREAD: f64 = 8.0;
pub const INTERGALACTIC_POWER: f32 = 5.0;
pub const INTERGALACTIC_BREAKS_BLOCKS: bool = true;
pub const INTERGALACTIC_RADIUS: f64 = 8.0;
pub const INTERGALACTIC_CLEAR_HEIGHT: f64 = 48.0;
pub const INTERGALACTIC_SKY_HEIGHT: f64 = 60.0;
pub const INTERGALACTIC_FALL_TICKS: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WoolColor {
    White,
    Orange,
    LightBlue,
    Green,
    Yellow,
    Pink,
    Purple,
    Blue,
    Brown,
    Black,
    Gray,
    Red,
    Cyan,
}

pub const fn display_name(kind: SheepKind) -> &'static str {
    match kind {
        SheepKind::Lightning => "§eInvocateur de foudre",
        SheepKind::Incendiary => "§6Mouton incendiaire",
        SheepKind::Boarding => "§fMouton d'abordage",
        SheepKind::Healing => "§dMouton soigneur",
        SheepKind::Seeker => "§aMouton chercheur",
        SheepKind::Torpedo => "§3Mouton torpilleur",
        SheepKind::Explosive => "§cMouton explosif",
        SheepKind::Fragment => "§7Fragment",
        SheepKind::Splitting => "§7Mouton à fragmentation",
        SheepKind::Darkness => "§8Mouton ténébreux",
        SheepKind::Frozen => "§bMouton glacé",
        SheepKind::Quake => "§6Mouton tremblement de terre",
        SheepKind::Warp => "§5Mouton à distorsion",
        SheepKind::Intergalactic => "§9Mouton intergalactique",
    }
}

pub const fn wool_color(kind: SheepKind) -> WoolColor {
    match kind {
        SheepKind::Lightning => WoolColor::Yellow,
        SheepKind::Incendiary => WoolColor::Orange,
        SheepKind::Boarding => WoolColor::White,
        SheepKind::Healing => WoolColor::Pink,
        SheepKind::Seeker => WoolColor::Green,
        SheepKind::Torpedo => WoolColor::Cyan,
        SheepKind::Explosive => WoolColor::Red,
        SheepKind::Fragment => WoolColor::Gray,
        SheepKind::Splitting => WoolColor::Gray,
        SheepKind::Darkness => WoolColor::Black,
        SheepKind::Frozen => WoolColor::LightBlue,
        SheepKind::Quake => WoolColor::Brown,
        SheepKind::Warp => WoolColor::Purple,
        SheepKind::Intergalactic => WoolColor::Blue,
    }
}

pub const fn flies_straight(kind: SheepKind) -> bool {
    matches!(kind, SheepKind::Torpedo)
}

pub const fn chases_after_impact(kind: SheepKind) -> bool {
    matches!(kind, SheepKind::Seeker)
}

pub const fn drop_weight(kind: SheepKind) -> u32 {
    match kind {
        SheepKind::Seeker => 10,
        SheepKind::Explosive => 20,
        SheepKind::Incendiary | SheepKind::Boarding => 14,
        SheepKind::Torpedo | SheepKind::Frozen => 12,
        SheepKind::Healing => 10,
        SheepKind::Splitting | SheepKind::Darkness | SheepKind::Lightning => 8,
        SheepKind::Warp | SheepKind::Quake => 6,
        SheepKind::Intergalactic => 3,
        SheepKind::Fragment => 0,
    }
}

pub fn weighted_kind(roll: u32) -> SheepKind {
    let total: u32 = SheepKind::ALL.into_iter().map(drop_weight).sum();
    let mut pick = roll % total;
    for kind in SheepKind::ALL {
        let weight = drop_weight(kind);
        if pick < weight {
            return kind;
        }
        pick -= weight;
    }
    SheepKind::ALL[0]
}

pub const fn announces_launch(kind: SheepKind) -> bool {
    matches!(kind, SheepKind::Intergalactic)
}

pub const fn launch_power(kind: SheepKind) -> f64 {
    match kind {
        SheepKind::Healing => HEALING_THROW_POWER,
        SheepKind::Boarding => BOARDING_THROW_POWER,
        _ => DEFAULT_THROW_POWER,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_kind_has_a_distinct_wool_and_name() {
        let colors: HashSet<_> = SheepKind::ALL.into_iter().map(wool_color).collect();
        let names: HashSet<_> = SheepKind::ALL.into_iter().map(display_name).collect();
        assert_eq!(colors.len(), SheepKind::ALL.len());
        assert_eq!(names.len(), SheepKind::ALL.len());
    }

    #[test]
    fn only_the_torpedo_flies_straight_and_only_the_seeker_chases() {
        let straight: Vec<_> = SheepKind::ALL
            .into_iter()
            .filter(|kind| flies_straight(*kind))
            .collect();
        let chasing: Vec<_> = SheepKind::ALL
            .into_iter()
            .filter(|kind| chases_after_impact(*kind))
            .collect();
        assert_eq!(straight, vec![SheepKind::Torpedo]);
        assert_eq!(chasing, vec![SheepKind::Seeker]);
    }

    #[test]
    fn boarding_is_white_and_warp_is_purple() {
        assert_eq!(wool_color(SheepKind::Boarding), WoolColor::White);
        assert_eq!(wool_color(SheepKind::Warp), WoolColor::Purple);
    }

    #[test]
    fn weighted_kind_covers_every_sheep_and_follows_the_weights() {
        let total: u32 = SheepKind::ALL.into_iter().map(drop_weight).sum();
        let mut counts = std::collections::HashMap::new();
        for roll in 0..total {
            *counts.entry(weighted_kind(roll)).or_insert(0_u32) += 1;
        }
        for kind in SheepKind::ALL {
            assert_eq!(counts.get(&kind).copied(), Some(drop_weight(kind)));
        }
        assert_eq!(drop_weight(SheepKind::Seeker), 10);
        assert_eq!(drop_weight(SheepKind::Explosive), 20);
    }

    #[test]
    fn the_explosive_breaks_more_than_the_incendiary() {
        const {
            assert!(EXPLOSIVE_POWER > INCENDIARY_POWER);
            assert!(EXPLOSIVE_DAMAGE > INCENDIARY_DAMAGE);
        }
    }

    #[test]
    fn no_damage_constant_one_shots_a_full_health_player() {
        for points in [
            INCENDIARY_DAMAGE,
            EXPLOSIVE_DAMAGE,
            TORPEDO_DAMAGE,
            SEEKER_DAMAGE,
            SPLIT_PARENT_DAMAGE,
            FRAGMENT_DAMAGE,
        ] {
            assert!(points <= crate::domain::MAX_NON_LETHAL);
        }
    }

    #[test]
    fn only_the_intergalactic_announces_its_launch() {
        let announcing: Vec<_> = SheepKind::ALL
            .into_iter()
            .filter(|kind| announces_launch(*kind))
            .collect();
        assert_eq!(announcing, vec![SheepKind::Intergalactic]);
    }

    #[test]
    fn healing_and_boarding_are_thrown_shorter_than_the_others() {
        for kind in SheepKind::ALL {
            if matches!(kind, SheepKind::Healing | SheepKind::Boarding) {
                assert!(launch_power(kind) < super::DEFAULT_THROW_POWER);
            } else {
                assert_eq!(launch_power(kind), super::DEFAULT_THROW_POWER);
            }
        }
    }

    #[test]
    fn there_are_thirteen_kinds() {
        assert_eq!(SheepKind::ALL.len(), 13);
    }
}
