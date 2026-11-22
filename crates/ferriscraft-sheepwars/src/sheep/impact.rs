use std::f64::consts::TAU;

use super::catalog;
use crate::domain::{Damage, SheepKind, TeamId};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn cell(self) -> Cell {
        Cell {
            x: self.x.floor() as i32,
            y: self.y.floor() as i32,
            z: self.z.floor() as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Ice,
    Fire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    Explosion,
    Flame,
    Snowflake,
    Crit,
    Heart,
    Portal,
    LargeSmoke,
    TeamDust(TeamId),
    EndRod,
    Firework,
    Totem,
    Note,
    ElectricSpark,
    Rainbow(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    Explode,
    Ice,
    Thunder,
    Heal,
    HealTick,
    Teleport,
    Quake,
    Darkness,
    Split,
    Warp,
    Intergalactic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Slowness,
    Regeneration,
    Blindness,
    Darkness,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Explode {
        at: Vec3,
        power: f32,
        breaks_blocks: bool,
    },
    SetBlock {
        at: Cell,
        block: BlockKind,
    },
    Particle {
        kind: ParticleKind,
        at: Vec3,
        count: i32,
    },
    Sound {
        kind: SoundKind,
        at: Vec3,
    },
    Effect {
        team: TeamId,
        center: Vec3,
        radius: f64,
        effect: EffectKind,
        duration_ticks: i32,
        amplifier: u8,
    },
    Heal {
        team: TeamId,
        center: Vec3,
        radius: f64,
        amount: f32,
    },
    Ignite {
        team: TeamId,
        center: Vec3,
        radius: f64,
        fire_ticks: i32,
    },
    Velocity {
        team: TeamId,
        center: Vec3,
        radius: f64,
        velocity: Vec3,
    },
    Push {
        team: TeamId,
        center: Vec3,
        radius: f64,
        strength: f64,
        lift: f64,
    },
    SwapPositions {
        team: TeamId,
        center: Vec3,
        radius: f64,
    },
    Lightning {
        at: Vec3,
    },
    SpawnSheep {
        kind: SheepKind,
        at: Vec3,
        velocity: Vec3,
    },
    TeleportThrower {
        to: Vec3,
    },
    Damage {
        team: TeamId,
        center: Vec3,
        radius: f64,
        damage: Damage,
    },
    ClearSphere {
        center: Vec3,
        radius: f64,
    },
    ClearAbove {
        center: Vec3,
        radius: f64,
        height: f64,
    },
    SwapBlocks {
        a: Cell,
        b: Cell,
    },
}

pub fn healing_pulse(at: Vec3, team: TeamId, with_chime: bool) -> Vec<Action> {
    let mut actions = vec![
        Action::Heal {
            team,
            center: at,
            radius: catalog::HEALING_RADIUS,
            amount: catalog::HEALING_PULSE_HEALTH,
        },
        Action::Effect {
            team,
            center: at,
            radius: catalog::HEALING_RADIUS,
            effect: EffectKind::Regeneration,
            duration_ticks: catalog::HEALING_REGENERATION_TICKS,
            amplifier: catalog::HEALING_REGENERATION_AMPLIFIER,
        },
        particle(ParticleKind::Heart, at, 14),
    ];
    if with_chime {
        actions.push(sound(SoundKind::HealTick, at));
    }
    actions
}

pub fn plan(kind: SheepKind, at: Vec3, thrower_team: TeamId) -> Vec<Action> {
    let enemies = thrower_team.other();
    match kind {
        SheepKind::Incendiary => {
            let mut actions = vec![
                Action::Explode {
                    at,
                    power: catalog::INCENDIARY_POWER,
                    breaks_blocks: catalog::INCENDIARY_BREAKS_BLOCKS,
                },
                Action::Damage {
                    team: enemies,
                    center: at,
                    radius: catalog::INCENDIARY_DAMAGE_RADIUS,
                    damage: Damage::capped(catalog::INCENDIARY_DAMAGE),
                },
                Action::Ignite {
                    team: enemies,
                    center: at,
                    radius: catalog::INCENDIARY_FIRE_RADIUS,
                    fire_ticks: catalog::INCENDIARY_FIRE_TICKS,
                },
            ];
            for index in 0..catalog::INCENDIARY_IGNITE_BLOCKS {
                let spot = spiral_point(
                    at,
                    index,
                    catalog::INCENDIARY_IGNITE_BLOCKS,
                    catalog::INCENDIARY_FIRE_SPREAD,
                );
                actions.push(Action::SetBlock {
                    at: spot.cell(),
                    block: BlockKind::Fire,
                });
            }
            actions.push(particle(ParticleKind::Explosion, at, 6));
            actions.push(particle(ParticleKind::Flame, at, 70));
            actions.push(sound(SoundKind::Explode, at));
            actions
        }
        SheepKind::Frozen => vec![
            particle(ParticleKind::Snowflake, at, 80),
            sound(SoundKind::Ice, at),
        ],
        SheepKind::Explosive => vec![
            Action::Explode {
                at,
                power: catalog::EXPLOSIVE_POWER,
                breaks_blocks: catalog::EXPLOSIVE_BREAKS_BLOCKS,
            },
            Action::Damage {
                team: enemies,
                center: at,
                radius: catalog::EXPLOSIVE_DAMAGE_RADIUS,
                damage: Damage::capped(catalog::EXPLOSIVE_DAMAGE),
            },
            particle(ParticleKind::Explosion, at, 10),
            particle(ParticleKind::LargeSmoke, at, 30),
            sound(SoundKind::Explode, at),
        ],
        SheepKind::Fragment => vec![
            Action::Explode {
                at,
                power: catalog::FRAGMENT_POWER,
                breaks_blocks: catalog::FRAGMENT_BREAKS_BLOCKS,
            },
            Action::Damage {
                team: enemies,
                center: at,
                radius: catalog::FRAGMENT_DAMAGE_RADIUS,
                damage: Damage::capped(catalog::FRAGMENT_DAMAGE),
            },
            particle(ParticleKind::Explosion, at, 4),
            sound(SoundKind::Explode, at),
        ],
        SheepKind::Torpedo => vec![
            Action::Explode {
                at,
                power: catalog::TORPEDO_POWER,
                breaks_blocks: false,
            },
            Action::Damage {
                team: enemies,
                center: at,
                radius: catalog::TORPEDO_DAMAGE_RADIUS,
                damage: Damage::capped(catalog::TORPEDO_DAMAGE),
            },
            particle(ParticleKind::Explosion, at, 8),
            particle(ParticleKind::Crit, at, 30),
            sound(SoundKind::Explode, at),
        ],
        SheepKind::Seeker => vec![
            Action::Explode {
                at,
                power: catalog::SEEKER_POWER,
                breaks_blocks: catalog::SEEKER_BREAKS_BLOCKS,
            },
            Action::Damage {
                team: enemies,
                center: at,
                radius: catalog::SEEKER_DAMAGE_RADIUS,
                damage: Damage::capped(catalog::SEEKER_DAMAGE),
            },
            particle(ParticleKind::Crit, at, 35),
            sound(SoundKind::Explode, at),
        ],
        SheepKind::Lightning => {
            let mut actions = vec![Action::Lightning { at }];
            for index in 1..catalog::LIGHTNING_STRIKES {
                actions.push(Action::Lightning {
                    at: spiral_point(
                        at,
                        index,
                        catalog::LIGHTNING_STRIKES,
                        catalog::LIGHTNING_SPREAD,
                    ),
                });
            }
            actions.push(sound(SoundKind::Thunder, at));
            actions
        }
        SheepKind::Healing => {
            let mut actions = healing_pulse(at, thrower_team, false);
            actions.push(sound(SoundKind::Heal, at));
            actions
        }
        SheepKind::Boarding => vec![
            Action::TeleportThrower { to: at },
            particle(ParticleKind::Portal, at, 50),
            sound(SoundKind::Teleport, at),
        ],
        SheepKind::Quake => vec![
            particle(ParticleKind::Explosion, at, 14),
            particle(ParticleKind::LargeSmoke, at, 24),
            sound(SoundKind::Quake, at),
        ],
        SheepKind::Darkness => vec![
            Action::Effect {
                team: enemies,
                center: at,
                radius: catalog::DARKNESS_RADIUS,
                effect: EffectKind::Blindness,
                duration_ticks: catalog::DARKNESS_BLINDNESS_TICKS,
                amplifier: catalog::DARKNESS_BLINDNESS_AMPLIFIER,
            },
            Action::Effect {
                team: enemies,
                center: at,
                radius: catalog::DARKNESS_RADIUS,
                effect: EffectKind::Darkness,
                duration_ticks: catalog::DARKNESS_BLINDNESS_TICKS,
                amplifier: catalog::DARKNESS_BLINDNESS_AMPLIFIER,
            },
            particle(ParticleKind::LargeSmoke, at, 70),
            sound(SoundKind::Darkness, at),
        ],
        SheepKind::Splitting => {
            let mut actions = vec![Action::Explode {
                at,
                power: catalog::SPLIT_PARENT_POWER,
                breaks_blocks: catalog::SPLIT_PARENT_BREAKS_BLOCKS,
            }];
            actions.push(Action::Damage {
                team: enemies,
                center: at,
                radius: catalog::SPLIT_PARENT_DAMAGE_RADIUS,
                damage: Damage::capped(catalog::SPLIT_PARENT_DAMAGE),
            });
            actions.extend((0..catalog::SPLIT_COUNT).map(|index| {
                let angle = TAU * f64::from(index) / f64::from(catalog::SPLIT_COUNT);
                Action::SpawnSheep {
                    kind: catalog::SPLIT_CHILD,
                    at,
                    velocity: Vec3 {
                        x: angle.cos() * catalog::SPLIT_HORIZONTAL_SPEED,
                        y: catalog::SPLIT_VERTICAL_SPEED,
                        z: angle.sin() * catalog::SPLIT_HORIZONTAL_SPEED,
                    },
                }
            }));
            actions.push(sound(SoundKind::Split, at));
            actions
        }
        SheepKind::Warp => vec![
            particle(ParticleKind::Portal, at, 80),
            sound(SoundKind::Warp, at),
        ],
        SheepKind::Intergalactic => vec![
            particle(ParticleKind::Explosion, at, 20),
            sound(SoundKind::Intergalactic, at),
        ],
    }
}

const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

fn spiral_point(center: Vec3, index: u32, count: u32, spread: f64) -> Vec3 {
    let share = f64::from(index + 1) / f64::from(count.max(1));
    let distance = spread * share.sqrt();
    let angle = GOLDEN_ANGLE * f64::from(index);
    Vec3 {
        x: center.x + angle.cos() * distance,
        y: center.y,
        z: center.z + angle.sin() * distance,
    }
}

pub fn outward_velocity(center: Vec3, target: Vec3, strength: f64, lift: f64) -> Vec3 {
    let (dx, dz) = (target.x - center.x, target.z - center.z);
    let length = dx.hypot(dz);
    if length <= f64::EPSILON {
        return Vec3 {
            x: 0.0,
            y: lift,
            z: 0.0,
        };
    }
    Vec3 {
        x: dx / length * strength,
        y: lift,
        z: dz / length * strength,
    }
}

fn particle(kind: ParticleKind, at: Vec3, count: i32) -> Action {
    Action::Particle { kind, at, count }
}

fn sound(kind: SoundKind, at: Vec3) -> Action {
    Action::Sound { kind, at }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Vec3 = Vec3 {
        x: 10.0,
        y: 64.0,
        z: -5.0,
    };

    #[test]
    fn outward_velocity_points_away_from_the_center() {
        let v = outward_velocity(
            Vec3::ZERO,
            Vec3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            2.0,
            0.5,
        );
        assert_eq!(
            v,
            Vec3 {
                x: 2.0,
                y: 0.5,
                z: 0.0
            }
        );
        let on_center = outward_velocity(Vec3::ZERO, Vec3::ZERO, 2.0, 0.5);
        assert_eq!(
            on_center,
            Vec3 {
                x: 0.0,
                y: 0.5,
                z: 0.0
            }
        );
    }

    fn is_sensory(action: &Action) -> bool {
        matches!(action, Action::Particle { .. } | Action::Sound { .. })
    }

    fn damage_of(actions: &[Action]) -> Vec<(Damage, f64)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Damage { damage, radius, .. } => Some((*damage, *radius)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn intergalactic_plan_is_sensory_only() {
        let actions = plan(SheepKind::Intergalactic, AT, TeamId::Red);
        assert!(!actions.is_empty());
        assert!(actions.iter().all(is_sensory));
        assert!(actions.contains(&Action::Sound {
            kind: SoundKind::Intergalactic,
            at: AT
        }));
    }

    #[test]
    fn quake_and_warp_plans_are_sensory_only() {
        for kind in [SheepKind::Quake, SheepKind::Warp] {
            let actions = plan(kind, AT, TeamId::Red);
            assert!(!actions.is_empty());
            assert!(actions.iter().all(is_sensory));
        }
    }

    #[test]
    fn no_sheep_but_the_intergalactic_deals_lethal_damage() {
        for kind in SheepKind::ALL {
            if kind == SheepKind::Intergalactic {
                continue;
            }
            for (damage, radius) in damage_of(&plan(kind, AT, TeamId::Red)) {
                assert!(!damage.is_lethal());
                assert!(damage.at_distance(0.0, radius) < 20.0);
            }
        }
    }

    #[test]
    fn damage_only_targets_the_enemy_team() {
        for kind in SheepKind::ALL {
            for action in plan(kind, AT, TeamId::Red) {
                if let Action::Damage { team, .. } = action {
                    assert_eq!(team, TeamId::Blue);
                }
            }
        }
    }

    #[test]
    fn explosive_torpedo_and_seeker_explode_and_hurt() {
        for (kind, power) in [
            (SheepKind::Explosive, catalog::EXPLOSIVE_POWER),
            (SheepKind::Torpedo, catalog::TORPEDO_POWER),
            (SheepKind::Seeker, catalog::SEEKER_POWER),
        ] {
            let actions = plan(kind, AT, TeamId::Red);
            assert!(actions.iter().any(
                |a| matches!(a, Action::Explode { at, power: p, .. } if *at == AT && *p == power)
            ));
            assert_eq!(damage_of(&actions).len(), 1);
        }
        assert!(
            !plan(SheepKind::Torpedo, AT, TeamId::Red)
                .iter()
                .any(|a| matches!(
                    a,
                    Action::Explode {
                        breaks_blocks: true,
                        ..
                    }
                ))
        );
    }

    #[test]
    fn splitting_children_are_fragments_and_counted() {
        let children = plan(SheepKind::Splitting, AT, TeamId::Red)
            .into_iter()
            .filter(|a| {
                matches!(
                    a,
                    Action::SpawnSheep {
                        kind: SheepKind::Fragment,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(children, catalog::SPLIT_COUNT as usize);
        assert_eq!(catalog::SPLIT_COUNT, 4);
    }

    #[test]
    fn a_fragment_explodes_and_hurts_without_one_shotting() {
        let actions = plan(SheepKind::Fragment, AT, TeamId::Red);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Explode { at, .. } if *at == AT))
        );
        let damage = actions.iter().find_map(|a| match a {
            Action::Damage { damage, .. } => Some(*damage),
            _ => None,
        });
        assert!(damage.is_some_and(|d| !d.is_lethal() && d.at_distance(0.0, 4.0) < 20.0));
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::SpawnSheep { .. }))
        );
    }

    #[test]
    fn a_fragment_is_smaller_than_the_explosive() {
        const {
            assert!(catalog::FRAGMENT_POWER < catalog::EXPLOSIVE_POWER);
            assert!(catalog::FRAGMENT_SCALE < 1.0);
        }
    }

    #[test]
    fn incendiary_hurts_less_than_explosive_and_lights_more_fire_than_before() {
        let incendiary = plan(SheepKind::Incendiary, AT, TeamId::Red);
        let explosive = plan(SheepKind::Explosive, AT, TeamId::Red);
        let hurt = |actions: &[Action]| {
            let (damage, radius) = damage_of(actions)[0];
            damage.at_distance(0.0, radius)
        };
        assert!(hurt(&incendiary) < hurt(&explosive));
        let fires = incendiary
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    Action::SetBlock {
                        block: BlockKind::Fire,
                        ..
                    }
                )
            })
            .count();
        assert!(fires > 7);
    }

    #[test]
    fn frozen_only_announces_its_landing_and_leaves_the_frost_to_the_zone() {
        let actions = plan(SheepKind::Frozen, AT, TeamId::Red);
        assert!(actions.iter().any(|a| matches!(a, Action::Sound { .. })));
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::Effect { .. } | Action::SetBlock { .. }))
        );
    }

    #[test]
    fn healing_targets_the_thrower_team() {
        assert!(
            plan(SheepKind::Healing, AT, TeamId::Blue)
                .iter()
                .any(|a| matches!(
                    a,
                    Action::Effect {
                        team: TeamId::Blue,
                        effect: EffectKind::Regeneration,
                        ..
                    }
                ))
        );
    }

    #[test]
    fn boarding_teleports_the_thrower_to_the_impact() {
        assert!(
            plan(SheepKind::Boarding, AT, TeamId::Red)
                .contains(&Action::TeleportThrower { to: AT })
        );
    }

    #[test]
    fn lightning_strikes_at_the_impact() {
        assert!(
            plan(SheepKind::Lightning, AT, TeamId::Red).contains(&Action::Lightning { at: AT })
        );
    }

    #[test]
    fn darkness_blinds_the_enemy_team() {
        assert!(
            plan(SheepKind::Darkness, AT, TeamId::Red)
                .iter()
                .any(|a| matches!(
                    a,
                    Action::Effect {
                        team: TeamId::Blue,
                        effect: EffectKind::Blindness,
                        ..
                    }
                ))
        );
    }

    #[test]
    fn splitting_also_explodes_itself() {
        assert!(
            plan(SheepKind::Splitting, AT, TeamId::Red)
                .iter()
                .any(|a| matches!(a, Action::Explode { at, .. } if *at == AT))
        );
    }

    #[test]
    fn splitting_spawns_the_configured_number_of_children_in_distinct_directions() {
        let children: Vec<Vec3> = plan(SheepKind::Splitting, AT, TeamId::Red)
            .into_iter()
            .filter_map(|a| match a {
                Action::SpawnSheep { kind, velocity, .. } if kind == catalog::SPLIT_CHILD => {
                    Some(velocity)
                }
                _ => None,
            })
            .collect();
        assert_eq!(children.len(), catalog::SPLIT_COUNT as usize);
        for (index, child) in children.iter().enumerate() {
            assert!(!children[index + 1..].contains(child));
        }
    }

    #[test]
    fn incendiary_explodes_ignites_enemies_and_places_fire() {
        let actions = plan(SheepKind::Incendiary, AT, TeamId::Red);
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Explode { power, breaks_blocks: true, .. } if *power >= catalog::INCENDIARY_POWER
        )));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::Ignite { team: TeamId::Blue, fire_ticks, .. } if *fire_ticks > 0
        )));
        assert!(actions.iter().any(|a| matches!(
            a,
            Action::SetBlock {
                block: BlockKind::Fire,
                ..
            }
        )));
    }

    #[test]
    fn lightning_strikes_multiple_points_and_healing_pulses_heal_and_regen() {
        let strikes = plan(SheepKind::Lightning, AT, TeamId::Red)
            .into_iter()
            .filter(|a| matches!(a, Action::Lightning { .. }))
            .count();
        assert_eq!(strikes, catalog::LIGHTNING_STRIKES as usize);

        let pulse = healing_pulse(AT, TeamId::Blue, true);
        assert!(pulse.iter().any(|a| matches!(
            a,
            Action::Heal { team: TeamId::Blue, amount, .. } if *amount > 0.0
        )));
        assert!(pulse.iter().any(|a| matches!(
            a,
            Action::Effect {
                team: TeamId::Blue,
                effect: EffectKind::Regeneration,
                ..
            }
        )));
        assert!(pulse.iter().any(|a| matches!(
            a,
            Action::Sound {
                kind: SoundKind::HealTick,
                ..
            }
        )));
    }

    #[test]
    fn darkness_applies_darkness() {
        assert!(
            plan(SheepKind::Darkness, AT, TeamId::Red)
                .iter()
                .any(|a| matches!(
                    a,
                    Action::Effect {
                        team: TeamId::Blue,
                        effect: EffectKind::Darkness,
                        ..
                    }
                ))
        );
    }
}
