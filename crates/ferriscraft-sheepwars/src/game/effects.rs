use crate::domain::Damage;
use crate::domain::SheepKind;
use crate::domain::TeamId;
use crate::sheep::Action;
use crate::sheep::Cell;
use crate::sheep::INTERGALACTIC_CLEAR_HEIGHT;
use crate::sheep::INTERGALACTIC_FALL_TICKS;
use crate::sheep::INTERGALACTIC_POWER;
use crate::sheep::INTERGALACTIC_RADIUS;
use crate::sheep::INTERGALACTIC_SKY_HEIGHT;
use crate::sheep::ParticleKind;
use crate::sheep::QUAKE_DURATION_TICKS;
use crate::sheep::QUAKE_PULSE_LIFT;
use crate::sheep::QUAKE_PULSE_TICKS;
use crate::sheep::QUAKE_RADIUS;
use crate::sheep::SoundKind;
use crate::sheep::Vec3;
use crate::sheep::WARP_BLOCK_PAIRS;
use crate::sheep::WARP_BLOCK_RADIUS;
use crate::sheep::WARP_DELAY_TICKS;

const METEOR_OFFSET_X: f64 = 6.0;
const METEOR_OFFSET_Z: f64 = 4.0;
const QUAKE_PARTICLE_POINTS: u32 = 6;
const WARP_MAX_ATTEMPTS: u32 = 4096;

#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn seeded(at: Vec3) -> Self {
        let mixed = at.x.to_bits()
            ^ at.y.to_bits().rotate_left(21)
            ^ at.z.to_bits().rotate_left(42)
            ^ 0x9E37_79B9_7F4A_7C15;
        Self(mixed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn offset(&mut self, bound: i32) -> i32 {
        let span = (bound * 2 + 1) as u64;
        (self.next() % span) as i32 - bound
    }
}

#[derive(Debug)]
enum Effect {
    Quake { at: Vec3, team: TeamId, age: u32 },
    Warp { at: Vec3, age: u32, rng: Rng },
    Meteor { at: Vec3, team: TeamId, age: u32 },
}

#[derive(Debug, Default)]
pub struct Effects {
    active: Vec<Effect>,
}

impl Effects {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, kind: SheepKind, at: Vec3, team: TeamId) {
        let effect = match kind {
            SheepKind::Quake => Effect::Quake { at, team, age: 0 },
            SheepKind::Warp => Effect::Warp {
                at,
                age: 0,
                rng: Rng::seeded(at),
            },
            SheepKind::Intergalactic => Effect::Meteor { at, team, age: 0 },
            _ => return,
        };
        self.active.push(effect);
    }

    pub fn tick(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        self.active.retain_mut(|effect| match effect {
            Effect::Quake { at, team, age } => {
                *age += 1;
                if *age % QUAKE_PULSE_TICKS == 0 {
                    quake_pulse(*at, team.other(), &mut actions);
                }
                *age < QUAKE_DURATION_TICKS
            }
            Effect::Warp { at, age, rng } => {
                *age += 1;
                if *age < WARP_DELAY_TICKS {
                    return true;
                }
                warp_burst(*at, rng, &mut actions);
                false
            }
            Effect::Meteor { at, team, age } => {
                *age += 1;
                meteor_step(*at, team.other(), *age, &mut actions);
                *age < INTERGALACTIC_FALL_TICKS
            }
        });
        actions
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }
}

fn quake_pulse(at: Vec3, enemies: TeamId, actions: &mut Vec<Action>) {
    actions.push(Action::Velocity {
        team: enemies,
        center: at,
        radius: QUAKE_RADIUS,
        velocity: Vec3 {
            x: 0.0,
            y: QUAKE_PULSE_LIFT,
            z: 0.0,
        },
    });
    for point in 0..QUAKE_PARTICLE_POINTS {
        let angle = f64::from(point) / f64::from(QUAKE_PARTICLE_POINTS) * std::f64::consts::TAU;
        let ring = Vec3 {
            x: at.x + angle.cos() * QUAKE_RADIUS * 0.5,
            y: at.y + 0.2,
            z: at.z + angle.sin() * QUAKE_RADIUS * 0.5,
        };
        let kind = if point % 2 == 0 {
            ParticleKind::Explosion
        } else {
            ParticleKind::LargeSmoke
        };
        actions.push(Action::Particle {
            kind,
            at: ring,
            count: 2,
        });
    }
    actions.push(Action::Sound {
        kind: SoundKind::Quake,
        at,
    });
}

fn within_warp_radius(at: Vec3, origin: Cell, cell: Cell) -> bool {
    let radius = WARP_BLOCK_RADIUS;
    let index = [cell.x - origin.x, cell.y - origin.y, cell.z - origin.z]
        .map(|d| f64::from(d) * f64::from(d))
        .iter()
        .sum::<f64>();
    let dx = f64::from(cell.x) + 0.5 - at.x;
    let dy = f64::from(cell.y) + 0.5 - at.y;
    let dz = f64::from(cell.z) + 0.5 - at.z;
    index <= radius * radius && dx * dx + dy * dy + dz * dz <= radius * radius
}

fn draw_cell(at: Vec3, rng: &mut Rng, used: &[Cell]) -> Option<Cell> {
    let origin = at.cell();
    bound_attempts().find_map(|_| {
        let bound = WARP_BLOCK_RADIUS as i32;
        let cell = Cell {
            x: origin.x + rng.offset(bound),
            y: origin.y + rng.offset(bound),
            z: origin.z + rng.offset(bound),
        };
        (within_warp_radius(at, origin, cell) && !used.contains(&cell)).then_some(cell)
    })
}

fn bound_attempts() -> std::ops::Range<u32> {
    0..WARP_MAX_ATTEMPTS
}

fn warp_burst(at: Vec3, rng: &mut Rng, actions: &mut Vec<Action>) {
    let mut used: Vec<Cell> = Vec::new();
    for _ in 0..WARP_BLOCK_PAIRS {
        let Some(a) = draw_cell(at, rng, &used) else {
            break;
        };
        used.push(a);
        let Some(b) = draw_cell(at, rng, &used) else {
            break;
        };
        used.push(b);
        actions.push(Action::SwapBlocks { a, b });
    }
    actions.push(Action::Particle {
        kind: ParticleKind::Portal,
        at,
        count: 80,
    });
    actions.push(Action::Sound {
        kind: SoundKind::Warp,
        at,
    });
}

fn meteor_step(at: Vec3, enemies: TeamId, age: u32, actions: &mut Vec<Action>) {
    let progress = f64::from(age) / f64::from(INTERGALACTIC_FALL_TICKS);
    let remaining = 1.0 - progress;
    let position = Vec3 {
        x: at.x + METEOR_OFFSET_X * remaining,
        y: at.y + INTERGALACTIC_SKY_HEIGHT * remaining,
        z: at.z + METEOR_OFFSET_Z * remaining,
    };
    for kind in [
        ParticleKind::Flame,
        ParticleKind::Explosion,
        ParticleKind::LargeSmoke,
    ] {
        actions.push(Action::Particle {
            kind,
            at: position,
            count: 4,
        });
    }
    if age < INTERGALACTIC_FALL_TICKS {
        return;
    }
    actions.push(Action::Explode {
        at,
        power: INTERGALACTIC_POWER,
        breaks_blocks: false,
    });
    actions.push(Action::ClearAbove {
        center: at,
        radius: INTERGALACTIC_RADIUS,
        height: INTERGALACTIC_CLEAR_HEIGHT,
    });
    actions.push(Action::Damage {
        team: enemies,
        center: at,
        radius: INTERGALACTIC_RADIUS,
        damage: Damage::lethal(),
    });
    actions.push(Action::Lightning { at });
    actions.push(Action::Particle {
        kind: ParticleKind::Explosion,
        at,
        count: 10,
    });
    actions.push(Action::Sound {
        kind: SoundKind::Intergalactic,
        at,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: Vec3 = Vec3 {
        x: 10.5,
        y: 64.0,
        z: -3.25,
    };

    fn run(effects: &mut Effects, ticks: u32) -> Vec<Vec<Action>> {
        (0..ticks).map(|_| effects.tick()).collect()
    }

    fn is_game_action(action: &Action) -> bool {
        !matches!(action, Action::Particle { .. } | Action::Sound { .. })
    }

    #[test]
    fn spec_swi_12_quake_pulses_every_forty_ticks_then_idles() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Quake, ORIGIN, TeamId::Red);
        let frames = run(&mut effects, QUAKE_DURATION_TICKS);
        for (index, frame) in frames.iter().enumerate() {
            let tick = index as u32 + 1;
            let velocities: Vec<&Action> = frame
                .iter()
                .filter(|a| matches!(a, Action::Velocity { .. }))
                .collect();
            if tick.is_multiple_of(QUAKE_PULSE_TICKS) {
                assert_eq!(velocities.len(), 1, "tick {tick}");
                assert!(matches!(
                    velocities[0],
                    Action::Velocity { team: TeamId::Blue, radius, velocity, .. }
                        if *radius == QUAKE_RADIUS && velocity.y == QUAKE_PULSE_LIFT
                ));
                assert!(frame.iter().any(|a| matches!(
                    a,
                    Action::Sound {
                        kind: SoundKind::Quake,
                        ..
                    }
                )));
                assert!(frame.iter().any(|a| matches!(
                    a,
                    Action::Particle {
                        kind: ParticleKind::Explosion,
                        ..
                    }
                )));
                assert!(frame.iter().any(|a| matches!(
                    a,
                    Action::Particle {
                        kind: ParticleKind::LargeSmoke,
                        ..
                    }
                )));
            } else {
                assert!(frame.is_empty(), "tick {tick}");
            }
        }
        assert!(effects.is_idle());
        assert!(effects.tick().is_empty());
    }

    #[test]
    fn spec_swi_12_quake_is_active_until_last_tick() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Quake, ORIGIN, TeamId::Blue);
        run(&mut effects, QUAKE_DURATION_TICKS - 1);
        assert!(!effects.is_idle());
        effects.tick();
        assert!(effects.is_idle());
    }

    #[test]
    fn spec_swi_13_warp_waits_then_swaps_twelve_pairs() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Warp, ORIGIN, TeamId::Red);
        let frames = run(&mut effects, WARP_DELAY_TICKS - 1);
        assert!(frames.iter().flatten().all(|a| !is_game_action(a)));
        assert!(!effects.is_idle());
        let burst = effects.tick();
        let swaps: Vec<(Cell, Cell)> = burst
            .iter()
            .filter_map(|a| match a {
                Action::SwapBlocks { a, b } => Some((*a, *b)),
                _ => None,
            })
            .collect();
        assert_eq!(swaps.len(), WARP_BLOCK_PAIRS as usize);
        let mut seen: Vec<Cell> = Vec::new();
        for (a, b) in swaps {
            assert_ne!(a, b);
            for cell in [a, b] {
                let dx = f64::from(cell.x) - ORIGIN.x;
                let dy = f64::from(cell.y) - ORIGIN.y;
                let dz = f64::from(cell.z) - ORIGIN.z;
                assert!((dx * dx + dy * dy + dz * dz).sqrt() <= WARP_BLOCK_RADIUS + 1.0);
                let origin = ORIGIN.cell();
                let ix = f64::from(cell.x - origin.x);
                let iy = f64::from(cell.y - origin.y);
                let iz = f64::from(cell.z - origin.z);
                assert!((ix * ix + iy * iy + iz * iz).sqrt() <= WARP_BLOCK_RADIUS);
                assert!(!seen.contains(&cell));
                seen.push(cell);
            }
        }
        assert!(burst.iter().any(|a| matches!(
            a,
            Action::Particle {
                kind: ParticleKind::Portal,
                ..
            }
        )));
        assert!(burst.iter().any(|a| matches!(
            a,
            Action::Sound {
                kind: SoundKind::Warp,
                ..
            }
        )));
        assert!(effects.is_idle());
    }

    #[test]
    fn spec_swi_13_warp_is_deterministic() {
        let mut first = Effects::new();
        let mut second = Effects::new();
        first.start(SheepKind::Warp, ORIGIN, TeamId::Red);
        second.start(SheepKind::Warp, ORIGIN, TeamId::Red);
        assert_eq!(run(&mut first, 40), run(&mut second, 40));
    }

    #[test]
    fn spec_swi_14_meteor_falls_then_impacts_on_last_tick() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Intergalactic, ORIGIN, TeamId::Red);
        let frames = run(&mut effects, INTERGALACTIC_FALL_TICKS);
        let mut last_y = f64::INFINITY;
        for (index, frame) in frames.iter().enumerate() {
            let last = index as u32 + 1 == INTERGALACTIC_FALL_TICKS;
            let flame = frame
                .iter()
                .find_map(|a| match a {
                    Action::Particle {
                        kind: ParticleKind::Flame,
                        at,
                        ..
                    } => Some(at.y),
                    _ => None,
                })
                .expect("flame particle every tick");
            assert!(flame < last_y);
            last_y = flame;
            assert!(frame.iter().any(|a| matches!(
                a,
                Action::Particle {
                    kind: ParticleKind::Explosion,
                    ..
                }
            )));
            assert!(frame.iter().any(|a| matches!(
                a,
                Action::Particle {
                    kind: ParticleKind::LargeSmoke,
                    ..
                }
            )));
            let impact = frame.iter().any(is_game_action);
            assert_eq!(impact, last, "tick {}", index + 1);
        }
        let last = &frames[frames.len() - 1];
        assert!(last.iter().any(|a| matches!(
            a,
            Action::Explode { power, breaks_blocks: false, .. } if *power == INTERGALACTIC_POWER
        )));
        assert!(last
            .iter()
            .any(|a| matches!(a, Action::ClearAbove { height, .. } if *height == INTERGALACTIC_CLEAR_HEIGHT)));
        assert!(last.iter().any(|a| matches!(
            a,
            Action::Damage { team: TeamId::Blue, damage, .. } if damage.is_lethal()
        )));
        assert!(last.iter().any(|a| matches!(a, Action::Lightning { .. })));
        assert!(last.iter().any(|a| matches!(
            a,
            Action::Sound {
                kind: SoundKind::Intergalactic,
                ..
            }
        )));
        assert_eq!(last_y, ORIGIN.y);
        assert!(effects.is_idle());
    }

    #[test]
    fn other_kinds_are_ignored() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Explosive, ORIGIN, TeamId::Red);
        assert!(effects.is_idle());
        assert!(effects.tick().is_empty());
    }

    #[test]
    fn effects_run_concurrently_and_are_removed() {
        let mut effects = Effects::new();
        effects.start(SheepKind::Intergalactic, ORIGIN, TeamId::Red);
        effects.start(SheepKind::Warp, ORIGIN, TeamId::Red);
        run(&mut effects, INTERGALACTIC_FALL_TICKS);
        assert!(!effects.is_idle());
        run(&mut effects, WARP_DELAY_TICKS - INTERGALACTIC_FALL_TICKS);
        assert!(effects.is_idle());
    }
}
