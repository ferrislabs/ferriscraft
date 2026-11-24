use std::sync::Arc;

use uuid::Uuid;

use crate::domain::{
    Aabb, Branch, Burn, GameMode, LaneHp, LaneId, Level, MobId, MobKind, PlayerId, Priority,
    TowerId, TowerKind, Vec3,
};
use crate::map::Path;
use crate::sim::event::Event;
use crate::sim::lane::{Lane, Mob, Tower};
use crate::sim::mobs;
use crate::sim::rng::SplitMix64;
use crate::sim::world::{Phase, PlayerState, World};

use super::{on_death, step_abilities, step_statuses, step_towers};

fn pid(index: u128) -> PlayerId {
    PlayerId(Uuid::from_u128(index + 1))
}

fn lane0() -> LaneId {
    LaneId::try_from(0).unwrap()
}

fn world() -> World {
    let path = Path::new(vec![Vec3::new(0.0, 64.0, 0.0), Vec3::new(60.0, 64.0, 0.0)]).unwrap();
    let bounds = vec![Aabb {
        min: Vec3::new(0.0, 64.0, 2.0),
        max: Vec3::new(59.0, 64.0, 8.0),
    }];
    let lane = Lane::new(
        lane0(),
        vec![pid(0)],
        LaneHp::full(25),
        Arc::new(path),
        bounds,
    );
    World {
        mode: GameMode::Solo,
        players: vec![PlayerState::new(pid(0), lane0())],
        lanes: vec![lane],
        elapsed: 1,
        rng: SplitMix64::new(1),
        next_mob_id: 100,
        next_tower_id: 0,
        pending: Vec::new(),
        events: Vec::new(),
        phase: Phase::Running,
        sudden_death_level: 0,
    }
}

fn add_tower(world: &mut World, kind: TowerKind, level: Level, cell: (i32, i32)) -> TowerId {
    let id = TowerId(world.next_tower_id);
    world.next_tower_id += 1;
    let tower = Tower {
        id,
        kind,
        level,
        cell,
        world_pos: Vec3::new(cell.0 as f32 + 0.5, 64.0, cell.1 as f32 + 0.5),
        owner: pid(0),
        priority: Priority::First,
        cooldown_ticks: 0,
        invested: 0,
        stun_remaining: 0,
        shots: 0,
    };
    world.lanes[0].place(tower).unwrap();
    id
}

fn prefer(world: &mut World, id: TowerId, priority: Priority) {
    world.lanes[0].tower_mut(id).unwrap().priority = priority;
}

fn add_mob(world: &mut World, kind: MobKind, progress: f32) -> usize {
    let id = MobId(world.lanes[0].mobs.len() as u32);
    let mob = Mob::new(id, kind, pid(1), kind.spec().hp, progress, 0.0);
    world.lanes[0].mobs.push(mob);
    world.lanes[0].mobs.len() - 1
}

fn mob(world: &World, index: usize) -> &Mob {
    &world.lanes[0].mobs[index]
}

fn tower(world: &World, id: TowerId) -> &Tower {
    world.lanes[0].tower(id).unwrap()
}

fn ready(world: &mut World, id: TowerId) {
    world.lanes[0].tower_mut(id).unwrap().cooldown_ticks = 0;
}

fn near(left: f32, right: f32) -> bool {
    (left - right).abs() < 1e-3
}

fn fired_targets(world: &World) -> Vec<f32> {
    world
        .events
        .iter()
        .filter_map(|event| match event {
            Event::TowerFired { to, .. } => Some(to.x),
            _ => None,
        })
        .collect()
}

fn explosions(world: &World) -> usize {
    world
        .events
        .iter()
        .filter(|event| matches!(event, Event::Explosion { .. }))
        .count()
}

#[test]
fn spec_tw_11_each_priority_picks_its_own_target() {
    let cases = [
        (Priority::First, 12.0),
        (Priority::Last, 4.0),
        (Priority::Strongest, 8.0),
        (Priority::Weakest, 9.0),
    ];
    for (priority, expected) in cases {
        let mut world = world();
        let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
        world.lanes[0].tower_mut(archer).unwrap().priority = priority;
        add_mob(&mut world, MobKind::Silverfish, 12.0);
        add_mob(&mut world, MobKind::Silverfish, 4.0);
        let strong = add_mob(&mut world, MobKind::Silverfish, 8.0);
        let weak = add_mob(&mut world, MobKind::Silverfish, 9.0);
        world.lanes[0].mobs[strong].hp = 90.0;
        world.lanes[0].mobs[weak].hp = 10.0;
        step_towers(&mut world);
        assert_eq!(fired_targets(&world), vec![expected], "{priority:?}");
    }
}

#[test]
fn a_dead_mob_is_never_targeted() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let dead = add_mob(&mut world, MobKind::Silverfish, 12.0);
    add_mob(&mut world, MobKind::Silverfish, 8.0);
    world.lanes[0].mobs[dead].hp = 0.0;
    step_towers(&mut world);
    assert_eq!(fired_targets(&world), vec![8.0]);
}

#[test]
fn spec_tw_12_an_archer_hits_for_twelve_every_twenty_ticks() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    let mut hp = Vec::new();
    for _ in 0..41 {
        step_towers(&mut world);
        hp.push(mob(&world, target).hp);
    }
    assert_eq!(hp[0], 23.0);
    assert_eq!(hp[19], 23.0);
    assert_eq!(hp[20], 11.0);
    assert_eq!(hp[39], 11.0);
    assert_eq!(hp[40], -1.0);
    assert_eq!(tower(&world, archer).shots, 3);
    assert_eq!(mob(&world, target).last_hit_by, Some(pid(0)));
}

#[test]
fn a_tower_with_nothing_in_range_stays_ready() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    add_mob(&mut world, MobKind::Silverfish, 40.0);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 0);
    assert!(world.events.is_empty());
    add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 20);
}

#[test]
fn spec_tw_13_armor_reduces_physical_but_not_magic() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let zombie = add_mob(&mut world, MobKind::Zombie, 10.0);
    step_towers(&mut world);
    assert!(near(mob(&world, zombie).hp, 95.0 - 10.8));
    let mut world = self::world();
    add_tower(&mut world, TowerKind::Mage, Level::T1, (10, 2));
    let zombie = add_mob(&mut world, MobKind::Zombie, 10.0);
    step_towers(&mut world);
    assert!(near(mob(&world, zombie).hp, 80.0));
}

#[test]
fn armor_is_capped_at_ninety_percent() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Slime, Level::T3, (10, 2));
    let golem = add_mob(&mut world, MobKind::IronGolem, 10.0);
    world.lanes[0].mobs[golem].statuses.shred = Some(crate::domain::ArmorShred {
        percent: 0.8,
        remaining: 10,
    });
    step_towers(&mut world);
    assert!(near(mob(&world, golem).hp, 5500.0 - 7.5));
}

#[test]
fn spec_tw_14_slows_do_not_stack_and_the_strongest_wins() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Ice, Level::T1, (10, 2));
    add_tower(&mut world, TowerKind::Ice, Level::T1, (11, 2));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    let slow = mob(&world, target).statuses.slow.unwrap();
    assert_eq!((slow.percent, slow.remaining), (0.25, 50));
    let stronger = add_tower(&mut world, TowerKind::Ice, Level::T2, (12, 2));
    ready(&mut world, stronger);
    step_towers(&mut world);
    let slow = mob(&world, target).statuses.slow.unwrap();
    assert_eq!((slow.percent, slow.remaining), (0.35, 60));
}

#[test]
fn spec_tw_14_a_spider_only_suffers_half_the_slow() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Ice, Level::T1, (10, 2));
    let spider = add_mob(&mut world, MobKind::Spider, 10.0);
    let fish = add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    mobs::advance(&mut world);
    let spider_step = mob(&world, spider).progress - 10.0;
    let fish_step = mob(&world, fish).progress - 10.0;
    assert!(near(spider_step, 3.8 / 20.0 * 0.875));
    assert!(near(fish_step, 3.2 / 20.0 * 0.75));
}

#[test]
fn a_slow_lasts_exactly_its_ticks() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Ice, Level::T1, (10, 2));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    for _ in 0..49 {
        step_statuses(&mut world);
    }
    assert!(mob(&world, target).statuses.slow.is_some());
    step_statuses(&mut world);
    assert!(mob(&world, target).statuses.slow.is_none());
}

#[test]
fn permafrost_makes_targets_vulnerable() {
    let mut world = world();
    let ice = add_tower(&mut world, TowerKind::Ice, Level::T4(Branch::A), (10, 2));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    let vulnerability = mob(&world, target).statuses.vulnerability.unwrap();
    assert_eq!((vulnerability.percent, vulnerability.remaining), (0.15, 80));
    let before = mob(&world, target).hp;
    ready(&mut world, ice);
    step_towers(&mut world);
    assert!(near(before - mob(&world, target).hp, 25.0 * 1.15));
}

#[test]
fn spec_tw_15_a_mage_burns_five_per_second_for_sixty_ticks() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Mage, Level::T1, (10, 2));
    let target = add_mob(&mut world, MobKind::Zombie, 10.0);
    step_towers(&mut world);
    assert_eq!(
        mob(&world, target).statuses.burns,
        vec![Burn {
            dps: 5.0,
            remaining: 60
        }]
    );
    let after_hit = mob(&world, target).hp;
    for _ in 0..60 {
        step_statuses(&mut world);
    }
    assert!(near(after_hit - mob(&world, target).hp, 15.0));
    assert!(mob(&world, target).statuses.burns.is_empty());
}

#[test]
fn spec_tw_15_a_blaze_is_immune_to_burn() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Mage, Level::T1, (10, 2));
    let blaze = add_mob(&mut world, MobKind::Blaze, 10.0);
    step_towers(&mut world);
    assert!(mob(&world, blaze).statuses.burns.is_empty());
    assert!(near(mob(&world, blaze).hp, 550.0 - 15.0));
}

#[test]
fn a_burning_mob_keeps_one_entry_refreshed_to_the_stronger_values() {
    let mut world = world();
    let weak = add_tower(&mut world, TowerKind::Mage, Level::T1, (10, 2));
    let strong = add_tower(&mut world, TowerKind::Mage, Level::T3, (11, 2));
    let target = add_mob(&mut world, MobKind::IronGolem, 10.0);
    world.lanes[0].tower_mut(strong).unwrap().cooldown_ticks = 5;
    step_towers(&mut world);
    assert_eq!(mob(&world, target).statuses.burns.len(), 1);
    ready(&mut world, weak);
    for _ in 0..5 {
        step_towers(&mut world);
    }
    let burns = &mob(&world, target).statuses.burns;
    assert_eq!(
        burns,
        &vec![Burn {
            dps: 25.0,
            remaining: 80
        }]
    );
}

#[test]
fn a_pyromancer_stacks_at_most_three_burns_on_everything_nearby() {
    let mut world = world();
    let pyro = add_tower(&mut world, TowerKind::Mage, Level::T4(Branch::A), (10, 2));
    prefer(&mut world, pyro, Priority::Last);
    let target = add_mob(&mut world, MobKind::IronGolem, 10.0);
    let neighbour = add_mob(&mut world, MobKind::IronGolem, 11.0);
    let far = add_mob(&mut world, MobKind::IronGolem, 16.0);
    for _ in 0..4 {
        ready(&mut world, pyro);
        step_towers(&mut world);
    }
    assert_eq!(mob(&world, target).statuses.burns.len(), 3);
    assert_eq!(mob(&world, neighbour).statuses.burns.len(), 3);
    assert!(mob(&world, far).statuses.burns.is_empty());
    assert_eq!(mob(&world, target).statuses.burns[0].dps, 45.0);
}

#[test]
fn a_necromancer_curses_and_the_death_bursts() {
    let mut world = world();
    let necro = add_tower(&mut world, TowerKind::Mage, Level::T4(Branch::B), (10, 2));
    prefer(&mut world, necro, Priority::Last);
    let target = add_mob(&mut world, MobKind::Zombie, 10.0);
    world.lanes[0].mobs[target].hp = 10.0;
    let near_mob = add_mob(&mut world, MobKind::IronGolem, 12.0);
    let far_mob = add_mob(&mut world, MobKind::IronGolem, 20.0);
    step_towers(&mut world);
    assert!(mob(&world, target).statuses.cursed);
    assert!(mob(&world, target).hp <= 0.0);
    let corpse = world.lanes[0].mobs.remove(target);
    world.events.clear();
    on_death(&mut world, lane0(), &corpse);
    assert!(near(
        mob(&world, near_mob - 1).hp,
        5500.0 - 0.30 * corpse.max_hp
    ));
    assert!(near(mob(&world, far_mob - 1).hp, 5500.0));
    assert_eq!(explosions(&world), 1);
}

#[test]
fn a_cannon_splashes_walkers_but_not_flyers() {
    let mut world = world();
    let cannon = add_tower(&mut world, TowerKind::Cannon, Level::T1, (10, 2));
    prefer(&mut world, cannon, Priority::Last);
    let primary = add_mob(&mut world, MobKind::IronGolem, 10.0);
    let close = add_mob(&mut world, MobKind::Silverfish, 11.5);
    let flyer = add_mob(&mut world, MobKind::Ghast, 10.5);
    let out = add_mob(&mut world, MobKind::Silverfish, 14.0);
    step_towers(&mut world);
    assert!(near(mob(&world, primary).hp, 5500.0 - 28.0 * 0.6));
    assert!(near(mob(&world, close).hp, 35.0 - 28.0));
    assert_eq!(mob(&world, flyer).hp, 2400.0);
    assert_eq!(mob(&world, out).hp, 35.0);
    assert_eq!(explosions(&world), 1);
}

#[test]
fn cluster_bombs_add_submunition_impacts_deterministically() {
    let run = || {
        let mut world = world();
        add_tower(&mut world, TowerKind::Cannon, Level::T4(Branch::B), (10, 2));
        add_mob(&mut world, MobKind::IronGolem, 10.0);
        step_towers(&mut world);
        world.events
    };
    let events = run();
    let count = events
        .iter()
        .filter(|event| matches!(event, Event::Explosion { .. }))
        .count();
    assert_eq!(count, 5);
    assert_eq!(events, run());
}

#[test]
fn spec_tw_16_a_tesla_chains_through_three_distinct_mobs() {
    let mut world = world();
    let tesla = add_tower(&mut world, TowerKind::Tesla, Level::T1, (10, 2));
    prefer(&mut world, tesla, Priority::Last);
    let a = add_mob(&mut world, MobKind::Silverfish, 10.0);
    let b = add_mob(&mut world, MobKind::Silverfish, 12.0);
    let c = add_mob(&mut world, MobKind::Silverfish, 14.0);
    let d = add_mob(&mut world, MobKind::Silverfish, 16.0);
    step_towers(&mut world);
    for index in [a, b, c] {
        assert_eq!(mob(&world, index).hp, 15.0);
    }
    assert_eq!(mob(&world, d).hp, 35.0);
    let points = world.events.iter().find_map(|event| match event {
        Event::Chain { points, .. } => Some(points.len()),
        _ => None,
    });
    assert_eq!(points, Some(4));
}

#[test]
fn archers_and_teslas_hit_flyers_harder() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let flyer = add_mob(&mut world, MobKind::Ghast, 10.0);
    step_towers(&mut world);
    assert!(near(mob(&world, flyer).hp, 2400.0 - 15.0));
}

#[test]
fn an_overload_tesla_stuns_the_primary_every_third_shot() {
    let mut world = world();
    let tesla = add_tower(&mut world, TowerKind::Tesla, Level::T4(Branch::B), (10, 2));
    let target = add_mob(&mut world, MobKind::Zombie, 10.0);
    let golem = add_mob(&mut world, MobKind::IronGolem, 40.0);
    world.lanes[0].mobs[target].hp = 1000.0;
    for _ in 0..2 {
        step_towers(&mut world);
        ready(&mut world, tesla);
    }
    assert_eq!(mob(&world, target).statuses.stun_remaining, 0);
    step_towers(&mut world);
    assert_eq!(mob(&world, target).statuses.stun_remaining, 8);
    assert_eq!(tower(&world, tesla).shots, 3);
    assert_eq!(mob(&world, golem).statuses.stun_remaining, 0);
}

#[test]
fn an_overload_stun_skips_stun_immune_mobs() {
    let mut world = world();
    let tesla = add_tower(&mut world, TowerKind::Tesla, Level::T4(Branch::B), (10, 2));
    let golem = add_mob(&mut world, MobKind::IronGolem, 10.0);
    world.lanes[0].tower_mut(tesla).unwrap().shots = 2;
    step_towers(&mut world);
    assert_eq!(mob(&world, golem).statuses.stun_remaining, 0);
}

#[test]
fn blizzard_damages_every_tick_and_freezes_with_immunity() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Ice, Level::T4(Branch::B), (10, 2));
    let mut crowd = Vec::new();
    for _ in 0..40 {
        crowd.push(add_mob(&mut world, MobKind::Silverfish, 10.0));
    }
    let golem = add_mob(&mut world, MobKind::IronGolem, 10.0);
    step_towers(&mut world);
    assert!(near(mob(&world, crowd[0]).hp, 35.0 - 65.0 / 20.0));
    assert!(
        crowd
            .iter()
            .all(|i| mob(&world, *i).statuses.stun_remaining == 0)
    );
    world.elapsed = 20;
    step_towers(&mut world);
    let frozen: Vec<usize> = crowd
        .iter()
        .copied()
        .filter(|i| mob(&world, *i).statuses.stun_remaining == 15)
        .collect();
    assert!(!frozen.is_empty() && frozen.len() < crowd.len());
    for index in &frozen {
        assert_eq!(mob(&world, *index).statuses.freeze_immunity_remaining, 80);
    }
    assert_eq!(mob(&world, golem).statuses.stun_remaining, 0);
    for index in &frozen {
        world.lanes[0].mobs[*index].statuses.stun_remaining = 0;
    }
    world.elapsed = 40;
    step_towers(&mut world);
    for index in &frozen {
        assert_eq!(mob(&world, *index).statuses.stun_remaining, 0);
    }
    assert!(world.events.is_empty());
}

#[test]
fn a_corroding_slime_shreds_armor_and_the_strongest_shred_wins() {
    let mut world = world();
    let slime = add_tower(&mut world, TowerKind::Slime, Level::T1, (10, 2));
    let target = add_mob(&mut world, MobKind::Zombie, 10.0);
    step_towers(&mut world);
    let shred = mob(&world, target).statuses.shred.unwrap();
    assert_eq!((shred.percent, shred.remaining), (0.15, 80));
    assert!(near(mob(&world, target).hp, 95.0 - 14.0 * 0.9));
    let better = add_tower(&mut world, TowerKind::Slime, Level::T2, (11, 2));
    ready(&mut world, better);
    ready(&mut world, slime);
    step_towers(&mut world);
    assert_eq!(mob(&world, target).statuses.shred.unwrap().percent, 0.25);
}

#[test]
fn a_pool_burns_walkers_inside_and_expires() {
    let mut world = world();
    let slime = add_tower(&mut world, TowerKind::Slime, Level::T4(Branch::A), (10, 2));
    prefer(&mut world, slime, Priority::Last);
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    world.lanes[0].mobs[target].hp = 10_000.0;
    let inside = add_mob(&mut world, MobKind::Silverfish, 12.0);
    let outside = add_mob(&mut world, MobKind::Silverfish, 20.0);
    let flyer = add_mob(&mut world, MobKind::Ghast, 10.5);
    let ravager = add_mob(&mut world, MobKind::Ravager, 10.5);
    step_towers(&mut world);
    assert_eq!(world.lanes[0].pools.len(), 1);
    step_statuses(&mut world);
    assert!(near(mob(&world, inside).hp, 35.0 - 3.0));
    assert_eq!(mob(&world, inside).statuses.shred.unwrap().percent, 0.5);
    step_statuses(&mut world);
    assert!(near(mob(&world, inside).hp, 35.0 - 3.0 - 1.5));
    for _ in 0..77 {
        step_statuses(&mut world);
    }
    assert_eq!(world.lanes[0].pools.len(), 1);
    step_statuses(&mut world);
    assert_eq!(mob(&world, outside).hp, 35.0);
    assert_eq!(mob(&world, flyer).hp, 2400.0);
    assert_eq!(mob(&world, ravager).hp, 7800.0);
    assert!(world.lanes[0].pools.is_empty());
}

#[test]
fn a_mutator_adds_a_fraction_of_current_hp() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Slime, Level::T4(Branch::B), (10, 2));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    world.lanes[0].mobs[target].hp = 1000.0;
    step_towers(&mut world);
    assert!(near(mob(&world, target).hp, 925.0 - 0.025 * 925.0));
}

#[test]
fn spec_tw_17_a_beacon_speeds_attacks_and_a_third_is_ignored() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    add_tower(&mut world, TowerKind::Beacon, Level::T1, (11, 2));
    add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 18);
    add_tower(&mut world, TowerKind::Beacon, Level::T1, (9, 2));
    ready(&mut world, archer);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 17);
    add_tower(&mut world, TowerKind::Beacon, Level::T1, (10, 3));
    ready(&mut world, archer);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 17);
}

#[test]
fn the_two_strongest_beacons_are_kept_and_range_and_damage_apply() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    add_tower(&mut world, TowerKind::Beacon, Level::T1, (11, 2));
    add_tower(&mut world, TowerKind::Beacon, Level::T4(Branch::A), (9, 2));
    add_tower(&mut world, TowerKind::Beacon, Level::T3, (10, 3));
    let target = add_mob(&mut world, MobKind::Silverfish, 10.0);
    world.lanes[0].mobs[target].hp = 1000.0;
    step_towers(&mut world);
    assert!(near(mob(&world, target).hp, 1000.0 - 12.0 * 1.2));
    assert_eq!(tower(&world, archer).cooldown_ticks, 12);
}

#[test]
fn a_distant_beacon_gives_nothing_and_beacons_never_fire() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let beacon = add_tower(&mut world, TowerKind::Beacon, Level::T1, (30, 2));
    add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 20);
    assert_eq!(tower(&world, beacon).shots, 0);
}

#[test]
fn a_warden_slows_nearby_towers() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    add_mob(&mut world, MobKind::Warden, 10.0);
    step_towers(&mut world);
    assert_eq!(tower(&world, archer).cooldown_ticks, 25);
}

#[test]
fn a_stunned_tower_does_not_fire_and_counts_down() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    world.lanes[0].tower_mut(archer).unwrap().stun_remaining = 2;
    add_mob(&mut world, MobKind::Silverfish, 10.0);
    step_towers(&mut world);
    step_towers(&mut world);
    assert!(world.events.is_empty());
    assert_eq!(tower(&world, archer).stun_remaining, 0);
    step_towers(&mut world);
    assert_eq!(world.events.len(), 1);
}

#[test]
fn an_eliminated_lane_does_not_fire() {
    let mut world = world();
    add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    add_mob(&mut world, MobKind::Silverfish, 10.0);
    world.lanes[0].eliminated = true;
    step_towers(&mut world);
    assert!(world.events.is_empty());
}

#[test]
fn a_skeleton_dodges_some_shots_deterministically() {
    let count = || {
        let mut world = world();
        let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
        let target = add_mob(&mut world, MobKind::Skeleton, 10.0);
        world.lanes[0].mobs[target].hp = 100_000.0;
        let mut dodged = 0;
        for _ in 0..200 {
            let before = mob(&world, target).hp;
            ready(&mut world, archer);
            step_towers(&mut world);
            if mob(&world, target).hp == before {
                dodged += 1;
            }
        }
        dodged
    };
    let first = count();
    assert!((10..=60).contains(&first), "{first}");
    assert_eq!(first, count());
}

#[test]
fn a_marksman_crit_multiplies_the_damage() {
    let mut world = world();
    let archer = add_tower(&mut world, TowerKind::Archer, Level::T4(Branch::A), (10, 2));
    let target = add_mob(&mut world, MobKind::IronGolem, 10.0);
    world.lanes[0].mobs[target].hp = 1_000_000.0;
    let mut amounts = Vec::new();
    for _ in 0..60 {
        let before = mob(&world, target).hp;
        ready(&mut world, archer);
        step_towers(&mut world);
        amounts.push(before - mob(&world, target).hp);
    }
    assert!(amounts.iter().any(|a| near(*a, 180.0 * 0.6)));
    assert!(amounts.iter().any(|a| near(*a, 180.0 * 2.5 * 0.6)));
}

#[test]
fn spec_tw_18_a_giant_slime_splits_into_two_medium_slimes() {
    let mut world = world();
    let parent = Mob::new(MobId(7), MobKind::GiantSlime, pid(1), 1100.0, 20.0, 0.0);
    on_death(&mut world, lane0(), &parent);
    let mobs = &world.lanes[0].mobs;
    assert_eq!(mobs.len(), 2);
    for child in mobs {
        assert_eq!(child.kind, MobKind::MediumSlime);
        assert_eq!(
            (child.progress, child.hp, child.sender),
            (20.0, 300.0, pid(1))
        );
    }
    assert_ne!(mobs[0].id, mobs[1].id);
    assert_eq!(world.next_mob_id, 102);
    assert_eq!(
        world.events,
        vec![Event::MobSplit {
            lane: lane0(),
            parent: MobId(7),
            children: [MobId(100), MobId(101)],
        }]
    );
}

#[test]
fn spec_tw_19_split_children_scale_with_sudden_death() {
    let mut world = world();
    world.sudden_death_level = 2;
    let parent = Mob::new(MobId(7), MobKind::GiantSlime, pid(1), 1100.0, 20.0, 0.0);
    on_death(&mut world, lane0(), &parent);
    assert!(near(world.lanes[0].mobs[0].hp, 600.0));
}

#[test]
fn spec_tw_18_an_enderman_blinks_once_below_half_hp() {
    let mut world = world();
    let enderman = add_mob(&mut world, MobKind::Enderman, 10.0);
    step_abilities(&mut world);
    assert_eq!(mob(&world, enderman).progress, 10.0);
    world.lanes[0].mobs[enderman].hp = 700.0;
    step_abilities(&mut world);
    step_abilities(&mut world);
    assert_eq!(mob(&world, enderman).progress, 14.0);
    assert!(mob(&world, enderman).blinked);
    let blinks = world
        .events
        .iter()
        .filter(|event| matches!(event, Event::MobBlinked { .. }))
        .count();
    assert_eq!(blinks, 1);
}

#[test]
fn a_blink_never_leaves_the_path() {
    let mut world = world();
    let enderman = add_mob(&mut world, MobKind::Enderman, 58.0);
    world.lanes[0].mobs[enderman].hp = 10.0;
    step_abilities(&mut world);
    assert_eq!(mob(&world, enderman).progress, 60.0);
}

#[test]
fn spec_tw_18_a_witch_heals_nearby_allies() {
    let mut world = world();
    add_mob(&mut world, MobKind::Witch, 10.0);
    let hurt = add_mob(&mut world, MobKind::Zombie, 12.0);
    let far = add_mob(&mut world, MobKind::Zombie, 20.0);
    world.lanes[0].mobs[hurt].hp = 50.0;
    world.lanes[0].mobs[far].hp = 50.0;
    for _ in 0..20 {
        step_statuses(&mut world);
    }
    assert!(near(mob(&world, hurt).hp, 80.0));
    assert_eq!(mob(&world, far).hp, 50.0);
    for _ in 0..40 {
        step_statuses(&mut world);
    }
    assert_eq!(mob(&world, hurt).hp, 95.0);
}

#[test]
fn a_witch_does_not_heal_herself() {
    let mut world = world();
    let witch = add_mob(&mut world, MobKind::Witch, 10.0);
    world.lanes[0].mobs[witch].hp = 100.0;
    step_statuses(&mut world);
    assert_eq!(mob(&world, witch).hp, 100.0);
}

#[test]
fn spec_tw_19_sudden_death_stops_the_witch_and_the_wither() {
    let mut world = world();
    world.sudden_death_level = 1;
    add_mob(&mut world, MobKind::Witch, 10.0);
    let hurt = add_mob(&mut world, MobKind::Zombie, 12.0);
    let wither = add_mob(&mut world, MobKind::Wither, 30.0);
    world.lanes[0].mobs[hurt].hp = 50.0;
    world.lanes[0].mobs[wither].hp = 1000.0;
    step_statuses(&mut world);
    assert_eq!(mob(&world, hurt).hp, 50.0);
    assert_eq!(mob(&world, wither).hp, 1000.0);
}

#[test]
fn a_wither_regenerates_outside_sudden_death() {
    let mut world = world();
    let wither = add_mob(&mut world, MobKind::Wither, 30.0);
    world.lanes[0].mobs[wither].hp = 1000.0;
    step_statuses(&mut world);
    assert!(near(mob(&world, wither).hp, 1007.5));
}

#[test]
fn spec_tw_18_a_creeper_stuns_towers_within_two_and_a_half_blocks() {
    let mut world = world();
    let close = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    let far = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 6));
    let creeper = Mob::new(MobId(7), MobKind::Creeper, pid(1), 420.0, 10.5, 0.0);
    on_death(&mut world, lane0(), &creeper);
    assert_eq!(tower(&world, close).stun_remaining, 20);
    assert_eq!(tower(&world, far).stun_remaining, 0);
    assert_eq!(
        world.events,
        vec![Event::TowerStunned {
            lane: lane0(),
            tower: close,
            ticks: 20,
        }]
    );
}

#[test]
fn a_creeper_does_not_shorten_a_longer_stun() {
    let mut world = world();
    let close = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
    world.lanes[0].tower_mut(close).unwrap().stun_remaining = 50;
    let creeper = Mob::new(MobId(7), MobKind::Creeper, pid(1), 420.0, 10.5, 0.0);
    on_death(&mut world, lane0(), &creeper);
    assert_eq!(tower(&world, close).stun_remaining, 50);
}
