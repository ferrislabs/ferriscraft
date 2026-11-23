use crate::domain::{
    GameMode, Gold, LaneId, MobKind, PlayerId, SENDABLE_MOBS, STARTING_GOLD, STARTING_INCOME,
    STOCK_MAX,
};

use super::event::Event;
use super::lane::Lane;
use super::rng::SplitMix64;

#[derive(Debug, Clone)]
pub struct MobStock {
    pub charges: [u8; SENDABLE_MOBS],
    pub recharge: [u16; SENDABLE_MOBS],
}

impl Default for MobStock {
    fn default() -> Self {
        Self {
            charges: [STOCK_MAX; SENDABLE_MOBS],
            recharge: [0; SENDABLE_MOBS],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub id: PlayerId,
    pub lane: LaneId,
    pub gold: Gold,
    pub income: u32,
    pub stock: MobStock,
    pub left: bool,
}

impl PlayerState {
    pub fn new(id: PlayerId, lane: LaneId) -> Self {
        Self {
            id,
            lane,
            gold: Gold::new(STARTING_GOLD),
            income: STARTING_INCOME,
            stock: MobStock::default(),
            left: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingSpawn {
    pub due_tick: u64,
    pub lane: LaneId,
    pub kind: MobKind,
    pub sender: PlayerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Ended { winner: Option<LaneId> },
}

#[derive(Debug)]
pub struct World {
    pub mode: GameMode,
    pub players: Vec<PlayerState>,
    pub lanes: Vec<Lane>,
    pub elapsed: u64,
    pub rng: SplitMix64,
    pub next_mob_id: u32,
    pub next_tower_id: u32,
    pub pending: Vec<PendingSpawn>,
    pub events: Vec<Event>,
    pub phase: Phase,
    pub sudden_death_level: u32,
}

impl World {
    pub fn player(&self, id: PlayerId) -> Option<&PlayerState> {
        self.players.iter().find(|player| player.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut PlayerState> {
        self.players.iter_mut().find(|player| player.id == id)
    }

    pub fn lane(&self, id: LaneId) -> &Lane {
        &self.lanes[id.index()]
    }

    pub fn lane_mut(&mut self, id: LaneId) -> &mut Lane {
        &mut self.lanes[id.index()]
    }

    pub fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}
