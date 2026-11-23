use std::collections::HashMap;

use rspigot_plugin::{BlockType, Color, Material, Particle, Transformation};

use crate::domain::{LaneId, Level, MobId, MobKind, TowerId, TowerKind, Vec3};
use crate::sim::{Event, Match};

pub const MOVE_EVERY_TICKS: u64 = 2;
pub const TELEPORT_DURATION_TICKS: i32 = 2;
pub const MIN_MOVE_BLOCKS: f32 = 0.02;
pub const MIN_YAW_CHANGE: f32 = 1.0;
pub const MAX_SPAWNS_PER_TICK: usize = 60;
pub const MAX_PARTICLES_PER_TICK: usize = 120;
pub const LABEL_LIFT: f32 = 0.7;
pub const SHOT_LIFT: f32 = 0.8;
pub const SHOT_STEP: f32 = 1.0;
pub const MAX_SHOT_POINTS: usize = 12;
pub const MIN_AIM_CHANGE: f32 = 8.0;
pub const RING_POINTS: usize = 28;
const BAR_SEGMENTS: usize = 6;
const FLYING_LIFT: f32 = 1.5;

pub const fn mob_material(kind: MobKind) -> Material {
    match kind {
        MobKind::Silverfish => Material::SilverfishSpawnEgg,
        MobKind::Zombie => Material::ZombieSpawnEgg,
        MobKind::Endermite => Material::EndermiteSpawnEgg,
        MobKind::Skeleton => Material::SkeletonSpawnEgg,
        MobKind::Spider => Material::SpiderSpawnEgg,
        MobKind::Creeper => Material::CreeperSpawnEgg,
        MobKind::Blaze => Material::BlazeSpawnEgg,
        MobKind::Witch => Material::WitchSpawnEgg,
        MobKind::GiantSlime | MobKind::MediumSlime => Material::SlimeSpawnEgg,
        MobKind::PiglinBrute => Material::PiglinBruteSpawnEgg,
        MobKind::Enderman => Material::EndermanSpawnEgg,
        MobKind::Ghast => Material::GhastSpawnEgg,
        MobKind::IronGolem => Material::IronGolemSpawnEgg,
        MobKind::Ravager => Material::RavagerSpawnEgg,
        MobKind::Warden => Material::WardenSpawnEgg,
        MobKind::Wither => Material::WitherSpawnEgg,
    }
}

pub fn mob_scale(kind: MobKind) -> f32 {
    match kind {
        MobKind::MediumSlime => 0.6,
        other => 0.6 + 0.15 * f32::from(other.spec().tier),
    }
}

pub const fn tower_block(kind: TowerKind) -> BlockType {
    match kind {
        TowerKind::Archer => BlockType::Dispenser,
        TowerKind::Ice => BlockType::BlueIce,
        TowerKind::Mage => BlockType::AmethystBlock,
        TowerKind::Cannon => BlockType::Tnt,
        TowerKind::Tesla => BlockType::LightningRod,
        TowerKind::Slime => BlockType::SlimeBlock,
        TowerKind::Beacon => BlockType::Beacon,
    }
}

pub fn tower_scale(level: Level) -> f32 {
    match level {
        Level::T1 => 0.6,
        Level::T2 => 0.75,
        Level::T3 => 0.9,
        Level::T4(_) => 1.05,
    }
}

pub fn tower_height(level: Level) -> f32 {
    tower_scale(level) + 0.2
}

pub fn aim_yaw(from: Vec3, to: Vec3) -> f32 {
    (-(to.x - from.x)).atan2(to.z - from.z).to_degrees()
}

pub fn yaw_differs(a: f32, b: f32, threshold: f32) -> bool {
    let mut delta = (a - b) % 360.0;
    if delta > 180.0 {
        delta -= 360.0;
    } else if delta < -180.0 {
        delta += 360.0;
    }
    delta.abs() > threshold
}

pub fn tower_rotation(yaw: f32) -> [f32; 4] {
    let half = (-yaw).to_radians() / 2.0;
    [0.0, half.sin(), 0.0, half.cos()]
}

pub fn tower_translation(level: Level, yaw: f32) -> [f32; 3] {
    let scale = tower_scale(level);
    let angle = (-yaw).to_radians();
    let (sin, cos) = angle.sin_cos();
    let half = scale / 2.0;
    [-(half * cos + half * sin), 0.0, -(-half * sin + half * cos)]
}

pub fn tower_transformation(level: Level, yaw: f32) -> Transformation {
    let scale = tower_scale(level);
    let [tx, ty, tz] = tower_translation(level, yaw);
    let [qx, qy, qz, qw] = tower_rotation(yaw);
    Transformation::default()
        .translation(tx, ty, tz)
        .scale(scale, scale, scale)
        .left_rotation(qx, qy, qz, qw)
}

pub fn hp_label(hp: f32, max_hp: f32) -> String {
    let ratio = if max_hp > 0.0 {
        (hp / max_hp).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((ratio * BAR_SEGMENTS as f32).ceil() as usize).min(BAR_SEGMENTS);
    let color = if ratio > 0.66 {
        "§a"
    } else if ratio > 0.33 {
        "§e"
    } else {
        "§c"
    };
    format!(
        "{color}{}§8{} §f{}",
        "█".repeat(filled),
        "█".repeat(BAR_SEGMENTS - filled),
        hp.max(0.0).ceil() as u32
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobSnapshot {
    pub id: MobId,
    pub lane: LaneId,
    pub kind: MobKind,
    pub position: Vec3,
    pub yaw: f32,
    pub label: String,
}

pub fn snapshots(game: &Match, observed: &[LaneId]) -> Vec<MobSnapshot> {
    game.world()
        .lanes
        .iter()
        .filter(|lane| observed.contains(&lane.id))
        .flat_map(|lane| {
            lane.mobs.iter().map(|mob| {
                let sample = lane.path().sample(mob.progress);
                let mut position = lane.mob_position(mob);
                position.y += mob_scale(mob.kind) / 2.0;
                if mob.kind.spec().flying {
                    position.y += FLYING_LIFT;
                }
                MobSnapshot {
                    id: mob.id,
                    lane: lane.id,
                    kind: mob.kind,
                    position,
                    yaw: sample.yaw,
                    label: hp_label(mob.hp, mob.max_hp),
                }
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobTrack {
    pub position: Vec3,
    pub yaw: f32,
    pub label: String,
}

impl MobTrack {
    pub fn of(snapshot: &MobSnapshot) -> Self {
        Self {
            position: snapshot.position,
            yaw: snapshot.yaw,
            label: snapshot.label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MobOp {
    Spawn(MobSnapshot),
    Move {
        mob: MobId,
        position: Vec3,
        yaw: Option<f32>,
    },
    Relabel {
        mob: MobId,
        text: String,
    },
    Remove {
        mob: MobId,
    },
}

pub fn diff_mobs<'a>(
    known: impl IntoIterator<Item = (MobId, &'a MobTrack)>,
    now: &[MobSnapshot],
    tick: u64,
) -> Vec<MobOp> {
    let known: HashMap<MobId, &MobTrack> = known.into_iter().collect();
    let mut ops = Vec::new();
    let mut spawns = 0;
    for snapshot in now {
        match known.get(&snapshot.id) {
            None => {
                if spawns < MAX_SPAWNS_PER_TICK {
                    spawns += 1;
                    ops.push(MobOp::Spawn(snapshot.clone()));
                }
            }
            Some(track) => {
                if snapshot.label != track.label {
                    ops.push(MobOp::Relabel {
                        mob: snapshot.id,
                        text: snapshot.label.clone(),
                    });
                }
                let moved = snapshot.position.distance(track.position) > MIN_MOVE_BLOCKS;
                if moved && tick.is_multiple_of(MOVE_EVERY_TICKS) {
                    ops.push(MobOp::Move {
                        mob: snapshot.id,
                        position: snapshot.position,
                        yaw: yaw_differs(snapshot.yaw, track.yaw, MIN_YAW_CHANGE)
                            .then_some(snapshot.yaw),
                    });
                }
            }
        }
    }
    let present: Vec<MobId> = now.iter().map(|snapshot| snapshot.id).collect();
    let mut gone: Vec<MobId> = known
        .keys()
        .filter(|id| !present.contains(id))
        .copied()
        .collect();
    gone.sort();
    ops.extend(gone.into_iter().map(|mob| MobOp::Remove { mob }));
    ops
}

#[derive(Debug, Clone, PartialEq)]
pub enum TowerOp {
    Spawn {
        tower: TowerId,
        kind: TowerKind,
        level: Level,
        position: Vec3,
    },
    Upgrade {
        tower: TowerId,
        level: Level,
    },
    Aim {
        tower: TowerId,
        yaw: f32,
    },
    Remove {
        tower: TowerId,
    },
}

pub fn tower_ops(events: &[Event], game: &Match) -> Vec<TowerOp> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TowerPlaced { lane, tower, .. } => {
                let found = game.lane(*lane).tower(*tower)?;
                Some(TowerOp::Spawn {
                    tower: *tower,
                    kind: found.kind,
                    level: found.level,
                    position: found.world_pos,
                })
            }
            Event::TowerUpgraded { tower, level, .. } => Some(TowerOp::Upgrade {
                tower: *tower,
                level: *level,
            }),
            Event::TowerSold { tower, .. } => Some(TowerOp::Remove { tower: *tower }),
            Event::TowerFired {
                tower, from, to, ..
            } => Some(TowerOp::Aim {
                tower: *tower,
                yaw: aim_yaw(*from, *to),
            }),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub particle: Particle,
    pub at: Vec3,
    pub count: i32,
}

pub fn shot_particle(kind: TowerKind) -> Particle {
    match kind {
        TowerKind::Archer => Particle::Crit,
        TowerKind::Ice => Particle::Snowflake,
        TowerKind::Mage => Particle::Flame,
        TowerKind::Cannon => Particle::LargeSmoke,
        TowerKind::Tesla => Particle::ElectricSpark,
        TowerKind::Slime => Particle::ItemSlime,
        TowerKind::Beacon => Particle::EndRod,
    }
}

pub fn line_points(from: Vec3, to: Vec3, step: f32, max: usize) -> Vec<Vec3> {
    let length = from.distance(to);
    if length <= f32::EPSILON {
        return vec![from];
    }
    let segments = ((length / step).ceil() as usize).clamp(1, max);
    (0..=segments)
        .map(|index| from.lerp(to, index as f32 / segments as f32))
        .collect()
}

fn lifted(point: Vec3, lift: f32) -> Vec3 {
    Vec3::new(point.x, point.y + lift, point.z)
}

pub fn shots(events: &[Event], game: &Match) -> Vec<Shot> {
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::TowerFired {
                lane,
                tower,
                from,
                to,
            } => {
                let Some(found) = game.lane(*lane).tower(*tower) else {
                    continue;
                };
                let particle = shot_particle(found.kind);
                out.extend(
                    line_points(
                        lifted(*from, SHOT_LIFT),
                        lifted(*to, SHOT_LIFT),
                        SHOT_STEP,
                        MAX_SHOT_POINTS,
                    )
                    .into_iter()
                    .map(|at| Shot {
                        particle,
                        at,
                        count: 1,
                    }),
                );
            }
            Event::Explosion { at, radius, .. } => {
                out.push(Shot {
                    particle: Particle::Explosion,
                    at: lifted(*at, 0.5),
                    count: 1,
                });
                if *radius >= 2.0 {
                    out.push(Shot {
                        particle: Particle::ExplosionEmitter,
                        at: lifted(*at, 0.5),
                        count: 1,
                    });
                }
            }
            Event::Chain { points, .. } => {
                for pair in points.windows(2) {
                    out.extend(
                        line_points(
                            lifted(pair[0], SHOT_LIFT),
                            lifted(pair[1], SHOT_LIFT),
                            SHOT_STEP / 2.0,
                            MAX_SHOT_POINTS,
                        )
                        .into_iter()
                        .map(|at| Shot {
                            particle: Particle::ElectricSpark,
                            at,
                            count: 2,
                        }),
                    );
                }
            }
            Event::TowerStunned { lane, tower, .. } => {
                if let Some(found) = game.lane(*lane).tower(*tower) {
                    out.push(Shot {
                        particle: Particle::Smoke,
                        at: lifted(found.world_pos, tower_height(found.level)),
                        count: 6,
                    });
                }
            }
            Event::MobBlinked { lane, mob } => {
                let lane = game.lane(*lane);
                if let Some(found) = lane.mobs.iter().find(|candidate| candidate.id == *mob) {
                    out.push(Shot {
                        particle: Particle::Portal,
                        at: lifted(lane.mob_position(found), 0.5),
                        count: 12,
                    });
                }
            }
            _ => {}
        }
    }
    out.truncate(MAX_PARTICLES_PER_TICK);
    out
}

pub fn death_shot(position: Vec3) -> Shot {
    Shot {
        particle: Particle::Poof,
        at: position,
        count: 6,
    }
}

pub fn leak_shot(position: Vec3) -> Shot {
    Shot {
        particle: Particle::AngryVillager,
        at: position,
        count: 3,
    }
}

pub fn ring_points(center: Vec3, radius: f32, count: usize) -> Vec<Vec3> {
    (0..count)
        .map(|index| {
            let angle = index as f32 / count as f32 * std::f32::consts::TAU;
            Vec3::new(
                center.x + radius * angle.cos(),
                center.y,
                center.z + radius * angle.sin(),
            )
        })
        .collect()
}

pub const fn range_color() -> Color {
    Color::rgb(80, 200, 255)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{GameMode, PlayerId};
    use crate::map::Map;
    use crate::sim::Seat;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn lane(index: u8) -> LaneId {
        LaneId::try_from(index).expect("lane")
    }

    fn game() -> Match {
        let seats: Vec<Seat> = (0..4u8)
            .map(|index| Seat {
                player: pid(u128::from(index)),
                lane: lane(index),
            })
            .collect();
        Match::new(GameMode::Solo, 3, &Map::test_map(), &seats).expect("match")
    }

    fn track(snapshot: &MobSnapshot) -> MobTrack {
        MobTrack::of(snapshot)
    }

    fn mobs_after_send(ticks: u64) -> (Match, Vec<MobSnapshot>) {
        let mut game = game();
        game.send(pid(0), MobKind::Silverfish, 1).expect("send");
        for _ in 0..ticks {
            game.tick();
        }
        let all = snapshots(&game, &LaneId::ALL);
        (game, all)
    }

    #[test]
    fn snapshots_cover_only_observed_lanes() {
        let (game, all) = mobs_after_send(2);
        assert_eq!(all.len(), 3);
        let only = snapshots(&game, &[lane(1)]);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].lane, lane(1));
        assert!(snapshots(&game, &[lane(0)]).is_empty());
    }

    #[test]
    fn a_snapshot_follows_the_path_and_the_lateral_offset() {
        let (game, all) = mobs_after_send(5);
        let snapshot = &all[0];
        let lane_ref = game.lane(snapshot.lane);
        let mob = &lane_ref.mobs[0];
        let expected = lane_ref.mob_position(mob);
        assert!((snapshot.position.x - expected.x).abs() < 1e-4);
        assert!((snapshot.position.z - expected.z).abs() < 1e-4);
        assert!(snapshot.position.y > expected.y);
        assert_eq!(snapshot.yaw, lane_ref.path().sample(mob.progress).yaw);
    }

    #[test]
    fn a_known_unchanged_mob_needs_no_operation() {
        let (_, all) = mobs_after_send(4);
        let tracks: Vec<(MobId, MobTrack)> = all
            .iter()
            .map(|snapshot| (snapshot.id, track(snapshot)))
            .collect();
        let known = tracks.iter().map(|(id, track)| (*id, track));
        assert!(diff_mobs(known, &all, 4).is_empty());
    }

    #[test]
    fn a_new_mob_is_spawned_and_a_vanished_one_removed() {
        let (_, all) = mobs_after_send(4);
        let spawns = diff_mobs(std::iter::empty(), &all, 4);
        assert_eq!(spawns.len(), all.len());
        assert!(spawns.iter().all(|op| matches!(op, MobOp::Spawn(_))));
        let stale = MobTrack {
            position: Vec3::default(),
            yaw: 0.0,
            label: String::new(),
        };
        let ops = diff_mobs([(MobId(999), &stale)], &[], 4);
        assert_eq!(ops, vec![MobOp::Remove { mob: MobId(999) }]);
    }

    #[test]
    fn spawns_per_tick_are_capped() {
        let snapshot = MobSnapshot {
            id: MobId(0),
            lane: lane(0),
            kind: MobKind::Zombie,
            position: Vec3::default(),
            yaw: 0.0,
            label: String::new(),
        };
        let crowd: Vec<MobSnapshot> = (0..200)
            .map(|index| MobSnapshot {
                id: MobId(index),
                ..snapshot.clone()
            })
            .collect();
        assert_eq!(
            diff_mobs(std::iter::empty(), &crowd, 0).len(),
            MAX_SPAWNS_PER_TICK
        );
    }

    #[test]
    fn a_moved_mob_is_teleported_only_on_move_ticks() {
        let (_, all) = mobs_after_send(4);
        let mut old = track(&all[0]);
        old.position.x -= 1.0;
        let even = diff_mobs([(all[0].id, &old)], &all[..1], 4);
        assert!(matches!(even.as_slice(), [MobOp::Move { yaw: None, .. }]));
        let odd = diff_mobs([(all[0].id, &old)], &all[..1], 5);
        assert!(odd.is_empty());
    }

    #[test]
    fn a_turn_adds_the_new_yaw_to_the_move() {
        let (_, all) = mobs_after_send(4);
        let mut old = track(&all[0]);
        old.position.x -= 1.0;
        old.yaw += 90.0;
        let ops = diff_mobs([(all[0].id, &old)], &all[..1], 4);
        assert!(matches!(ops.as_slice(), [MobOp::Move { yaw: Some(_), .. }]));
    }

    #[test]
    fn the_label_is_resent_only_when_the_hp_text_changes() {
        let (_, all) = mobs_after_send(4);
        let mut old = track(&all[0]);
        old.label = hp_label(1.0, 100.0);
        let ops = diff_mobs([(all[0].id, &old)], &all[..1], 5);
        assert_eq!(
            ops,
            vec![MobOp::Relabel {
                mob: all[0].id,
                text: all[0].label.clone()
            }]
        );
    }

    #[test]
    fn the_hp_label_shrinks_and_changes_colour() {
        assert_eq!(hp_label(100.0, 100.0), "§a██████§8 §f100");
        assert_eq!(hp_label(50.0, 100.0), "§e███§8███ §f50");
        assert_eq!(hp_label(10.0, 100.0), "§c█§8█████ §f10");
        assert_eq!(hp_label(0.0, 100.0), "§c§8██████ §f0");
        assert_eq!(hp_label(34.2, 100.0), hp_label(34.9, 100.0));
    }

    #[test]
    fn yaw_comparison_wraps_around() {
        assert!(!yaw_differs(179.0, -179.0, 5.0));
        assert!(yaw_differs(0.0, 90.0, 5.0));
        assert!(!yaw_differs(10.0, 10.5, 1.0));
    }

    #[test]
    fn aiming_follows_the_minecraft_yaw_convention() {
        let origin = Vec3::default();
        assert!(aim_yaw(origin, Vec3::new(0.0, 0.0, 5.0)).abs() < 1e-4);
        assert!((aim_yaw(origin, Vec3::new(-5.0, 0.0, 0.0)) - 90.0).abs() < 1e-4);
        assert!((aim_yaw(origin, Vec3::new(5.0, 0.0, 0.0)) + 90.0).abs() < 1e-4);
    }

    #[test]
    fn towers_grow_with_their_level() {
        let scales: Vec<f32> = [
            Level::T1,
            Level::T2,
            Level::T3,
            Level::T4(crate::domain::Branch::A),
        ]
        .into_iter()
        .map(tower_scale)
        .collect();
        assert!(scales.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_tower_stays_centred_on_its_cell_whatever_the_aim() {
        for level in [Level::T1, Level::T3] {
            for yaw in [0.0f32, 37.0, 90.0, 180.0, -120.0] {
                let scale = tower_scale(level);
                let [tx, _, tz] = tower_translation(level, yaw);
                let angle = (-yaw).to_radians();
                let (sin, cos) = angle.sin_cos();
                let (cx, cz) = (scale / 2.0, scale / 2.0);
                let x = cx * cos + cz * sin + tx;
                let z = -cx * sin + cz * cos + tz;
                assert!(x.abs() < 1e-4 && z.abs() < 1e-4, "{level:?} {yaw}: {x} {z}");
            }
        }
    }

    #[test]
    fn the_rotation_is_a_unit_quaternion_about_the_vertical_axis() {
        let [x, y, z, w] = tower_rotation(53.0);
        assert_eq!((x, z), (0.0, 0.0));
        assert!((y * y + w * w - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_transformation_carries_scale_rotation_and_translation() {
        let transformation = tower_transformation(Level::T2, 0.0);
        assert_eq!(transformation.scale, Some([0.75, 0.75, 0.75]));
        assert!(transformation.left_rotation.is_some());
        assert!(transformation.translation.is_some());
    }

    #[test]
    fn every_tower_kind_has_its_own_block_and_particle() {
        let blocks: Vec<BlockType> = TowerKind::ALL.into_iter().map(tower_block).collect();
        for (index, block) in blocks.iter().enumerate() {
            assert!(!blocks[..index].contains(block));
        }
        assert_ne!(
            shot_particle(TowerKind::Archer),
            shot_particle(TowerKind::Tesla)
        );
    }

    #[test]
    fn placing_a_tower_spawns_a_display_at_its_world_position() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        let events = game.drain_events();
        let ops = tower_ops(&events, &game);
        assert_eq!(
            ops,
            vec![TowerOp::Spawn {
                tower,
                kind: TowerKind::Archer,
                level: Level::T1,
                position: game.lane(lane(0)).tower(tower).expect("tower").world_pos,
            }]
        );
    }

    #[test]
    fn selling_removes_the_display_and_upgrading_restyles_it() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        game.drain_events();
        game.sell(pid(0), tower).expect("sell");
        let events = game.drain_events();
        assert_eq!(tower_ops(&events, &game), vec![TowerOp::Remove { tower }]);
        let upgraded = [Event::TowerUpgraded {
            lane: lane(0),
            tower,
            level: Level::T2,
        }];
        assert_eq!(
            tower_ops(&upgraded, &game),
            vec![TowerOp::Upgrade {
                tower,
                level: Level::T2
            }]
        );
    }

    #[test]
    fn a_placement_undone_in_the_same_batch_spawns_nothing() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        game.sell(pid(0), tower).expect("sell");
        let events = game.drain_events();
        let ops = tower_ops(&events, &game);
        assert_eq!(ops, vec![TowerOp::Remove { tower }]);
    }

    #[test]
    fn a_shot_aims_the_tower_and_draws_a_particle_line() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        let from = Vec3::new(2.5, 64.0, 1.5);
        let to = Vec3::new(8.5, 64.0, 1.5);
        let events = [Event::TowerFired {
            lane: lane(0),
            tower,
            from,
            to,
        }];
        let aim = tower_ops(&events, &game);
        assert!(matches!(aim.as_slice(), [TowerOp::Aim { yaw, .. }] if (*yaw + 90.0).abs() < 1e-3));
        let drawn = shots(&events, &game);
        assert!(drawn.len() >= 2);
        assert!(drawn.iter().all(|shot| shot.particle == Particle::Crit));
    }

    #[test]
    fn a_shot_from_an_unknown_tower_draws_nothing() {
        let game = game();
        let events = [Event::TowerFired {
            lane: lane(0),
            tower: TowerId(42),
            from: Vec3::default(),
            to: Vec3::new(3.0, 0.0, 0.0),
        }];
        assert!(shots(&events, &game).is_empty());
    }

    #[test]
    fn explosions_and_chains_draw_particles() {
        let game = game();
        let events = [
            Event::Explosion {
                lane: lane(0),
                at: Vec3::new(3.0, 64.0, 3.0),
                radius: 2.5,
            },
            Event::Chain {
                lane: lane(0),
                points: vec![
                    Vec3::new(1.0, 64.0, 1.0),
                    Vec3::new(3.0, 64.0, 1.0),
                    Vec3::new(5.0, 64.0, 2.0),
                ],
            },
        ];
        let drawn = shots(&events, &game);
        assert!(
            drawn
                .iter()
                .any(|shot| shot.particle == Particle::Explosion)
        );
        assert!(
            drawn
                .iter()
                .any(|shot| shot.particle == Particle::ExplosionEmitter)
        );
        assert!(
            drawn
                .iter()
                .any(|shot| shot.particle == Particle::ElectricSpark)
        );
    }

    #[test]
    fn particles_per_tick_are_capped() {
        let game = game();
        let events: Vec<Event> = (0..100)
            .map(|_| Event::Explosion {
                lane: lane(0),
                at: Vec3::default(),
                radius: 3.0,
            })
            .collect();
        assert_eq!(shots(&events, &game).len(), MAX_PARTICLES_PER_TICK);
    }

    #[test]
    fn a_line_includes_both_ends_and_is_bounded() {
        let from = Vec3::new(0.0, 0.0, 0.0);
        let to = Vec3::new(100.0, 0.0, 0.0);
        let points = line_points(from, to, 1.0, MAX_SHOT_POINTS);
        assert_eq!(points.len(), MAX_SHOT_POINTS + 1);
        assert_eq!(points[0], from);
        assert_eq!(points[MAX_SHOT_POINTS], to);
        assert_eq!(line_points(from, from, 1.0, 5).len(), 1);
    }

    #[test]
    fn the_range_ring_lies_on_the_circle() {
        let center = Vec3::new(10.0, 64.0, 5.0);
        let ring = ring_points(center, 3.5, RING_POINTS);
        assert_eq!(ring.len(), RING_POINTS);
        for point in ring {
            assert!((point.distance_xz(center) - 3.5).abs() < 1e-3);
            assert_eq!(point.y, 64.0);
        }
    }

    #[test]
    fn every_mob_has_a_material_and_a_scale() {
        for kind in MobKind::SENDABLE {
            assert!(mob_scale(kind) > 0.0);
            let _ = mob_material(kind);
        }
        assert!(mob_scale(MobKind::Warden) > mob_scale(MobKind::Silverfish));
        assert!(mob_scale(MobKind::MediumSlime) < mob_scale(MobKind::GiantSlime));
    }
}
