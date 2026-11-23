use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::{
    Aabb, Branch, LaneHp, LaneId, Level, MobId, MobKind, PlayerId, Priority, SimError, Statuses,
    TOWER_LIMIT, TowerId, TowerKind, Vec3,
};
use crate::map::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Mob {
    pub id: MobId,
    pub kind: MobKind,
    pub sender: PlayerId,
    pub hp: f32,
    pub max_hp: f32,
    pub progress: f32,
    pub lateral_offset: f32,
    pub statuses: Statuses,
    pub blinked: bool,
    pub last_hit_by: Option<PlayerId>,
}

impl Mob {
    pub fn new(
        id: MobId,
        kind: MobKind,
        sender: PlayerId,
        max_hp: f32,
        progress: f32,
        lateral_offset: f32,
    ) -> Self {
        Self {
            id,
            kind,
            sender,
            hp: max_hp,
            max_hp,
            progress,
            lateral_offset,
            statuses: Statuses::default(),
            blinked: false,
            last_hit_by: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tower {
    pub id: TowerId,
    pub kind: TowerKind,
    pub level: Level,
    pub cell: (i32, i32),
    pub world_pos: Vec3,
    pub owner: PlayerId,
    pub priority: Priority,
    pub cooldown_ticks: u16,
    pub invested: u32,
    pub stun_remaining: u16,
    pub shots: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pool {
    pub center: Vec3,
    pub radius: f32,
    pub remaining: u16,
    pub dps: f32,
    pub armor: f32,
    pub owner: PlayerId,
}

#[derive(Debug, Clone)]
pub struct Lane {
    pub id: LaneId,
    pub owners: Vec<PlayerId>,
    pub hp: LaneHp,
    pub mobs: Vec<Mob>,
    pub eliminated: bool,
    pub pools: Vec<Pool>,
    path: Arc<Path>,
    bounds: Vec<Aabb>,
    towers: Vec<Tower>,
    grid: HashMap<(i32, i32), TowerId>,
}

impl Lane {
    pub fn new(
        id: LaneId,
        owners: Vec<PlayerId>,
        hp: LaneHp,
        path: Arc<Path>,
        bounds: Vec<Aabb>,
    ) -> Self {
        Self {
            id,
            owners,
            hp,
            mobs: Vec::new(),
            eliminated: false,
            pools: Vec::new(),
            path,
            bounds,
            towers: Vec::new(),
            grid: HashMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn towers(&self) -> &[Tower] {
        &self.towers
    }

    pub fn tower(&self, id: TowerId) -> Option<&Tower> {
        self.towers.iter().find(|tower| tower.id == id)
    }

    pub fn tower_mut(&mut self, id: TowerId) -> Option<&mut Tower> {
        self.towers.iter_mut().find(|tower| tower.id == id)
    }

    pub fn towers_mut(&mut self) -> &mut [Tower] {
        &mut self.towers
    }

    pub fn tower_at(&self, cell: (i32, i32)) -> Option<TowerId> {
        self.grid.get(&cell).copied()
    }

    pub fn is_buildable(&self, cell: (i32, i32)) -> bool {
        self.bounds
            .iter()
            .any(|bounds| bounds.contains_cell(cell.0, cell.1))
    }

    pub fn place(&mut self, tower: Tower) -> Result<(), SimError> {
        if self.towers.len() >= TOWER_LIMIT {
            return Err(SimError::LaneFull);
        }
        if !self.is_buildable(tower.cell) {
            return Err(SimError::OutOfBuildBounds);
        }
        if self.grid.contains_key(&tower.cell) {
            return Err(SimError::OccupiedCell);
        }
        self.grid.insert(tower.cell, tower.id);
        self.towers.push(tower);
        Ok(())
    }

    pub fn remove(&mut self, id: TowerId) -> Option<Tower> {
        let index = self.towers.iter().position(|tower| tower.id == id)?;
        let tower = self.towers.remove(index);
        self.grid.remove(&tower.cell);
        Some(tower)
    }

    pub fn surface_y(&self) -> f32 {
        self.bounds.first().map_or(0.0, |bounds| bounds.min.y)
    }

    pub fn gold_mines_owned(&self, player: PlayerId) -> usize {
        self.towers
            .iter()
            .filter(|tower| {
                tower.owner == player
                    && tower.kind == TowerKind::Beacon
                    && tower.level == Level::T4(Branch::B)
            })
            .count()
    }

    pub fn mob_position(&self, mob: &Mob) -> Vec3 {
        let mut position = self.path.sample(mob.progress).position;
        position.x += mob.lateral_offset;
        position
    }
}
