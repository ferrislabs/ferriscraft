use std::sync::Arc;

use thiserror::Error;

use crate::domain::{
    Branch, GameMode, Gold, LANE_COUNT, LaneHp, LaneId, MobKind, PlayerId, Priority, SimError,
    TowerId, TowerKind,
};
use crate::map::{Map, MapError};

use super::event::Event;
use super::lane::Lane;
use super::rng::SplitMix64;
use super::world::{Phase, PlayerState, World};
use super::{combat, economy, mobs, towers};

#[derive(Debug, Error)]
pub enum MatchError {
    #[error(transparent)]
    Sim(#[from] SimError),
    #[error(transparent)]
    Map(#[from] MapError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub player: PlayerId,
    pub lane: LaneId,
}

#[derive(Debug)]
pub struct Match {
    world: World,
}

impl Match {
    pub fn new(mode: GameMode, seed: u64, map: &Map, seats: &[Seat]) -> Result<Self, MatchError> {
        if seats.len() < mode.min_players() {
            return Err(SimError::NotEnoughPlayers.into());
        }
        if seats.len() > mode.max_players() {
            return Err(SimError::InvalidSeating.into());
        }
        let mut owners: Vec<Vec<PlayerId>> = vec![Vec::new(); LANE_COUNT];
        for seat in seats {
            let lane_owners = &mut owners[seat.lane.index()];
            if lane_owners.len() >= mode.owners_per_lane() || lane_owners.contains(&seat.player) {
                return Err(SimError::InvalidSeating.into());
            }
            lane_owners.push(seat.player);
        }
        let mut lanes = Vec::with_capacity(LANE_COUNT);
        for id in LaneId::ALL {
            let path = Arc::new(map.path_of(id)?);
            let bounds = map
                .lane(id)
                .map(|lane| lane.build_bounds.clone())
                .unwrap_or_default();
            let mut lane = Lane::new(
                id,
                owners[id.index()].clone(),
                LaneHp::full(mode.starting_hp()),
                path,
                bounds,
            );
            lane.eliminated = lane.owners.is_empty();
            lanes.push(lane);
        }
        let players = seats
            .iter()
            .map(|seat| PlayerState::new(seat.player, seat.lane))
            .collect();
        Ok(Self {
            world: World {
                mode,
                players,
                lanes,
                elapsed: 0,
                rng: SplitMix64::new(seed),
                next_mob_id: 0,
                next_tower_id: 0,
                pending: Vec::new(),
                events: Vec::new(),
                phase: Phase::Running,
                sudden_death_level: 0,
            },
        })
    }

    pub fn tick(&mut self) {
        if self.is_ended() {
            return;
        }
        self.world.elapsed += 1;
        economy::pay_income(&mut self.world);
        economy::recharge_stock(&mut self.world);
        economy::unlock_tiers(&mut self.world);
        mobs::update_sudden_death(&mut self.world);
        economy::release_spawns(&mut self.world);
        combat::step_statuses(&mut self.world);
        combat::step_towers(&mut self.world);
        combat::step_abilities(&mut self.world);
        mobs::advance(&mut self.world);
        mobs::reap(&mut self.world);
        mobs::resolve_arrivals(&mut self.world);
        self.resolve_eliminations();
    }

    pub fn send(&mut self, player: PlayerId, kind: MobKind, count: u8) -> Result<(), SimError> {
        self.ensure_running()?;
        economy::send(&mut self.world, player, kind, count)
    }

    pub fn place(
        &mut self,
        player: PlayerId,
        kind: TowerKind,
        cell: (i32, i32),
    ) -> Result<TowerId, SimError> {
        self.ensure_running()?;
        towers::place(&mut self.world, player, kind, cell)
    }

    pub fn upgrade(
        &mut self,
        player: PlayerId,
        tower: TowerId,
        branch: Option<Branch>,
    ) -> Result<(), SimError> {
        self.ensure_running()?;
        towers::upgrade(&mut self.world, player, tower, branch)
    }

    pub fn sell(&mut self, player: PlayerId, tower: TowerId) -> Result<u32, SimError> {
        self.ensure_running()?;
        towers::sell(&mut self.world, player, tower)
    }

    pub fn set_priority(
        &mut self,
        player: PlayerId,
        tower: TowerId,
        priority: Priority,
    ) -> Result<(), SimError> {
        self.ensure_running()?;
        towers::set_priority(&mut self.world, player, tower, priority)
    }

    pub fn forfeit(&mut self, player: PlayerId) {
        let Some(state) = self.world.player_mut(player) else {
            return;
        };
        state.left = true;
        let lane = state.lane;
        let everyone_left = self
            .world
            .players
            .iter()
            .filter(|other| other.lane == lane)
            .all(|other| other.left);
        if everyone_left {
            let hp = self.world.lane(lane).hp.current();
            self.world.lane_mut(lane).hp.damage(hp);
        }
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.world.events)
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn elapsed(&self) -> u64 {
        self.world.elapsed
    }

    pub fn phase(&self) -> Phase {
        self.world.phase
    }

    pub fn is_ended(&self) -> bool {
        matches!(self.world.phase, Phase::Ended { .. })
    }

    pub fn gold(&self, player: PlayerId) -> Option<Gold> {
        self.world.player(player).map(|state| state.gold)
    }

    pub fn income(&self, player: PlayerId) -> Option<u32> {
        self.world.player(player).map(|state| state.income)
    }

    pub fn lane(&self, lane: LaneId) -> &Lane {
        self.world.lane(lane)
    }

    fn ensure_running(&self) -> Result<(), SimError> {
        if self.is_ended() {
            Err(SimError::MatchEnded)
        } else {
            Ok(())
        }
    }

    fn resolve_eliminations(&mut self) {
        for index in 0..self.world.lanes.len() {
            let lane = &mut self.world.lanes[index];
            if lane.eliminated || !lane.hp.is_dead() {
                continue;
            }
            lane.eliminated = true;
            lane.mobs.clear();
            let id = lane.id;
            self.world.pending.retain(|spawn| spawn.lane != id);
            self.world.emit(Event::LaneEliminated { lane: id });
        }
        let alive: Vec<LaneId> = self
            .world
            .lanes
            .iter()
            .filter(|lane| !lane.eliminated)
            .map(|lane| lane.id)
            .collect();
        if alive.len() <= 1 {
            let winner = alive.first().copied();
            self.world.phase = Phase::Ended { winner };
            self.world.emit(Event::Won { lane: winner });
        }
    }
}
