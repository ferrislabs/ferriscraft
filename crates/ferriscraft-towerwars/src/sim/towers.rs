use crate::domain::{
    Branch, Gold, LaneId, Level, MAX_GOLD_MINES_PER_PLAYER, PlayerId, Priority,
    SELL_REFUND_PERCENT, SimError, TowerId, TowerKind, Vec3,
};

use super::event::Event;
use super::lane::{Lane, Tower};
use super::world::World;

pub(crate) fn place(
    world: &mut World,
    player: PlayerId,
    kind: TowerKind,
    cell: (i32, i32),
) -> Result<TowerId, SimError> {
    let lane = acting_lane(world, player)?;
    let cost = kind.stats(Level::T1).cost;
    let remaining = gold_after(world, player, cost)?;
    let id = TowerId(world.next_tower_id);
    let surface_y = world.lane(lane).surface_y();
    world
        .lane_mut(lane)
        .place(new_tower(id, kind, cell, player, cost, surface_y))?;
    world.next_tower_id += 1;
    set_gold(world, player, remaining);
    world.emit(Event::TowerPlaced {
        lane,
        tower: id,
        kind,
        cell,
    });
    Ok(id)
}

pub(crate) fn upgrade(
    world: &mut World,
    player: PlayerId,
    tower: TowerId,
    branch: Option<Branch>,
) -> Result<(), SimError> {
    let lane = acting_lane(world, player)?;
    let (kind, level) = find_tower(world, lane, tower)?;
    let next = level.next(branch)?;
    ensure_mine_slot(world.lane(lane), player, kind, next)?;
    let cost = kind.stats(next).cost;
    let remaining = gold_after(world, player, cost)?;
    let target = world
        .lane_mut(lane)
        .tower_mut(tower)
        .ok_or(SimError::UnknownTower)?;
    target.level = next;
    target.invested += cost;
    set_gold(world, player, remaining);
    world.emit(Event::TowerUpgraded {
        lane,
        tower,
        level: next,
    });
    Ok(())
}

pub(crate) fn sell(world: &mut World, player: PlayerId, tower: TowerId) -> Result<u32, SimError> {
    let lane = acting_lane(world, player)?;
    find_tower(world, lane, tower)?;
    let removed = world
        .lane_mut(lane)
        .remove(tower)
        .ok_or(SimError::UnknownTower)?;
    let refund = refund_for(removed.invested);
    let state = world.player_mut(player).ok_or(SimError::UnknownPlayer)?;
    state.gold = state.gold.saturating_add(refund);
    world.emit(Event::TowerSold {
        lane,
        tower,
        refund,
    });
    Ok(refund)
}

pub(crate) fn set_priority(
    world: &mut World,
    player: PlayerId,
    tower: TowerId,
    priority: Priority,
) -> Result<(), SimError> {
    let lane = acting_lane(world, player)?;
    find_tower(world, lane, tower)?;
    let target = world
        .lane_mut(lane)
        .tower_mut(tower)
        .ok_or(SimError::UnknownTower)?;
    target.priority = priority;
    Ok(())
}

pub(crate) fn mobs_in_range(lane: &Lane, origin: Vec3, range: f32) -> Vec<usize> {
    lane.mobs
        .iter()
        .enumerate()
        .filter(|(_, mob)| mob.hp > 0.0 && origin.distance_xz(lane.mob_position(mob)) <= range)
        .map(|(index, _)| index)
        .collect()
}

pub(crate) fn pick_target(
    lane: &Lane,
    origin: Vec3,
    range: f32,
    priority: Priority,
) -> Option<usize> {
    mobs_in_range(lane, origin, range)
        .into_iter()
        .reduce(|best, candidate| {
            if outranks(lane, priority, candidate, best) {
                candidate
            } else {
                best
            }
        })
}

fn outranks(lane: &Lane, priority: Priority, candidate: usize, best: usize) -> bool {
    let (candidate, best) = (&lane.mobs[candidate], &lane.mobs[best]);
    match priority {
        Priority::First => candidate.progress > best.progress,
        Priority::Last => candidate.progress < best.progress,
        Priority::Strongest => candidate.hp > best.hp,
        Priority::Weakest => candidate.hp < best.hp,
    }
}

fn acting_lane(world: &World, player: PlayerId) -> Result<LaneId, SimError> {
    let state = world.player(player).ok_or(SimError::UnknownPlayer)?;
    if state.left {
        return Err(SimError::PlayerLeft);
    }
    let lane = state.lane;
    if world.lane(lane).eliminated {
        Err(SimError::LaneEliminated)
    } else {
        Ok(lane)
    }
}

fn gold_after(world: &World, player: PlayerId, cost: u32) -> Result<Gold, SimError> {
    world
        .player(player)
        .ok_or(SimError::UnknownPlayer)?
        .gold
        .checked_sub(cost)
        .ok_or(SimError::NotEnoughGold)
}

fn set_gold(world: &mut World, player: PlayerId, gold: Gold) {
    if let Some(state) = world.player_mut(player) {
        state.gold = gold;
    }
}

fn new_tower(
    id: TowerId,
    kind: TowerKind,
    cell: (i32, i32),
    owner: PlayerId,
    cost: u32,
    surface_y: f32,
) -> Tower {
    Tower {
        id,
        kind,
        level: Level::T1,
        cell,
        world_pos: Vec3::new(cell.0 as f32 + 0.5, surface_y, cell.1 as f32 + 0.5),
        owner,
        priority: Priority::First,
        cooldown_ticks: 0,
        invested: cost,
        stun_remaining: 0,
        shots: 0,
    }
}

fn find_tower(world: &World, lane: LaneId, tower: TowerId) -> Result<(TowerKind, Level), SimError> {
    match world.lane(lane).tower(tower) {
        Some(found) => Ok((found.kind, found.level)),
        None if world.lanes.iter().any(|other| other.tower(tower).is_some()) => {
            Err(SimError::NotOwner)
        }
        None => Err(SimError::UnknownTower),
    }
}

fn ensure_mine_slot(
    lane: &Lane,
    player: PlayerId,
    kind: TowerKind,
    next: Level,
) -> Result<(), SimError> {
    let is_mine = kind == TowerKind::Beacon && next == Level::T4(Branch::B);
    if is_mine && lane.gold_mines_owned(player) >= MAX_GOLD_MINES_PER_PLAYER {
        Err(SimError::GoldMineLimit)
    } else {
        Ok(())
    }
}

fn refund_for(invested: u32) -> u32 {
    let refund = u64::from(invested) * u64::from(SELL_REFUND_PERCENT) / 100;
    u32::try_from(refund).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::domain::{Aabb, GameMode, LaneHp, MobId, MobKind};
    use crate::map::Path;
    use crate::sim::lane::Mob;
    use crate::sim::rng::SplitMix64;
    use crate::sim::world::{Phase, PlayerState};

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn lane_at(index: u8, owners: Vec<PlayerId>) -> Lane {
        let z = f32::from(index) * 20.0;
        let path = Path::new(vec![Vec3::new(0.0, 64.0, z), Vec3::new(60.0, 64.0, z)]).unwrap();
        let bounds = vec![Aabb {
            min: Vec3::new(0.0, 64.0, z + 2.0),
            max: Vec3::new(99.0, 64.0, z + 5.0),
        }];
        Lane::new(
            LaneId::try_from(index).unwrap(),
            owners,
            LaneHp::full(25),
            Arc::new(path),
            bounds,
        )
    }

    fn world_with_gold(gold: u32) -> World {
        let (a, b, c) = (pid(0), pid(1), pid(2));
        let mut players = vec![
            PlayerState::new(a, LaneId::try_from(0).unwrap()),
            PlayerState::new(b, LaneId::try_from(0).unwrap()),
            PlayerState::new(c, LaneId::try_from(1).unwrap()),
        ];
        for state in &mut players {
            state.gold = Gold::new(gold);
        }
        World {
            mode: GameMode::Teams,
            players,
            lanes: vec![lane_at(0, vec![a, b]), lane_at(1, vec![c])],
            elapsed: 0,
            rng: SplitMix64::new(1),
            next_mob_id: 0,
            next_tower_id: 0,
            pending: Vec::new(),
            events: Vec::new(),
            phase: Phase::Running,
            sudden_death_level: 0,
        }
    }

    fn gold_of(world: &World, player: PlayerId) -> u32 {
        world.player(player).unwrap().gold.get()
    }

    fn cell(index: i32) -> (i32, i32) {
        (index % 100, 2 + index / 100)
    }

    #[test]
    fn the_eighty_first_tower_is_refused_without_spending_gold() {
        let mut world = world_with_gold(100_000);
        for index in 0..80 {
            place(&mut world, pid(0), TowerKind::Archer, cell(index)).unwrap();
        }
        let before = gold_of(&world, pid(0));
        let result = place(&mut world, pid(1), TowerKind::Archer, cell(80));
        assert_eq!(result, Err(SimError::LaneFull));
        assert_eq!(gold_of(&world, pid(1)), 100_000);
        assert_eq!(gold_of(&world, pid(0)), before);
        assert_eq!(world.lane(LaneId::try_from(0).unwrap()).towers().len(), 80);
    }

    #[test]
    fn a_placed_tower_has_the_documented_initial_state() {
        let mut world = world_with_gold(500);
        let id = place(&mut world, pid(0), TowerKind::Cannon, (3, 2)).unwrap();
        let lane = world.lane(LaneId::try_from(0).unwrap());
        let tower = lane.tower(id).unwrap();
        assert_eq!(tower.world_pos, Vec3::new(3.5, 64.0, 2.5));
        assert_eq!(tower.owner, pid(0));
        assert_eq!(tower.priority, Priority::First);
        assert_eq!(tower.cooldown_ticks, 0);
        assert_eq!(tower.invested, 125);
        assert_eq!(gold_of(&world, pid(0)), 375);
        assert_eq!(world.events.len(), 1);
    }

    #[test]
    fn tower_ids_increase_only_on_success() {
        let mut world = world_with_gold(500);
        let first = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        assert!(place(&mut world, pid(0), TowerKind::Archer, (3, 2)).is_err());
        let second = place(&mut world, pid(0), TowerKind::Archer, (4, 2)).unwrap();
        assert_eq!((first, second), (TowerId(0), TowerId(1)));
    }

    #[test]
    fn the_third_level_total_is_sold_back_at_eighty_percent() {
        let mut world = world_with_gold(1000);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        upgrade(&mut world, pid(0), id, None).unwrap();
        upgrade(&mut world, pid(0), id, None).unwrap();
        assert_eq!(gold_of(&world, pid(0)), 600);
        let refund = sell(&mut world, pid(0), id).unwrap();
        assert_eq!(refund, 320);
        assert_eq!(gold_of(&world, pid(0)), 920);
        assert!(world.lane(LaneId::try_from(0).unwrap()).tower(id).is_none());
        assert_eq!(
            world.lane(LaneId::try_from(0).unwrap()).tower_at((3, 2)),
            None
        );
    }

    #[test]
    fn refunds_round_down() {
        assert_eq!(refund_for(75), 60);
        assert_eq!(refund_for(125), 100);
        assert_eq!(refund_for(1), 0);
        assert_eq!(refund_for(399), 319);
    }

    #[test]
    fn a_teammate_acts_on_the_tower_with_their_own_gold() {
        let mut world = world_with_gold(1000);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        upgrade(&mut world, pid(1), id, None).unwrap();
        assert_eq!(gold_of(&world, pid(1)), 900);
        assert_eq!(gold_of(&world, pid(0)), 950);
        set_priority(&mut world, pid(1), id, Priority::Weakest).unwrap();
        assert_eq!(sell(&mut world, pid(1), id), Ok(120));
        assert_eq!(gold_of(&world, pid(1)), 1020);
        assert_eq!(gold_of(&world, pid(0)), 950);
    }

    #[test]
    fn a_tower_of_another_lane_is_not_owned() {
        let mut world = world_with_gold(1000);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        assert_eq!(
            upgrade(&mut world, pid(2), id, None),
            Err(SimError::NotOwner)
        );
        assert_eq!(sell(&mut world, pid(2), id), Err(SimError::NotOwner));
        assert_eq!(
            set_priority(&mut world, pid(2), id, Priority::Last),
            Err(SimError::NotOwner)
        );
        assert_eq!(
            sell(&mut world, pid(2), TowerId(99)),
            Err(SimError::UnknownTower)
        );
    }

    #[test]
    fn an_unknown_player_is_refused() {
        let mut world = world_with_gold(1000);
        assert_eq!(
            place(&mut world, pid(9), TowerKind::Archer, (3, 2)),
            Err(SimError::UnknownPlayer)
        );
    }

    #[test]
    fn an_eliminated_lane_refuses_every_command() {
        let mut world = world_with_gold(1000);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        world.lane_mut(LaneId::try_from(0).unwrap()).eliminated = true;
        assert_eq!(
            place(&mut world, pid(0), TowerKind::Archer, (4, 2)),
            Err(SimError::LaneEliminated)
        );
        assert_eq!(
            upgrade(&mut world, pid(0), id, None),
            Err(SimError::LaneEliminated)
        );
        assert_eq!(sell(&mut world, pid(0), id), Err(SimError::LaneEliminated));
    }

    #[test]
    fn a_fourth_tier_tower_needs_a_branch_and_stops_there() {
        let mut world = world_with_gold(5000);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        upgrade(&mut world, pid(0), id, None).unwrap();
        upgrade(&mut world, pid(0), id, None).unwrap();
        let gold = gold_of(&world, pid(0));
        assert_eq!(
            upgrade(&mut world, pid(0), id, None),
            Err(SimError::BranchChoice)
        );
        assert_eq!(gold_of(&world, pid(0)), gold);
        upgrade(&mut world, pid(0), id, Some(Branch::A)).unwrap();
        assert_eq!(gold_of(&world, pid(0)), gold - 600);
        assert_eq!(
            upgrade(&mut world, pid(0), id, Some(Branch::B)),
            Err(SimError::MaxLevel)
        );
        let lane = world.lane(LaneId::try_from(0).unwrap());
        assert_eq!(lane.tower(id).unwrap().level, Level::T4(Branch::A));
        assert_eq!(lane.tower(id).unwrap().invested, 1000);
    }

    #[test]
    fn an_upgrade_without_gold_changes_nothing() {
        let mut world = world_with_gold(60);
        let id = place(&mut world, pid(0), TowerKind::Archer, (3, 2)).unwrap();
        assert_eq!(
            upgrade(&mut world, pid(0), id, None),
            Err(SimError::NotEnoughGold)
        );
        let lane = world.lane(LaneId::try_from(0).unwrap());
        assert_eq!(lane.tower(id).unwrap().level, Level::T1);
        assert_eq!(gold_of(&world, pid(0)), 10);
    }

    #[test]
    fn a_player_owns_at_most_four_gold_mines() {
        let mut world = world_with_gold(100_000);
        let mut beacons = Vec::new();
        for index in 0..5 {
            let id = place(&mut world, pid(0), TowerKind::Beacon, cell(index)).unwrap();
            upgrade(&mut world, pid(0), id, None).unwrap();
            upgrade(&mut world, pid(0), id, None).unwrap();
            beacons.push(id);
        }
        for id in &beacons[..4] {
            upgrade(&mut world, pid(0), *id, Some(Branch::B)).unwrap();
        }
        let gold = gold_of(&world, pid(0));
        assert_eq!(
            upgrade(&mut world, pid(0), beacons[4], Some(Branch::B)),
            Err(SimError::GoldMineLimit)
        );
        assert_eq!(gold_of(&world, pid(0)), gold);
        upgrade(&mut world, pid(0), beacons[4], Some(Branch::A)).unwrap();
        let teammate = upgrade(&mut world, pid(1), beacons[4], Some(Branch::B));
        assert_eq!(teammate, Err(SimError::MaxLevel));
    }

    #[test]
    fn a_teammate_has_their_own_gold_mine_allowance() {
        let mut world = world_with_gold(100_000);
        let mut beacons = Vec::new();
        for index in 0..5 {
            let id = place(&mut world, pid(0), TowerKind::Beacon, cell(index)).unwrap();
            upgrade(&mut world, pid(0), id, None).unwrap();
            upgrade(&mut world, pid(0), id, None).unwrap();
            beacons.push(id);
        }
        for id in &beacons[..4] {
            upgrade(&mut world, pid(0), *id, Some(Branch::B)).unwrap();
        }
        assert_eq!(
            upgrade(&mut world, pid(0), beacons[4], Some(Branch::B)),
            Err(SimError::GoldMineLimit)
        );
        let other = place(&mut world, pid(1), TowerKind::Beacon, cell(10)).unwrap();
        upgrade(&mut world, pid(1), other, None).unwrap();
        upgrade(&mut world, pid(1), other, None).unwrap();
        assert_eq!(upgrade(&mut world, pid(1), other, Some(Branch::B)), Ok(()));
    }

    fn lane_with_mobs(specs: &[(f32, f32)]) -> Lane {
        let mut lane = lane_at(0, vec![pid(0)]);
        for (index, (progress, hp)) in specs.iter().enumerate() {
            let mut mob = Mob::new(
                MobId(index as u32),
                MobKind::Silverfish,
                pid(1),
                *hp,
                *progress,
                0.0,
            );
            mob.hp = *hp;
            lane.mobs.push(mob);
        }
        lane
    }

    const ORIGIN: Vec3 = Vec3::new(10.0, 64.0, 0.0);

    #[test]
    fn the_four_priorities_choose_different_targets() {
        let lane = lane_with_mobs(&[(8.0, 50.0), (12.0, 10.0), (5.0, 90.0)]);
        assert_eq!(pick_target(&lane, ORIGIN, 5.0, Priority::First), Some(1));
        assert_eq!(pick_target(&lane, ORIGIN, 5.0, Priority::Last), Some(2));
        assert_eq!(
            pick_target(&lane, ORIGIN, 5.0, Priority::Strongest),
            Some(2)
        );
        assert_eq!(pick_target(&lane, ORIGIN, 5.0, Priority::Weakest), Some(1));
    }

    #[test]
    fn targets_outside_the_range_are_ignored() {
        let lane = lane_with_mobs(&[(40.0, 5.0), (12.0, 99.0)]);
        assert_eq!(mobs_in_range(&lane, ORIGIN, 3.0), vec![1]);
        assert_eq!(pick_target(&lane, ORIGIN, 3.0, Priority::Weakest), Some(1));
        assert_eq!(pick_target(&lane, ORIGIN, 1.0, Priority::First), None);
    }

    #[test]
    fn ties_go_to_the_lowest_index() {
        let lane = lane_with_mobs(&[(9.0, 50.0), (9.0, 50.0), (9.0, 50.0)]);
        for priority in [
            Priority::First,
            Priority::Last,
            Priority::Strongest,
            Priority::Weakest,
        ] {
            assert_eq!(pick_target(&lane, ORIGIN, 5.0, priority), Some(0));
        }
        assert_eq!(mobs_in_range(&lane, ORIGIN, 5.0), vec![0, 1, 2]);
    }

    #[test]
    fn range_is_measured_on_the_horizontal_plane() {
        let lane = lane_with_mobs(&[(10.0, 50.0)]);
        let high = Vec3::new(10.0, 200.0, 0.0);
        assert_eq!(pick_target(&lane, high, 0.5, Priority::First), Some(0));
    }
}

#[cfg(test)]
mod combat_tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::domain::{
        Aabb, GameMode, LaneHp, LaneId, Level, MobId, MobKind, PlayerId, Priority, TowerId,
        TowerKind, Vec3,
    };
    use crate::map::Path;
    use crate::sim::combat;
    use crate::sim::lane::{Lane, Mob};
    use crate::sim::rng::SplitMix64;
    use crate::sim::world::{Phase, PlayerState, World};

    use super::new_tower;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn world() -> World {
        let id = LaneId::try_from(0).unwrap();
        let path = Path::new(vec![Vec3::new(0.0, 64.0, 0.0), Vec3::new(60.0, 64.0, 0.0)]).unwrap();
        let bounds = vec![Aabb {
            min: Vec3::new(0.0, 64.0, 2.0),
            max: Vec3::new(59.0, 64.0, 8.0),
        }];
        let lane = Lane::new(id, vec![pid(0)], LaneHp::full(25), Arc::new(path), bounds);
        World {
            mode: GameMode::Solo,
            players: vec![PlayerState::new(pid(0), id)],
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
        let mut tower = new_tower(id, kind, cell, pid(0), 0, 64.0);
        tower.level = level;
        world.lanes[0].place(tower).unwrap();
        id
    }

    fn add_mob(world: &mut World, x: f32) -> usize {
        let id = MobId(world.lanes[0].mobs.len() as u32);
        let kind = MobKind::Silverfish;
        let mob = Mob::new(id, kind, pid(1), kind.spec().hp, x, 0.0);
        world.lanes[0].mobs.push(mob);
        world.lanes[0].mobs.len() - 1
    }

    fn chain_of(bounce_gap: f32) -> (World, [usize; 3]) {
        let mut world = world();
        let tesla = add_tower(&mut world, TowerKind::Tesla, Level::T1, (10, 2));
        world.lanes[0].tower_mut(tesla).unwrap().priority = Priority::Last;
        let first = add_mob(&mut world, 10.0);
        let second = add_mob(&mut world, 12.0);
        let third = add_mob(&mut world, 12.0 + bounce_gap);
        combat::step_towers(&mut world);
        (world, [first, second, third])
    }

    #[test]
    fn spec_tw_16_a_mob_just_beyond_the_bounce_radius_is_not_hit() {
        let (world, [first, second, third]) = chain_of(3.6);
        let hp = |index: usize| world.lanes[0].mobs[index].hp;
        assert_eq!(hp(first), 15.0);
        assert_eq!(hp(second), 15.0);
        assert_eq!(hp(third), 35.0);
    }

    #[test]
    fn spec_tw_16_a_mob_just_inside_the_bounce_radius_is_hit() {
        let (world, [first, second, third]) = chain_of(3.4);
        let hp = |index: usize| world.lanes[0].mobs[index].hp;
        assert_eq!(hp(first), 15.0);
        assert_eq!(hp(second), 15.0);
        assert_eq!(hp(third), 15.0);
    }

    fn archer_cooldown_after_first_shot(with_beacon: bool) -> u16 {
        let mut world = world();
        let archer = add_tower(&mut world, TowerKind::Archer, Level::T1, (10, 2));
        if with_beacon {
            add_tower(&mut world, TowerKind::Beacon, Level::T1, (12, 2));
        }
        add_mob(&mut world, 10.0);
        combat::step_towers(&mut world);
        world.lanes[0].tower(archer).unwrap().cooldown_ticks
    }

    #[test]
    fn spec_tw_17_a_beacon_shortens_an_archer_cooldown_from_20_to_18_ticks() {
        assert_eq!(archer_cooldown_after_first_shot(false), 20);
        assert_eq!(archer_cooldown_after_first_shot(true), 18);
    }
}
