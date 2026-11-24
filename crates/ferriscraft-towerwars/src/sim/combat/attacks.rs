use crate::domain::{
    LaneId, MAX_BEACONS_PER_TOWER, MobKind, PlayerId, TICKS_PER_SECOND, TowerEffect, TowerKind,
    TowerStats, Vec3,
};
use crate::sim::event::Event;
use crate::sim::lane::{Lane, Tower};
use crate::sim::towers::{mobs_in_range, pick_target};
use crate::sim::world::World;

use super::damage::{DamageKind, apply_damage};
use super::effects;

const WARDEN_RANGE: f32 = 4.0;
const WARDEN_SPEED_FACTOR: f32 = 0.8;
const FREEZE_PERIOD: u64 = TICKS_PER_SECOND as u64;

pub(super) struct Attack {
    pub lane: LaneId,
    pub lane_index: usize,
    pub owner: PlayerId,
    pub origin: Vec3,
    pub range: f32,
    pub multiplier: f32,
    pub damage: f32,
    pub flying_bonus: bool,
}

#[derive(Debug, Clone, Copy)]
struct Aura {
    speed: f32,
    range: f32,
    damage: f32,
}

#[derive(Debug, Clone, Copy)]
struct Buffs {
    speed: f32,
    range: f32,
    damage: f32,
}

pub(super) fn run(world: &mut World) {
    for lane_index in 0..world.lanes.len() {
        if world.lanes[lane_index].eliminated {
            continue;
        }
        for tower_index in 0..world.lanes[lane_index].towers().len() {
            step_tower(world, lane_index, tower_index);
        }
    }
}

fn step_tower(world: &mut World, lane_index: usize, tower_index: usize) {
    let tower = world.lanes[lane_index].towers()[tower_index].clone();
    if tower.stun_remaining > 0 {
        world.lanes[lane_index].towers_mut()[tower_index].stun_remaining -= 1;
        return;
    }
    let stats = tower.kind.stats(tower.level);
    match stats.effect {
        TowerEffect::Beacon { .. } | TowerEffect::GoldMine { .. } => {}
        TowerEffect::Aura {
            dps,
            freeze_chance,
            freeze_ticks,
            freeze_immunity_ticks,
        } => {
            let freeze = Freeze {
                chance: freeze_chance,
                ticks: freeze_ticks,
                immunity: freeze_immunity_ticks,
            };
            blizzard(world, lane_index, &tower, &stats, dps, &freeze);
        }
        _ => charge(world, lane_index, tower_index, &tower, &stats),
    }
}

fn charge(
    world: &mut World,
    lane_index: usize,
    tower_index: usize,
    tower: &Tower,
    stats: &TowerStats,
) {
    let cooldown = tower.cooldown_ticks.saturating_sub(1);
    world.lanes[lane_index].towers_mut()[tower_index].cooldown_ticks = cooldown;
    if cooldown == 0 {
        try_fire(world, lane_index, tower_index, tower, stats);
    }
}

fn try_fire(
    world: &mut World,
    lane_index: usize,
    tower_index: usize,
    tower: &Tower,
    stats: &TowerStats,
) {
    let lane = &world.lanes[lane_index];
    let buffs = buffs_for(lane, tower);
    let range = stats.range * (1.0 + buffs.range);
    let Some(target) = pick_target(lane, tower.world_pos, range, tower.priority) else {
        return;
    };
    let to = lane.mob_position(&lane.mobs[target]);
    let attack = Attack {
        lane: lane.id,
        lane_index,
        owner: tower.owner,
        origin: tower.world_pos,
        range,
        multiplier: 1.0 + buffs.damage,
        damage: stats.damage * (1.0 + buffs.damage),
        flying_bonus: matches!(tower.kind, TowerKind::Archer | TowerKind::Tesla),
    };
    let slot = &mut world.lanes[lane_index].towers_mut()[tower_index];
    slot.cooldown_ticks = cooldown_for(stats.cooldown_ticks, buffs.speed);
    slot.shots = slot.shots.saturating_add(1);
    let shots = slot.shots;
    world.emit(Event::TowerFired {
        lane: attack.lane,
        tower: tower.id,
        from: tower.world_pos,
        to,
    });
    effects::apply(world, &attack, stats.effect, target, shots);
}

fn cooldown_for(base: u16, speed: f32) -> u16 {
    ((f32::from(base) / speed).round() as u16).max(1)
}

fn buffs_for(lane: &Lane, tower: &Tower) -> Buffs {
    let mut auras: Vec<Aura> = lane
        .towers()
        .iter()
        .filter(|other| other.id != tower.id)
        .filter_map(|other| aura_of(other, tower))
        .collect();
    auras.sort_by(|a, b| b.speed.total_cmp(&a.speed));
    auras.truncate(MAX_BEACONS_PER_TOWER);
    let sum = |pick: fn(&Aura) -> f32| auras.iter().map(pick).sum::<f32>();
    Buffs {
        speed: (1.0 + sum(|aura| aura.speed)) * warden_factor(lane, tower),
        range: sum(|aura| aura.range),
        damage: sum(|aura| aura.damage),
    }
}

fn aura_of(source: &Tower, target: &Tower) -> Option<Aura> {
    let (aura, radius) = match source.kind.stats(source.level).effect {
        TowerEffect::Beacon {
            attack_speed,
            range,
            damage,
            radius,
        } => (
            Aura {
                speed: attack_speed,
                range,
                damage,
            },
            radius,
        ),
        TowerEffect::GoldMine {
            attack_speed,
            radius,
            ..
        } => (
            Aura {
                speed: attack_speed,
                range: 0.0,
                damage: 0.0,
            },
            radius,
        ),
        _ => return None,
    };
    (source.world_pos.distance_xz(target.world_pos) <= radius).then_some(aura)
}

fn warden_factor(lane: &Lane, tower: &Tower) -> f32 {
    let near = lane.mobs.iter().any(|mob| {
        mob.kind == MobKind::Warden
            && mob.hp > 0.0
            && tower.world_pos.distance_xz(lane.mob_position(mob)) <= WARDEN_RANGE
    });
    if near { WARDEN_SPEED_FACTOR } else { 1.0 }
}

struct Freeze {
    chance: f32,
    ticks: u16,
    immunity: u16,
}

fn blizzard(
    world: &mut World,
    lane_index: usize,
    tower: &Tower,
    stats: &TowerStats,
    dps: f32,
    freeze: &Freeze,
) {
    let lane = &world.lanes[lane_index];
    let buffs = buffs_for(lane, tower);
    let range = stats.range * (1.0 + buffs.range);
    let victims = mobs_in_range(lane, tower.world_pos, range);
    let rolling = world.elapsed.is_multiple_of(FREEZE_PERIOD);
    let damage = dps / TICKS_PER_SECOND as f32 * (1.0 + buffs.damage);
    for index in victims {
        apply_damage(
            &mut world.lanes[lane_index].mobs[index],
            damage,
            DamageKind::Physical,
            Some(tower.owner),
        );
        if rolling {
            roll_freeze(world, lane_index, index, freeze);
        }
    }
}

fn roll_freeze(world: &mut World, lane_index: usize, index: usize, freeze: &Freeze) {
    let mob = &world.lanes[lane_index].mobs[index];
    if mob.statuses.freeze_immunity_remaining > 0 || mob.kind.spec().stun_immune {
        return;
    }
    if world.rng.chance(freeze.chance) {
        let statuses = &mut world.lanes[lane_index].mobs[index].statuses;
        statuses.stun_remaining = statuses.stun_remaining.max(freeze.ticks);
        statuses.freeze_immunity_remaining = freeze.immunity;
    }
}
