use std::f32::consts::TAU;

use crate::domain::{TowerEffect, Vec3};
use crate::sim::event::Event;
use crate::sim::lane::{Lane, Mob, Pool};
use crate::sim::towers::mobs_in_range;
use crate::sim::world::World;

use super::attacks::Attack;
use super::damage::{DamageKind, apply_damage};
use super::statuses::{apply_shred, apply_slow, apply_vulnerability, refresh_burn, stack_burn};

const FLYING_BONUS: f32 = 1.25;

pub(super) fn apply(
    world: &mut World,
    attack: &Attack,
    effect: TowerEffect,
    target: usize,
    shots: u32,
) {
    match effect {
        TowerEffect::None => archer(world, attack, target, None),
        TowerEffect::Crit { chance, multiplier } => {
            archer(world, attack, target, Some((chance, multiplier)));
        }
        TowerEffect::Slow {
            percent,
            ticks,
            vulnerability,
        } => ice(world, attack, (percent, ticks, vulnerability)),
        TowerEffect::Burn { dps, ticks } => mage(world, attack, target, dps, ticks),
        TowerEffect::Pyro { radius, dps, ticks } => {
            pyromancer(world, attack, target, radius, (dps, ticks));
        }
        TowerEffect::Curse { .. } => necromancer(world, attack, target),
        TowerEffect::Splash { radius } => splash(world, attack, target, radius),
        TowerEffect::Cluster {
            radius,
            submunitions,
            damage,
            sub_radius,
        } => cluster(
            world,
            attack,
            target,
            (radius, submunitions, damage, sub_radius),
        ),
        TowerEffect::Chain {
            targets,
            bounce,
            stun_every,
        } => chain(
            world,
            attack,
            target,
            (targets, bounce),
            stun_every.map(|(every, ticks)| (every, ticks, shots)),
        ),
        TowerEffect::Corrode { armor, ticks } => corrode(world, attack, target, armor, ticks),
        TowerEffect::Pool {
            radius,
            ticks,
            dps,
            armor,
        } => pool(world, attack, target, (radius, ticks, dps, armor)),
        TowerEffect::Mutator { fraction } => mutator(world, attack, target, fraction),
        TowerEffect::Aura { .. } | TowerEffect::Beacon { .. } | TowerEffect::GoldMine { .. } => {}
    }
}

fn dodged(world: &mut World, attack: &Attack, target: usize) -> bool {
    let dodge = world.lanes[attack.lane_index].mobs[target]
        .kind
        .spec()
        .dodge;
    dodge > 0.0 && world.rng.chance(dodge)
}

fn hit(world: &mut World, attack: &Attack, target: usize, amount: f32, kind: DamageKind) {
    let mob = &mut world.lanes[attack.lane_index].mobs[target];
    let flying = attack.flying_bonus && mob.kind.spec().flying;
    let scaled = if flying {
        amount * FLYING_BONUS
    } else {
        amount
    };
    apply_damage(mob, scaled, kind, Some(attack.owner));
}

fn lane_of<'a>(world: &'a World, attack: &Attack) -> &'a Lane {
    &world.lanes[attack.lane_index]
}

fn position_of(world: &World, attack: &Attack, target: usize) -> Vec3 {
    let lane = lane_of(world, attack);
    lane.mob_position(&lane.mobs[target])
}

fn archer(world: &mut World, attack: &Attack, target: usize, crit: Option<(f32, f32)>) {
    if dodged(world, attack, target) {
        return;
    }
    let mut amount = attack.damage;
    if let Some((chance, multiplier)) = crit
        && world.rng.chance(chance)
    {
        amount *= multiplier;
    }
    hit(world, attack, target, amount, DamageKind::Physical);
}

fn ice(world: &mut World, attack: &Attack, (percent, ticks, vulnerability): (f32, u16, f32)) {
    let victims = mobs_in_range(lane_of(world, attack), attack.origin, attack.range);
    for index in victims {
        hit(world, attack, index, attack.damage, DamageKind::Physical);
        let mob = &mut world.lanes[attack.lane_index].mobs[index];
        apply_slow(mob, percent, ticks);
        if vulnerability > 0.0 {
            apply_vulnerability(mob, vulnerability, ticks);
        }
    }
}

fn mage(world: &mut World, attack: &Attack, target: usize, dps: f32, ticks: u16) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Magic);
    refresh_burn(&mut world.lanes[attack.lane_index].mobs[target], dps, ticks);
}

fn pyromancer(
    world: &mut World,
    attack: &Attack,
    target: usize,
    radius: f32,
    (dps, ticks): (f32, u16),
) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Magic);
    let center = position_of(world, attack, target);
    for index in mobs_in_range(lane_of(world, attack), center, radius) {
        stack_burn(&mut world.lanes[attack.lane_index].mobs[index], dps, ticks);
    }
}

fn necromancer(world: &mut World, attack: &Attack, target: usize) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Magic);
    world.lanes[attack.lane_index].mobs[target].statuses.cursed = true;
}

fn impact(world: &mut World, attack: &Attack, at: Vec3, radius: f32, damage: f32) {
    let lane = lane_of(world, attack);
    let victims: Vec<usize> = mobs_in_range(lane, at, radius)
        .into_iter()
        .filter(|index| !lane.mobs[*index].kind.spec().flying)
        .collect();
    for index in victims {
        hit(world, attack, index, damage, DamageKind::Physical);
    }
    world.emit(Event::Explosion {
        lane: attack.lane,
        at,
        radius,
    });
}

fn splash(world: &mut World, attack: &Attack, target: usize, radius: f32) {
    let at = position_of(world, attack, target);
    impact(world, attack, at, radius, attack.damage);
}

fn cluster(
    world: &mut World,
    attack: &Attack,
    target: usize,
    (radius, submunitions, damage, sub_radius): (f32, u8, f32, f32),
) {
    let center = position_of(world, attack, target);
    impact(world, attack, center, radius, attack.damage);
    for _ in 0..submunitions {
        let angle = world.rng.range(0.0, TAU);
        let distance = world.rng.range(0.0, radius);
        let at = Vec3::new(
            center.x + angle.cos() * distance,
            center.y,
            center.z + angle.sin() * distance,
        );
        impact(world, attack, at, sub_radius, damage * attack.multiplier);
    }
}

fn chain_targets(lane: &Lane, first: usize, limit: usize, bounce: f32) -> Vec<usize> {
    let mut hits = vec![first];
    while hits.len() < limit {
        let last = lane.mob_position(&lane.mobs[hits[hits.len() - 1]]);
        let next = mobs_in_range(lane, last, bounce)
            .into_iter()
            .filter(|index| !hits.contains(index))
            .min_by(|a, b| {
                let da = last.distance_xz(lane.mob_position(&lane.mobs[*a]));
                let db = last.distance_xz(lane.mob_position(&lane.mobs[*b]));
                da.total_cmp(&db)
            });
        match next {
            Some(index) => hits.push(index),
            None => break,
        }
    }
    hits
}

fn chain(
    world: &mut World,
    attack: &Attack,
    target: usize,
    (targets, bounce): (u8, f32),
    overload: Option<(u8, u16, u32)>,
) {
    let lane = lane_of(world, attack);
    let hits = chain_targets(lane, target, usize::from(targets), bounce);
    let mut points = vec![attack.origin];
    points.extend(
        hits.iter()
            .map(|index| lane.mob_position(&lane.mobs[*index])),
    );
    for index in hits {
        hit(world, attack, index, attack.damage, DamageKind::Physical);
    }
    world.emit(Event::Chain {
        lane: attack.lane,
        points,
    });
    if let Some((every, ticks, shots)) = overload
        && shots.is_multiple_of(u32::from(every))
    {
        stun(&mut world.lanes[attack.lane_index].mobs[target], ticks);
    }
}

fn stun(mob: &mut Mob, ticks: u16) {
    if !mob.kind.spec().stun_immune {
        mob.statuses.stun_remaining = mob.statuses.stun_remaining.max(ticks);
    }
}

fn corrode(world: &mut World, attack: &Attack, target: usize, armor: f32, ticks: u16) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Physical);
    apply_shred(
        &mut world.lanes[attack.lane_index].mobs[target],
        armor,
        ticks,
    );
}

fn pool(
    world: &mut World,
    attack: &Attack,
    target: usize,
    (radius, ticks, dps, armor): (f32, u16, f32, f32),
) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Physical);
    apply_shred(
        &mut world.lanes[attack.lane_index].mobs[target],
        armor,
        ticks,
    );
    let center = position_of(world, attack, target);
    world.lanes[attack.lane_index].pools.push(Pool {
        center,
        radius,
        remaining: ticks,
        dps,
        armor,
        owner: attack.owner,
    });
}

fn mutator(world: &mut World, attack: &Attack, target: usize, fraction: f32) {
    if dodged(world, attack, target) {
        return;
    }
    hit(world, attack, target, attack.damage, DamageKind::Physical);
    let hp = world.lanes[attack.lane_index].mobs[target].hp;
    if hp > 0.0 {
        hit(world, attack, target, fraction * hp, DamageKind::Physical);
    }
}
