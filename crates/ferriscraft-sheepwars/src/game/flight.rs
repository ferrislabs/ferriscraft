use rspigot_plugin::Material;

use crate::domain::{PlayerId, SheepKind, TeamId};
use crate::sheep::{
    Action, FRAGMENT_LANDING_GRACE_TICKS, SEEKER_DETONATE_DISTANCE, SEEKER_FUSE_TICKS,
    SEEKER_RUN_SPEED, TORPEDO_CLEAR_DELAY_TICKS, TORPEDO_CLEAR_RADIUS, TORPEDO_MAX_FLIGHT_TICKS,
    TORPEDO_SPEED, TORPEDO_TRIGGER_RADIUS, Vec3, chases_after_impact, display_name, flies_straight,
    launch_power, launch_velocity, sheep_wool, step_ballistic, throw_velocity,
};

pub const MAX_CONCURRENT_FLIGHTS: usize = 32;
pub const RIDER_TELEPORT_DISTANCE: f64 = 6.0;
pub const RIDER_MAX_SPEED: f64 = 2.0;
pub const MAX_FLIGHT_TICKS: u32 = 20 * 15;
pub const LANDING_GRACE_TICKS: u32 = 4;
pub const SHEEP_FUSE_TICKS: u32 = 20 * 3;
pub const EYE_HEIGHT: f64 = 1.62;
pub const CHASE_DOWN_SPEED: f64 = -0.08;

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
    !(matches!(
        kind,
        SheepKind::Healing | SheepKind::Boarding | SheepKind::Torpedo
    ) || lands_instantly(kind)
        || is_fragment(kind))
}

pub const fn lands_instantly(kind: SheepKind) -> bool {
    matches!(
        kind,
        SheepKind::Quake | SheepKind::Warp | SheepKind::Intergalactic
    )
}

pub const fn disables_ai_when_armed(kind: SheepKind) -> bool {
    !chases_after_impact(kind)
}

pub const fn is_fragment(kind: SheepKind) -> bool {
    matches!(kind, SheepKind::Fragment)
}

pub const fn landing_grace_ticks(kind: SheepKind) -> u32 {
    if is_fragment(kind) {
        FRAGMENT_LANDING_GRACE_TICKS
    } else {
        LANDING_GRACE_TICKS
    }
}

pub const fn starts_effect(kind: SheepKind) -> bool {
    matches!(
        kind,
        SheepKind::Quake | SheepKind::Warp | SheepKind::Intergalactic | SheepKind::Frozen
    )
}

pub fn initial_velocity(kind: SheepKind, yaw: f64, pitch: f64) -> Vec3 {
    if flies_straight(kind) {
        launch_velocity(yaw, pitch, TORPEDO_SPEED)
    } else {
        throw_velocity(yaw, pitch, launch_power(kind))
    }
}

pub fn flight_velocity(kind: SheepKind, velocity: Vec3, ticks: u32) -> Vec3 {
    if flies_straight(kind) {
        velocity
    } else {
        (0..ticks).fold(velocity, |v, _| step_ballistic(v))
    }
}

pub fn torpedo_clear(at: Vec3) -> Action {
    Action::ClearSphere {
        center: at,
        radius: TORPEDO_CLEAR_RADIUS,
    }
}

pub const fn torpedo_clears(age: u32) -> bool {
    age >= TORPEDO_CLEAR_DELAY_TICKS
}

pub fn rider_needs_teleport(distance_squared: f64) -> bool {
    distance_squared > RIDER_TELEPORT_DISTANCE * RIDER_TELEPORT_DISTANCE
}

pub fn torpedo_triggered(distance: f64) -> bool {
    distance <= TORPEDO_TRIGGER_RADIUS
}

pub fn chase_velocity(from: Vec3, target: Vec3) -> Vec3 {
    let (dx, dz) = (target.x - from.x, target.z - from.z);
    let length = dx.hypot(dz);
    if length <= f64::EPSILON {
        return Vec3 {
            x: 0.0,
            y: CHASE_DOWN_SPEED,
            z: 0.0,
        };
    }
    Vec3 {
        x: dx / length * SEEKER_RUN_SPEED,
        y: CHASE_DOWN_SPEED,
        z: dz / length * SEEKER_RUN_SPEED,
    }
}

pub fn chase_detonates(distance: f64) -> bool {
    distance <= SEEKER_DETONATE_DISTANCE
}

pub const fn fuse_ticks(kind: SheepKind) -> u32 {
    if chases_after_impact(kind) {
        SEEKER_FUSE_TICKS
    } else {
        SHEEP_FUSE_TICKS
    }
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
        if flies_straight(self.kind) {
            return self.age >= TORPEDO_MAX_FLIGHT_TICKS;
        }
        let hit = on_ground || (wall_hit && !waits_for_ground(self.kind));
        (hit && self.age >= landing_grace_ticks(self.kind)) || self.age >= MAX_FLIGHT_TICKS
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
        assert!(arms_on_impact(SheepKind::Frozen));
        assert!(!arms_on_impact(SheepKind::Quake));
        assert!(!arms_on_impact(SheepKind::Warp));
        assert!(!arms_on_impact(SheepKind::Intergalactic));
        assert!(arms_on_impact(SheepKind::Seeker));
        assert!(!arms_on_impact(SheepKind::Torpedo));
    }

    #[test]
    fn only_the_chaser_keeps_its_ai_so_that_its_velocity_still_moves_it() {
        assert!(!disables_ai_when_armed(SheepKind::Seeker));
        for kind in SheepKind::ALL {
            if kind != SheepKind::Seeker {
                assert!(disables_ai_when_armed(kind), "{kind:?}");
            }
        }
    }

    #[test]
    fn a_boarding_rider_never_outruns_a_normal_throw() {
        const {
            assert!(RIDER_MAX_SPEED < crate::sheep::DEFAULT_THROW_POWER);
            assert!(RIDER_MAX_SPEED > crate::sheep::BOARDING_THROW_POWER);
        }
        let sheep = initial_velocity(SheepKind::Boarding, 0.0, -20.0);
        assert!(speed(sheep) < crate::sheep::DEFAULT_THROW_POWER);
    }

    #[test]
    fn the_torpedo_waits_a_tenth_of_a_second_before_clearing_blocks() {
        assert_eq!(TORPEDO_CLEAR_DELAY_TICKS, 2);
        assert!(!torpedo_clears(0));
        assert!(!torpedo_clears(TORPEDO_CLEAR_DELAY_TICKS - 1));
        assert!(torpedo_clears(TORPEDO_CLEAR_DELAY_TICKS));
        assert!(torpedo_clears(TORPEDO_CLEAR_DELAY_TICKS + 10));
    }

    #[test]
    fn a_boarding_rider_is_only_teleported_when_they_are_far_from_the_sheep() {
        assert!(!rider_needs_teleport(0.0));
        assert!(!rider_needs_teleport(2.6 * 2.6));
        assert!(!rider_needs_teleport(
            RIDER_TELEPORT_DISTANCE * RIDER_TELEPORT_DISTANCE
        ));
        assert!(rider_needs_teleport(
            (RIDER_TELEPORT_DISTANCE + 0.1) * (RIDER_TELEPORT_DISTANCE + 0.1)
        ));
    }

    #[test]
    fn the_frozen_sheep_starts_a_frost_zone_but_still_arms() {
        assert!(starts_effect(SheepKind::Frozen));
        assert!(arms_on_impact(SheepKind::Frozen));
    }

    fn speed(v: Vec3) -> f64 {
        (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
    }

    #[test]
    fn spec_swi_5_torpedo_flies_straight_at_constant_speed_without_gravity() {
        let launch = initial_velocity(SheepKind::Torpedo, 30.0, 10.0);
        assert!((speed(launch) - TORPEDO_SPEED).abs() < 1e-9);
        let later = flight_velocity(SheepKind::Torpedo, launch, 50);
        assert_eq!(later, launch);
        let level = initial_velocity(SheepKind::Torpedo, 0.0, 0.0);
        assert_eq!(flight_velocity(SheepKind::Torpedo, level, 20).y, 0.0);
        let arc = flight_velocity(SheepKind::Lightning, level, 20);
        assert!(arc.y < 0.0);
    }

    #[test]
    fn torpedo_ignores_ground_and_walls_until_its_lifetime_ends() {
        assert!(!flight(SheepKind::Torpedo, LANDING_GRACE_TICKS).should_land(true, true));
        assert!(!flight(SheepKind::Torpedo, TORPEDO_MAX_FLIGHT_TICKS - 1).should_land(true, true));
        assert!(flight(SheepKind::Torpedo, TORPEDO_MAX_FLIGHT_TICKS).should_land(false, false));
    }

    #[test]
    fn spec_swi_6_torpedo_triggers_inside_the_radius_only() {
        assert!(torpedo_triggered(TORPEDO_TRIGGER_RADIUS));
        assert!(torpedo_triggered(0.5));
        assert!(!torpedo_triggered(TORPEDO_TRIGGER_RADIUS + 0.01));
    }

    #[test]
    fn spec_swi_7_torpedo_clears_a_sphere_of_radius_three_around_itself() {
        let at = Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        assert_eq!(
            torpedo_clear(at),
            Action::ClearSphere {
                center: at,
                radius: 3.0
            }
        );
    }

    #[test]
    fn spec_swi_8_seeker_runs_horizontally_toward_the_target_and_fuses_within_forty_ticks() {
        let from = Vec3::ZERO;
        let target = Vec3 {
            x: 3.0,
            y: 40.0,
            z: 4.0,
        };
        let v = chase_velocity(from, target);
        assert!((v.x.hypot(v.z) - SEEKER_RUN_SPEED).abs() < 1e-9);
        assert!((v.x / v.z - 0.75).abs() < 1e-9);
        assert!(v.y <= 0.0);
        assert!(fuse_ticks(SheepKind::Seeker) <= 40);
        assert_eq!(fuse_ticks(SheepKind::Incendiary), SHEEP_FUSE_TICKS);
        assert!(chase_detonates(SEEKER_DETONATE_DISTANCE));
        assert!(!chase_detonates(SEEKER_DETONATE_DISTANCE + 0.1));
    }

    #[test]
    fn only_timed_sheep_start_an_effect() {
        assert!(starts_effect(SheepKind::Quake));
        assert!(starts_effect(SheepKind::Warp));
        assert!(starts_effect(SheepKind::Intergalactic));
        assert!(!starts_effect(SheepKind::Explosive));
    }

    #[test]
    fn a_fragment_lands_on_its_first_ground_contact_without_a_fuse() {
        assert!(!arms_on_impact(SheepKind::Fragment));
        let grace = landing_grace_ticks(SheepKind::Fragment);
        assert!(grace > LANDING_GRACE_TICKS);
        assert!(!flight(SheepKind::Fragment, grace - 1).should_land(true, false));
        assert!(flight(SheepKind::Fragment, grace).should_land(true, false));
        assert!(!flight(SheepKind::Fragment, grace).should_land(false, false));
    }

    #[test]
    fn only_the_fragment_has_a_longer_landing_grace() {
        for kind in SheepKind::ALL {
            assert_eq!(landing_grace_ticks(kind), LANDING_GRACE_TICKS);
        }
    }

    #[test]
    fn fuse_label_formats_seconds_and_turns_red_under_one_second() {
        assert!(fuse_label(SheepKind::Incendiary, 60).contains("§e(3.0s)"));
        assert!(fuse_label(SheepKind::Incendiary, 20).contains("§c(1.0s)"));
    }
}
