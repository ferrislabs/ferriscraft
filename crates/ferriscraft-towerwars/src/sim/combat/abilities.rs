use crate::domain::{LaneId, MobId, MobKind, SUDDEN_DEATH_HP_STEP, Vec3};
use crate::sim::event::Event;
use crate::sim::lane::{Lane, Mob};
use crate::sim::towers::mobs_in_range;
use crate::sim::world::World;

use super::damage::{DamageKind, apply_damage};

const BLINK_DISTANCE: f32 = 4.0;
const BLINK_HP: f32 = 0.5;
const CREEPER_RADIUS: f32 = 2.5;
const CREEPER_STUN_TICKS: u16 = 20;
const CURSE_RADIUS: f32 = 3.0;
const CURSE_FRACTION: f32 = 0.30;
const SLIME_BIRTH_MARGIN: f32 = 0.01;
const SLIME_SPREAD: f32 = 0.5;

pub(super) fn run(world: &mut World) {
    let mut blinks = Vec::new();
    for lane in world.lanes.iter_mut().filter(|lane| !lane.eliminated) {
        blink_enderman(lane, &mut blinks);
    }
    for (lane, mob) in blinks {
        world.emit(Event::MobBlinked { lane, mob });
    }
}

fn blink_enderman(lane: &mut Lane, blinks: &mut Vec<(LaneId, MobId)>) {
    let length = lane.path().length();
    for mob in &mut lane.mobs {
        if mob.kind == MobKind::Enderman
            && !mob.blinked
            && mob.hp > 0.0
            && mob.hp < BLINK_HP * mob.max_hp
        {
            mob.progress = (mob.progress + BLINK_DISTANCE).min(length);
            mob.blinked = true;
            blinks.push((lane.id, mob.id));
        }
    }
}

pub(super) fn on_death(world: &mut World, lane: LaneId, mob: &Mob) {
    if mob.kind == MobKind::GiantSlime {
        split(world, lane, mob);
    }
    if mob.kind == MobKind::Creeper {
        stun_towers(world, lane, mob);
    }
    if mob.statuses.cursed {
        curse_burst(world, lane, mob);
    }
}

fn split(world: &mut World, lane: LaneId, parent: &Mob) {
    let hp = MobKind::MediumSlime.spec().hp
        * (1.0 + SUDDEN_DEATH_HP_STEP * world.sudden_death_level as f32);
    let first = spawn_child(world, lane, parent, hp);
    let second = spawn_child(world, lane, parent, hp);
    world.emit(Event::MobSplit {
        lane,
        parent: parent.id,
        children: [first, second],
    });
}

fn spawn_child(world: &mut World, lane: LaneId, parent: &Mob, hp: f32) -> MobId {
    let id = MobId(world.next_mob_id);
    world.next_mob_id += 1;
    let offset = world.rng.range(-SLIME_SPREAD, SLIME_SPREAD);
    let ceiling = world.lane(lane).path().length() - SLIME_BIRTH_MARGIN;
    let child = Mob::new(
        id,
        MobKind::MediumSlime,
        parent.sender,
        hp,
        parent.progress.min(ceiling),
        offset,
    );
    world.lane_mut(lane).mobs.push(child);
    id
}

fn stun_towers(world: &mut World, lane: LaneId, mob: &Mob) {
    let at = world.lane(lane).mob_position(mob);
    let stunned: Vec<_> = world
        .lane_mut(lane)
        .towers_mut()
        .iter_mut()
        .filter(|tower| tower.world_pos.distance_xz(at) <= CREEPER_RADIUS)
        .map(|tower| {
            tower.stun_remaining = tower.stun_remaining.max(CREEPER_STUN_TICKS);
            tower.id
        })
        .collect();
    for tower in stunned {
        world.emit(Event::TowerStunned {
            lane,
            tower,
            ticks: CREEPER_STUN_TICKS,
        });
    }
}

fn curse_burst(world: &mut World, lane: LaneId, mob: &Mob) {
    let at: Vec3 = world.lane(lane).mob_position(mob);
    let victims = mobs_in_range(world.lane(lane), at, CURSE_RADIUS);
    let damage = CURSE_FRACTION * mob.max_hp;
    for index in victims {
        let victim = &mut world.lane_mut(lane).mobs[index];
        apply_damage(victim, damage, DamageKind::Magic, mob.last_hit_by);
    }
    world.emit(Event::Explosion {
        lane,
        at,
        radius: CURSE_RADIUS,
    });
}
