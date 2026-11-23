use crate::domain::{
    Branch, INCOME_CYCLE_TICKS, LaneId, Level, MAX_GOLD_MINES_PER_PLAYER, MobId, MobKind, PlayerId,
    SEND_STAGGER_TICKS, STOCK_MAX, STOCK_RECHARGE_TICKS, SUDDEN_DEATH_HP_STEP, SimError,
    TIER_UNLOCK_TICKS, TowerEffect, TowerKind,
};

use super::event::Event;
use super::lane::Mob;
use super::world::{PendingSpawn, World};

const LATERAL_RANGE: f32 = 0.35;

pub(crate) fn pay_income(world: &mut World) {
    if !world.elapsed.is_multiple_of(INCOME_CYCLE_TICKS) {
        return;
    }
    for index in 0..world.players.len() {
        let player = &world.players[index];
        if player.left || world.lane(player.lane).eliminated {
            continue;
        }
        let amount = player.income + mine_income(world, player.lane, player.id);
        let id = player.id;
        let state = &mut world.players[index];
        state.gold = state.gold.saturating_add(amount);
        world.emit(Event::IncomePaid { player: id, amount });
    }
}

fn mine_income(world: &World, lane: LaneId, player: PlayerId) -> u32 {
    world
        .lane(lane)
        .towers()
        .iter()
        .filter(|tower| tower.owner == player)
        .filter(|tower| tower.kind == TowerKind::Beacon && tower.level == Level::T4(Branch::B))
        .filter_map(|tower| match tower.kind.stats(tower.level).effect {
            TowerEffect::GoldMine { income, .. } => Some(income),
            _ => None,
        })
        .take(MAX_GOLD_MINES_PER_PLAYER)
        .sum()
}

pub(crate) fn recharge_stock(world: &mut World) {
    for player in &mut world.players {
        let stock = &mut player.stock;
        for (charges, counter) in stock.charges.iter_mut().zip(stock.recharge.iter_mut()) {
            if *charges >= STOCK_MAX {
                *counter = 0;
                continue;
            }
            *counter += 1;
            if *counter >= STOCK_RECHARGE_TICKS {
                *charges += 1;
                *counter = 0;
            }
        }
    }
}

pub(crate) fn unlock_tiers(world: &mut World) {
    for tier in 2..=4u8 {
        if world.elapsed == TIER_UNLOCK_TICKS[usize::from(tier) - 1] {
            world.emit(Event::TierUnlocked { tier });
        }
    }
}

pub(crate) fn send(
    world: &mut World,
    player: PlayerId,
    kind: MobKind,
    count: u8,
) -> Result<(), SimError> {
    let state = world.player(player).ok_or(SimError::UnknownPlayer)?;
    if state.left {
        return Err(SimError::PlayerLeft);
    }
    let sender_lane = state.lane;
    if world.lane(sender_lane).eliminated {
        return Err(SimError::LaneEliminated);
    }
    let slot = kind.stock_index().ok_or(SimError::NotSendable)?;
    let spec = kind.spec();
    if world.elapsed < TIER_UNLOCK_TICKS[usize::from(spec.tier) - 1] {
        return Err(SimError::LockedTier);
    }
    if state.stock.charges[slot] < count {
        return Err(SimError::NoCharge);
    }
    if count == 0 {
        return Ok(());
    }
    let remaining = state
        .gold
        .checked_sub(spec.cost * u32::from(count))
        .ok_or(SimError::NotEnoughGold)?;
    let state = world.players.iter_mut().find(|p| p.id == player);
    if let Some(state) = state {
        state.gold = remaining;
        state.stock.charges[slot] -= count;
        state.income += spec.income * u32::from(count);
    }
    world.emit(Event::MobSent {
        sender: player,
        kind,
        count,
    });
    queue_spawns(world, sender_lane, player, kind, count);
    Ok(())
}

fn queue_spawns(
    world: &mut World,
    sender_lane: LaneId,
    sender: PlayerId,
    kind: MobKind,
    count: u8,
) {
    let targets: Vec<LaneId> = world
        .lanes
        .iter()
        .filter(|lane| !lane.eliminated && !lane.owners.is_empty() && lane.id != sender_lane)
        .map(|lane| lane.id)
        .collect();
    for lane in targets {
        for index in 0..u64::from(count) {
            world.pending.push(PendingSpawn {
                due_tick: world.elapsed + index * SEND_STAGGER_TICKS,
                lane,
                kind,
                sender,
            });
        }
    }
}

pub(crate) fn release_spawns(world: &mut World) {
    let elapsed = world.elapsed;
    let (due, later): (Vec<PendingSpawn>, Vec<PendingSpawn>) = std::mem::take(&mut world.pending)
        .into_iter()
        .partition(|spawn| spawn.due_tick <= elapsed);
    world.pending = later;
    for spawn in due {
        spawn_mob(world, &spawn);
    }
}

fn spawn_mob(world: &mut World, spawn: &PendingSpawn) {
    if world.lane(spawn.lane).eliminated {
        return;
    }
    let scale = 1.0 + SUDDEN_DEATH_HP_STEP * world.sudden_death_level as f32;
    let max_hp = spawn.kind.spec().hp * scale;
    let offset = world.rng.range(-LATERAL_RANGE, LATERAL_RANGE);
    let id = MobId(world.next_mob_id);
    world.next_mob_id += 1;
    let mob = Mob::new(id, spawn.kind, spawn.sender, max_hp, 0.0, offset);
    world.lane_mut(spawn.lane).mobs.push(mob);
    world.emit(Event::MobSpawned {
        lane: spawn.lane,
        mob: id,
        kind: spawn.kind,
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::domain::{
        GameMode, Gold, LANE_COUNT, LaneHp, LaneId, MobKind, PlayerId, SimError, Vec3,
    };
    use crate::map::Path;
    use crate::sim::lane::Lane;
    use crate::sim::rng::SplitMix64;
    use crate::sim::world::{Phase, PlayerState, World};

    use super::{recharge_stock, send};

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
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
                    LaneId::try_from(index).unwrap(),
                    owners,
                    LaneHp::full(20),
                    Arc::new(path),
                    Vec::new(),
                )
            })
            .collect();
        let mut rich = PlayerState::new(pid(0), LaneId::try_from(0).unwrap());
        rich.gold = Gold::new(1_000_000);
        World {
            mode: GameMode::Solo,
            players: vec![rich, PlayerState::new(pid(1), LaneId::try_from(1).unwrap())],
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

    fn charges(world: &World, kind: MobKind) -> u8 {
        world.player(pid(0)).unwrap().stock.charges[kind.stock_index().unwrap()]
    }

    #[test]
    fn spec_tw_4_the_eleventh_send_is_refused_and_a_charge_returns_after_30_ticks() {
        let mut world = world();
        for _ in 0..10 {
            assert_eq!(send(&mut world, pid(0), MobKind::Silverfish, 1), Ok(()));
        }
        assert!(world.player(pid(0)).unwrap().gold.get() >= 15);
        assert_eq!(
            send(&mut world, pid(0), MobKind::Silverfish, 1),
            Err(SimError::NoCharge)
        );
        for _ in 0..29 {
            recharge_stock(&mut world);
        }
        assert_eq!(charges(&world, MobKind::Silverfish), 0);
        recharge_stock(&mut world);
        assert_eq!(charges(&world, MobKind::Silverfish), 1);
        assert_eq!(send(&mut world, pid(0), MobKind::Silverfish, 1), Ok(()));
        assert_eq!(
            send(&mut world, pid(0), MobKind::Silverfish, 1),
            Err(SimError::NoCharge)
        );
    }

    #[test]
    fn spec_tw_5_tiers_three_and_four_are_accepted_exactly_at_their_tick() {
        for (kind, tick) in [(MobKind::GiantSlime, 6_600), (MobKind::Warden, 10_800)] {
            let mut world = world();
            world.elapsed = tick - 1;
            assert_eq!(
                send(&mut world, pid(0), kind, 1),
                Err(SimError::LockedTier),
                "{kind:?}"
            );
            world.elapsed = tick;
            assert_eq!(send(&mut world, pid(0), kind, 1), Ok(()), "{kind:?}");
        }
    }

    #[test]
    fn a_player_who_left_cannot_send() {
        let mut world = world();
        world.player_mut(pid(0)).unwrap().left = true;
        assert_eq!(
            send(&mut world, pid(0), MobKind::Silverfish, 1),
            Err(SimError::PlayerLeft)
        );
    }
}
