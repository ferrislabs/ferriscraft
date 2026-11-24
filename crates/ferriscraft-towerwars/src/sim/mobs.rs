use crate::domain::{
    BOUNTY_PERCENT, MobKind, PlayerId, SUDDEN_DEATH_SPEED_STEP, SUDDEN_DEATH_STEP_TICKS,
    SUDDEN_DEATH_TICKS, TICKS_PER_SECOND,
};

use super::combat;
use super::event::Event;
use super::lane::{Lane, Mob};
use super::world::World;

const PIGLIN_RAGE_HP: f32 = 0.4;
const PIGLIN_RAGE_SPEED: f32 = 1.3;
const WITHER_HEAL: u16 = 2;

pub(crate) fn update_sudden_death(world: &mut World) {
    let level = if world.elapsed < SUDDEN_DEATH_TICKS {
        0
    } else {
        1 + ((world.elapsed - SUDDEN_DEATH_TICKS) / SUDDEN_DEATH_STEP_TICKS) as u32
    };
    if level > world.sudden_death_level {
        world.sudden_death_level = level;
        world.emit(Event::SuddenDeath { level });
    }
}

pub(crate) fn advance(world: &mut World) {
    let level = world.sudden_death_level;
    for lane in world.lanes.iter_mut().filter(|lane| !lane.eliminated) {
        for mob in &mut lane.mobs {
            if mob.statuses.stun_remaining == 0 {
                mob.progress += step_of(mob, level);
            }
        }
    }
}

fn step_of(mob: &Mob, level: u32) -> f32 {
    let spec = mob.kind.spec();
    let mut step = spec.speed / TICKS_PER_SECOND as f32;
    step *= 1.0 + SUDDEN_DEATH_SPEED_STEP * level as f32;
    let slow = mob
        .statuses
        .slow
        .map_or(0.0, |slow| slow.percent * (1.0 - spec.slow_resist));
    step *= 1.0 - slow;
    if mob.kind == MobKind::PiglinBrute && mob.hp < PIGLIN_RAGE_HP * mob.max_hp {
        step *= PIGLIN_RAGE_SPEED;
    }
    step
}

pub(crate) fn reap(world: &mut World) {
    while reap_pass(world) {}
}

fn reap_pass(world: &mut World) -> bool {
    let mut reaped = false;
    for index in 0..world.lanes.len() {
        let lane = &mut world.lanes[index];
        let id = lane.id;
        let (dead, alive): (Vec<Mob>, Vec<Mob>) = std::mem::take(&mut lane.mobs)
            .into_iter()
            .partition(|mob| mob.hp <= 0.0);
        lane.mobs = alive;
        reaped |= !dead.is_empty();
        for mob in dead {
            let bounty = mob.kind.spec().cost * BOUNTY_PERCENT / 100;
            pay_bounty(world, index, &mob, bounty);
            world.emit(Event::MobKilled {
                lane: id,
                mob: mob.id,
                kind: mob.kind,
                bounty,
            });
            combat::on_death(world, id, &mob);
        }
    }
    reaped
}

fn pay_bounty(world: &mut World, lane_index: usize, mob: &Mob, bounty: u32) {
    let present = |id: &PlayerId| world.player(*id).is_some_and(|state| !state.left);
    let owners = &world.lanes[lane_index].owners;
    let receiver = mob
        .last_hit_by
        .filter(|hitter| owners.contains(hitter) && present(hitter))
        .or_else(|| owners.iter().copied().find(present));
    let state = receiver.and_then(|id| world.player_mut(id));
    if let Some(state) = state {
        state.gold = state.gold.saturating_add(bounty);
    }
}

pub(crate) fn resolve_arrivals(world: &mut World) {
    for index in 0..world.lanes.len() {
        if world.lanes[index].eliminated {
            continue;
        }
        let arrived = take_arrived(&mut world.lanes[index]);
        for mob in arrived {
            leak(world, index, &mob);
        }
    }
}

fn take_arrived(lane: &mut Lane) -> Vec<Mob> {
    let length = lane.path().length();
    let (arrived, rest): (Vec<Mob>, Vec<Mob>) = std::mem::take(&mut lane.mobs)
        .into_iter()
        .partition(|mob| mob.hp > 0.0 && mob.progress >= length);
    lane.mobs = rest;
    arrived
}

fn leak(world: &mut World, lane_index: usize, mob: &Mob) {
    let spec = mob.kind.spec();
    let lane = &mut world.lanes[lane_index];
    let id = lane.id;
    lane.hp.damage(spec.damage);
    let hp = lane.hp.current();
    world.emit(Event::MobLeaked {
        lane: id,
        mob: mob.id,
        kind: mob.kind,
        damage: spec.damage,
    });
    world.emit(Event::LaneDamaged { lane: id, hp });
    if mob.kind == MobKind::Wither && world.sudden_death_level == 0 {
        heal_sender(world, mob);
    }
}

fn heal_sender(world: &mut World, mob: &Mob) {
    let Some(lane_id) = world.player(mob.sender).map(|state| state.lane) else {
        return;
    };
    let lane = world.lane_mut(lane_id);
    if lane.eliminated || lane.hp.is_dead() {
        return;
    }
    lane.hp.heal_to_cap(WITHER_HEAL);
    let hp = lane.hp.current();
    world.emit(Event::LaneHealed { lane: lane_id, hp });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::domain::{GameMode, LANE_COUNT, LaneHp, LaneId, MobId, MobKind, PlayerId, Vec3};
    use crate::map::Path;
    use crate::sim::event::Event;
    use crate::sim::lane::{Lane, Mob};
    use crate::sim::rng::SplitMix64;
    use crate::sim::world::{Phase, PlayerState, World};

    use super::{advance, reap, resolve_arrivals};

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn lane_id(index: u8) -> LaneId {
        LaneId::try_from(index).unwrap()
    }

    fn world() -> World {
        let lanes = (0..LANE_COUNT as u8)
            .map(|index| {
                let path =
                    Path::new(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)]).unwrap();
                let owners = if index < 2 {
                    vec![pid(u128::from(index))]
                } else {
                    vec![]
                };
                Lane::new(
                    lane_id(index),
                    owners,
                    LaneHp::full(20),
                    Arc::new(path),
                    Vec::new(),
                )
            })
            .collect();
        World {
            mode: GameMode::Solo,
            players: vec![
                PlayerState::new(pid(0), lane_id(0)),
                PlayerState::new(pid(1), lane_id(1)),
            ],
            lanes,
            elapsed: 1,
            rng: SplitMix64::new(1),
            next_mob_id: 0,
            next_tower_id: 0,
            pending: Vec::new(),
            events: Vec::new(),
            phase: Phase::Running,
            sudden_death_level: 0,
        }
    }

    fn mob(id: u32, kind: MobKind, sender: u128) -> Mob {
        Mob::new(MobId(id), kind, pid(sender), kind.spec().hp, 0.0, 0.0)
    }

    #[test]
    fn a_killed_mob_pays_its_bounty_to_the_last_hitter() {
        let mut world = world();
        let mut dead = mob(0, MobKind::Zombie, 1);
        dead.hp = 0.0;
        dead.last_hit_by = Some(pid(0));
        world.lanes[0].mobs.push(dead);
        reap(&mut world);
        assert!(world.lanes[0].mobs.is_empty());
        assert_eq!(world.player(pid(0)).unwrap().gold.get(), 103);
        assert!(world.events.contains(&Event::MobKilled {
            lane: lane_id(0),
            mob: MobId(0),
            kind: MobKind::Zombie,
            bounty: 3,
        }));
    }

    #[test]
    fn the_bounty_falls_back_to_the_first_owner() {
        let mut world = world();
        let mut dead = mob(0, MobKind::Silverfish, 1);
        dead.hp = -1.0;
        dead.last_hit_by = Some(pid(1));
        world.lanes[0].mobs.push(dead);
        reap(&mut world);
        assert_eq!(world.player(pid(0)).unwrap().gold.get(), 101);
        assert_eq!(world.player(pid(1)).unwrap().gold.get(), 100);
    }

    #[test]
    fn a_medium_slime_pays_nothing_and_living_mobs_stay_in_order() {
        let mut world = world();
        let mut dead = mob(1, MobKind::MediumSlime, 1);
        dead.hp = 0.0;
        world.lanes[0].mobs.push(mob(0, MobKind::Zombie, 1));
        world.lanes[0].mobs.push(dead);
        world.lanes[0].mobs.push(mob(2, MobKind::Zombie, 1));
        reap(&mut world);
        let ids: Vec<u32> = world.lanes[0].mobs.iter().map(|m| m.id.0).collect();
        assert_eq!(ids, vec![0, 2]);
        assert_eq!(world.player(pid(0)).unwrap().gold.get(), 100);
    }

    #[test]
    fn an_arriving_mob_damages_the_lane_and_pays_nothing() {
        let mut world = world();
        let mut leaker = mob(0, MobKind::Silverfish, 1);
        leaker.progress = 10.0;
        world.lanes[0].mobs.push(leaker);
        resolve_arrivals(&mut world);
        assert!(world.lanes[0].mobs.is_empty());
        assert_eq!(world.lanes[0].hp.current(), 19);
        assert_eq!(world.player(pid(0)).unwrap().gold.get(), 100);
        assert!(world.events.contains(&Event::LaneDamaged {
            lane: lane_id(0),
            hp: 19
        }));
    }

    #[test]
    fn a_wither_heals_its_sender_lane_up_to_the_cap() {
        let mut world = world();
        world.lanes[1].hp.damage(5);
        let mut wither = mob(0, MobKind::Wither, 1);
        wither.progress = 10.0;
        world.lanes[0].mobs.push(wither);
        resolve_arrivals(&mut world);
        assert_eq!(world.lanes[0].hp.current(), 5);
        assert_eq!(world.lanes[1].hp.current(), 17);
        let mut second = mob(1, MobKind::Wither, 1);
        second.progress = 10.0;
        world.lanes[1].hp.heal_to_cap(5);
        world.lanes[0].hp = LaneHp::full(20);
        world.lanes[0].mobs.push(second);
        resolve_arrivals(&mut world);
        assert_eq!(world.lanes[1].hp.current(), 20);
    }

    #[test]
    fn a_wither_does_not_heal_during_sudden_death() {
        let mut world = world();
        world.sudden_death_level = 1;
        world.lanes[1].hp.damage(5);
        let mut wither = mob(0, MobKind::Wither, 1);
        wither.progress = 10.0;
        world.lanes[0].mobs.push(wither);
        resolve_arrivals(&mut world);
        assert_eq!(world.lanes[1].hp.current(), 15);
        assert!(
            !world
                .events
                .iter()
                .any(|e| matches!(e, Event::LaneHealed { .. }))
        );
    }

    #[test]
    fn stunned_mobs_and_eliminated_lanes_do_not_move() {
        let mut world = world();
        let mut stunned = mob(0, MobKind::Silverfish, 1);
        stunned.statuses.stun_remaining = 3;
        world.lanes[0].mobs.push(stunned);
        world.lanes[1].mobs.push(mob(1, MobKind::Silverfish, 0));
        world.lanes[1].eliminated = true;
        advance(&mut world);
        assert_eq!(world.lanes[0].mobs[0].progress, 0.0);
        assert_eq!(world.lanes[1].mobs[0].progress, 0.0);
    }

    #[test]
    fn slow_is_reduced_by_resistance_and_a_hurt_piglin_runs() {
        use crate::domain::Slow;
        let mut world = world();
        let mut spider = mob(0, MobKind::Spider, 1);
        spider.statuses.slow = Some(Slow {
            percent: 0.4,
            remaining: 10,
        });
        let mut piglin = mob(1, MobKind::PiglinBrute, 1);
        piglin.hp = 100.0;
        world.lanes[0].mobs.push(spider);
        world.lanes[0].mobs.push(piglin);
        advance(&mut world);
        let spider_step = 3.8 / 20.0 * (1.0 - 0.2);
        let piglin_step = 2.8 / 20.0 * 1.3;
        assert!((world.lanes[0].mobs[0].progress - spider_step).abs() < 1e-5);
        assert!((world.lanes[0].mobs[1].progress - piglin_step).abs() < 1e-5);
    }

    fn dead(id: u32, kind: MobKind, sender: u128) -> Mob {
        let mut mob = mob(id, kind, sender);
        mob.hp = 0.0;
        mob
    }

    fn gold_of(world: &World, index: u128) -> u32 {
        world.player(pid(index)).unwrap().gold.get()
    }

    fn team_world() -> World {
        let mut world = world();
        world.lanes[0].owners = vec![pid(0), pid(1)];
        world
    }

    #[test]
    fn the_bounty_skips_a_first_owner_who_left() {
        let mut world = team_world();
        world.player_mut(pid(0)).unwrap().left = true;
        world.lanes[0].mobs.push(dead(0, MobKind::Zombie, 1));
        reap(&mut world);
        assert_eq!(gold_of(&world, 0), 100);
        assert_eq!(gold_of(&world, 1), 103);
    }

    #[test]
    fn the_bounty_of_a_hitter_who_left_goes_to_an_owner_still_present() {
        let mut world = team_world();
        world.player_mut(pid(0)).unwrap().left = true;
        let mut victim = dead(0, MobKind::Zombie, 1);
        victim.last_hit_by = Some(pid(0));
        world.lanes[0].mobs.push(victim);
        reap(&mut world);
        assert_eq!(gold_of(&world, 0), 100);
        assert_eq!(gold_of(&world, 1), 103);
    }

    #[test]
    fn nobody_is_paid_when_every_owner_left() {
        let mut world = team_world();
        world.player_mut(pid(0)).unwrap().left = true;
        world.player_mut(pid(1)).unwrap().left = true;
        world.lanes[0].mobs.push(dead(0, MobKind::Zombie, 1));
        reap(&mut world);
        assert_eq!(gold_of(&world, 0), 100);
        assert_eq!(gold_of(&world, 1), 100);
    }

    #[test]
    fn a_dead_mob_at_the_end_of_the_path_never_leaks() {
        let mut world = world();
        let mut corpse = dead(0, MobKind::Warden, 1);
        corpse.progress = 10.0;
        world.lanes[0].mobs.push(corpse);
        resolve_arrivals(&mut world);
        assert_eq!(world.lanes[0].hp.current(), 20);
        assert!(
            !world
                .events
                .iter()
                .any(|e| matches!(e, Event::MobLeaked { .. }))
        );
    }

    #[test]
    fn slime_children_born_at_the_end_of_the_path_do_not_leak() {
        let mut world = world();
        let mut parent = dead(0, MobKind::GiantSlime, 1);
        parent.progress = 10.0;
        world.lanes[0].mobs.push(parent);
        reap(&mut world);
        assert_eq!(world.lanes[0].mobs.len(), 2);
        for child in &world.lanes[0].mobs {
            assert!((child.progress - 9.99).abs() < 1e-4);
        }
        resolve_arrivals(&mut world);
        assert_eq!(world.lanes[0].hp.current(), 20);
        assert_eq!(world.lanes[0].mobs.len(), 2);
    }

    #[test]
    fn slime_children_keep_the_parent_progress_when_it_is_short_of_the_end() {
        let mut world = world();
        let mut parent = dead(0, MobKind::GiantSlime, 1);
        parent.progress = 4.0;
        world.lanes[0].mobs.push(parent);
        reap(&mut world);
        assert!(world.lanes[0].mobs.iter().all(|m| m.progress == 4.0));
    }

    #[test]
    fn reap_loops_until_a_curse_burst_chain_is_fully_collected() {
        let mut world = world();
        let mut first = dead(0, MobKind::Zombie, 1);
        first.statuses.cursed = true;
        first.last_hit_by = Some(pid(0));
        let mut second = mob(1, MobKind::Silverfish, 1);
        second.hp = 1.0;
        second.statuses.cursed = true;
        let mut third = mob(2, MobKind::Silverfish, 1);
        third.hp = 1.0;
        world.lanes[0].mobs.push(first);
        world.lanes[0].mobs.push(second);
        world.lanes[0].mobs.push(third);
        reap(&mut world);
        assert!(world.lanes[0].mobs.iter().all(|m| m.hp > 0.0));
        assert!(world.lanes[0].mobs.is_empty());
        let mut killed: Vec<u32> = world
            .events
            .iter()
            .filter_map(|e| match e {
                Event::MobKilled { mob, .. } => Some(mob.0),
                _ => None,
            })
            .collect();
        killed.sort_unstable();
        assert_eq!(killed, vec![0, 1, 2]);
        assert_eq!(gold_of(&world, 0), 100 + 3 + 1 + 1);
    }
}
