use crate::domain::{
    ArmorShred, Burn, MAX_BURN_STACKS, MobKind, Slow, TICKS_PER_SECOND, Vec3, Vulnerability,
};
use crate::sim::lane::{Lane, Mob, Pool};
use crate::sim::world::World;

use super::damage::{DamageKind, apply_damage};

const WITCH_RANGE: f32 = 3.0;
const WITCH_HEAL_PER_SECOND: f32 = 30.0;
const WITHER_REGEN_PER_SECOND: f32 = 150.0;
const POOL_SHRED_TICKS: u16 = 2;

pub(super) fn run(world: &mut World) {
    let sudden_death = world.sudden_death_level > 0;
    for lane in world.lanes.iter_mut().filter(|lane| !lane.eliminated) {
        burn_mobs(lane);
        tick_pools(lane);
        if !sudden_death {
            heal_mobs(lane);
        }
        decay_mobs(lane);
    }
}

pub(super) fn apply_slow(mob: &mut Mob, percent: f32, ticks: u16) {
    mob.statuses.slow = Some(match mob.statuses.slow {
        Some(current) => Slow {
            percent: current.percent.max(percent),
            remaining: current.remaining.max(ticks),
        },
        None => Slow {
            percent,
            remaining: ticks,
        },
    });
}

pub(super) fn apply_shred(mob: &mut Mob, percent: f32, ticks: u16) {
    mob.statuses.shred = Some(match mob.statuses.shred {
        Some(current) => ArmorShred {
            percent: current.percent.max(percent),
            remaining: current.remaining.max(ticks),
        },
        None => ArmorShred {
            percent,
            remaining: ticks,
        },
    });
}

pub(super) fn apply_vulnerability(mob: &mut Mob, percent: f32, ticks: u16) {
    mob.statuses.vulnerability = Some(match mob.statuses.vulnerability {
        Some(current) => Vulnerability {
            percent: current.percent.max(percent),
            remaining: current.remaining.max(ticks),
        },
        None => Vulnerability {
            percent,
            remaining: ticks,
        },
    });
}

pub(super) fn refresh_burn(mob: &mut Mob, dps: f32, ticks: u16) {
    if mob.kind.spec().fire_immune {
        return;
    }
    match mob.statuses.burns.first_mut() {
        Some(current) => merge_burn(current, dps, ticks),
        None => mob.statuses.burns.push(Burn {
            dps,
            remaining: ticks,
        }),
    }
}

pub(super) fn stack_burn(mob: &mut Mob, dps: f32, ticks: u16) {
    if mob.kind.spec().fire_immune {
        return;
    }
    if mob.statuses.burns.len() < MAX_BURN_STACKS {
        mob.statuses.burns.push(Burn {
            dps,
            remaining: ticks,
        });
        return;
    }
    if let Some(weakest) = mob
        .statuses
        .burns
        .iter_mut()
        .min_by(|a, b| a.dps.total_cmp(&b.dps))
    {
        merge_burn(weakest, dps, ticks);
    }
}

fn merge_burn(current: &mut Burn, dps: f32, ticks: u16) {
    current.dps = current.dps.max(dps);
    current.remaining = current.remaining.max(ticks);
}

fn burn_mobs(lane: &mut Lane) {
    for mob in lane.mobs.iter_mut().filter(|mob| mob.hp > 0.0) {
        let dps: f32 = mob.statuses.burns.iter().map(|burn| burn.dps).sum();
        if dps > 0.0 {
            apply_damage(mob, per_tick(dps), DamageKind::Magic, None);
        }
    }
}

fn per_tick(per_second: f32) -> f32 {
    per_second / TICKS_PER_SECOND as f32
}

fn positions(lane: &Lane) -> Vec<Vec3> {
    lane.mobs.iter().map(|mob| lane.mob_position(mob)).collect()
}

fn tick_pools(lane: &mut Lane) {
    let mut pools = std::mem::take(&mut lane.pools);
    let places = positions(lane);
    for pool in &pools {
        corrode_inside(lane, &places, pool);
    }
    pools.retain_mut(|pool| {
        pool.remaining = pool.remaining.saturating_sub(1);
        pool.remaining > 0
    });
    lane.pools = pools;
}

fn corrode_inside(lane: &mut Lane, places: &[Vec3], pool: &Pool) {
    for (mob, place) in lane.mobs.iter_mut().zip(places) {
        if mob.hp > 0.0 && pool_affects(mob.kind) && pool.center.distance_xz(*place) <= pool.radius
        {
            apply_damage(
                mob,
                per_tick(pool.dps),
                DamageKind::Physical,
                Some(pool.owner),
            );
            apply_shred(mob, pool.armor, POOL_SHRED_TICKS);
        }
    }
}

fn pool_affects(kind: MobKind) -> bool {
    !kind.spec().flying && kind != MobKind::Ravager
}

fn heal_mobs(lane: &mut Lane) {
    let places = positions(lane);
    let witches: Vec<usize> = lane
        .mobs
        .iter()
        .enumerate()
        .filter(|(_, mob)| mob.kind == MobKind::Witch && mob.hp > 0.0)
        .map(|(index, _)| index)
        .collect();
    for (index, mob) in lane.mobs.iter_mut().enumerate() {
        if mob.hp <= 0.0 {
            continue;
        }
        let mut amount = 0.0;
        if mob.kind == MobKind::Wither {
            amount += per_tick(WITHER_REGEN_PER_SECOND);
        }
        if witch_nearby(&witches, &places, index) {
            amount += per_tick(WITCH_HEAL_PER_SECOND);
        }
        mob.hp = (mob.hp + amount).min(mob.max_hp);
    }
}

fn witch_nearby(witches: &[usize], places: &[Vec3], index: usize) -> bool {
    witches
        .iter()
        .any(|witch| *witch != index && places[*witch].distance_xz(places[index]) <= WITCH_RANGE)
}

fn decay_mobs(lane: &mut Lane) {
    for mob in &mut lane.mobs {
        let statuses = &mut mob.statuses;
        decay(&mut statuses.slow, |slow| &mut slow.remaining);
        decay(&mut statuses.shred, |shred| &mut shred.remaining);
        decay(&mut statuses.vulnerability, |vulnerability| {
            &mut vulnerability.remaining
        });
        statuses.burns.retain_mut(|burn| {
            burn.remaining = burn.remaining.saturating_sub(1);
            burn.remaining > 0
        });
        statuses.stun_remaining = statuses.stun_remaining.saturating_sub(1);
        statuses.freeze_immunity_remaining = statuses.freeze_immunity_remaining.saturating_sub(1);
    }
}

fn decay<T>(slot: &mut Option<T>, remaining: impl Fn(&mut T) -> &mut u16) {
    let expired = slot.as_mut().is_some_and(|value| {
        let ticks = remaining(value);
        *ticks = ticks.saturating_sub(1);
        *ticks == 0
    });
    if expired {
        *slot = None;
    }
}
