use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use rspigot_plugin::{BlockPos, BlockType, Error, GameRule, GameRuleSetting, Server, Weather, World};
use thiserror::Error;

use crate::domain::{LANE_COUNT, LaneId, Vec3};
use crate::map::{LaneMap, Map};

const CHUNK_SIZE: i32 = 16;
const WALL_MARGIN: i32 = 2;
const WALL_HEIGHT: i32 = 3;
const PATH_STEP: f32 = 0.25;
const LOBBY_Y: i32 = 100;
const LOBBY_RADIUS: i32 = 4;
const LOBBY_DISTANCE: i32 = 25;
const VOID_DEPTH: f64 = 40.0;
const BLOCK_CENTER: f64 = 0.5;

pub const FLOOR_BLOCK: BlockType = BlockType::Stone;
pub const PATH_BLOCK: BlockType = BlockType::DirtPath;
pub const BUILD_BLOCK_LIGHT: BlockType = BlockType::SmoothStone;
pub const BUILD_BLOCK_DARK: BlockType = BlockType::WhiteTerracotta;
pub const WALL_BLOCK: BlockType = BlockType::Barrier;
pub const LOBBY_BLOCK: BlockType = BlockType::SmoothStone;

type Cell = (i32, i32, i32);

#[derive(Debug, Error)]
pub enum ArenaError {
    #[error("impossible de créer le monde d'arène : {0}")]
    World(#[from] Error),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl From<Vec3> for Point {
    fn from(value: Vec3) -> Self {
        Self {
            x: f64::from(value.x),
            y: f64::from(value.y),
            z: f64::from(value.z),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    blocks: BTreeMap<Cell, BlockType>,
    lane_of: BTreeMap<Cell, usize>,
    anchors: [Option<Point>; LANE_COUNT],
    lobby: Point,
    void_y: f64,
}

fn floor_i(value: f32) -> i32 {
    value.floor() as i32
}

struct Extent {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

impl Extent {
    fn of(lane: &LaneMap) -> Self {
        let mut extent = Self {
            min_x: i32::MAX,
            max_x: i32::MIN,
            min_z: i32::MAX,
            max_z: i32::MIN,
        };
        let points = std::iter::once(&lane.spawn_point).chain(lane.waypoints.iter());
        for point in points {
            extent.include(floor_i(point.x), floor_i(point.z));
        }
        for bounds in &lane.build_bounds {
            extent.include(floor_i(bounds.min.x), floor_i(bounds.min.z));
            extent.include(floor_i(bounds.max.x), floor_i(bounds.max.z));
        }
        extent
    }

    fn include(&mut self, x: i32, z: i32) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_z = self.min_z.min(z);
        self.max_z = self.max_z.max(z);
    }
}

fn place_lane(blocks: &mut BTreeMap<Cell, BlockType>, lane: &LaneMap) {
    let extent = Extent::of(lane);
    let floor_y = floor_i(lane.spawn_point.y) - 1;
    for x in extent.min_x..=extent.max_x {
        for z in extent.min_z..=extent.max_z {
            blocks.insert((x, floor_y, z), FLOOR_BLOCK);
        }
    }
    for bounds in &lane.build_bounds {
        let y = floor_i(bounds.min.y) - 1;
        for x in floor_i(bounds.min.x)..=floor_i(bounds.max.x) {
            for z in floor_i(bounds.min.z)..=floor_i(bounds.max.z) {
                if bounds.contains_cell(x, z) {
                    let block = if (x + z).rem_euclid(2) == 0 {
                        BUILD_BLOCK_LIGHT
                    } else {
                        BUILD_BLOCK_DARK
                    };
                    blocks.insert((x, y, z), block);
                }
            }
        }
    }
    let points: Vec<Vec3> = std::iter::once(lane.spawn_point)
        .chain(lane.waypoints.iter().copied())
        .collect();
    for pair in points.windows(2) {
        let length = pair[0].distance(pair[1]);
        let steps = (length / PATH_STEP).ceil() as usize;
        for step in 0..=steps {
            let at = pair[0].lerp(pair[1], step as f32 / steps.max(1) as f32);
            blocks.insert(
                (floor_i(at.x), floor_i(at.y) - 1, floor_i(at.z)),
                PATH_BLOCK,
            );
        }
    }
    let (low_x, high_x) = (extent.min_x - WALL_MARGIN, extent.max_x + WALL_MARGIN);
    let (low_z, high_z) = (extent.min_z - WALL_MARGIN, extent.max_z + WALL_MARGIN);
    for x in low_x..=high_x {
        for z in low_z..=high_z {
            if x != low_x && x != high_x && z != low_z && z != high_z {
                continue;
            }
            for height in 0..WALL_HEIGHT {
                blocks.insert((x, floor_y + 1 + height, z), WALL_BLOCK);
            }
        }
    }
}

impl Layout {
    pub fn from_map(map: &Map) -> Self {
        let mut blocks = BTreeMap::new();
        let mut lane_of = BTreeMap::new();
        let mut anchors = [None; LANE_COUNT];
        let mut min_z = i32::MAX;
        let mut anchor_x = 0.0;
        let mut floor_y = i32::MAX;
        for lane in &map.lanes {
            let before: BTreeSet<Cell> = blocks.keys().copied().collect();
            place_lane(&mut blocks, lane);
            for cell in blocks.keys().filter(|cell| !before.contains(*cell)) {
                lane_of.insert(*cell, usize::from(lane.lane_id));
            }
            if let Some(slot) = anchors.get_mut(usize::from(lane.lane_id)) {
                *slot = Some(Point::from(lane.spectate_anchor));
            }
            min_z = min_z.min(Extent::of(lane).min_z - WALL_MARGIN);
            anchor_x += f64::from(lane.spectate_anchor.x);
            floor_y = floor_y.min(floor_i(lane.spawn_point.y) - 1);
        }
        let lanes = map.lanes.len().max(1) as f64;
        let center_x = (anchor_x / lanes).floor() as i32;
        let center_z = min_z.saturating_sub(LOBBY_DISTANCE);
        for x in center_x - LOBBY_RADIUS..=center_x + LOBBY_RADIUS {
            for z in center_z - LOBBY_RADIUS..=center_z + LOBBY_RADIUS {
                blocks.insert((x, LOBBY_Y, z), LOBBY_BLOCK);
            }
        }
        Self {
            blocks,
            lane_of,
            anchors,
            lobby: Point {
                x: f64::from(center_x) + BLOCK_CENTER,
                y: f64::from(LOBBY_Y) + 1.0,
                z: f64::from(center_z) + BLOCK_CENTER,
            },
            void_y: f64::from(floor_y) - VOID_DEPTH,
        }
    }

    pub fn blocks(&self) -> impl Iterator<Item = (Cell, BlockType)> + '_ {
        self.blocks.iter().map(|(cell, block)| (*cell, *block))
    }

    pub fn block_at(&self, cell: Cell) -> Option<BlockType> {
        self.blocks.get(&cell).copied()
    }

    pub fn blocks_in_chunk(&self, chunk_x: i32, chunk_z: i32) -> Vec<(Cell, BlockType)> {
        self.blocks()
            .filter(|(cell, _)| chunk_of(*cell) == (chunk_x, chunk_z))
            .collect()
    }

    pub fn blocks_in_chunk_for(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        enabled: &[bool; LANE_COUNT],
    ) -> Vec<(Cell, BlockType)> {
        self.blocks_in_chunk(chunk_x, chunk_z)
            .into_iter()
            .filter(|(cell, _)| self.is_enabled(cell, enabled))
            .collect()
    }

    pub fn lane_blocks(&self, lane: LaneId) -> Vec<Cell> {
        self.lane_of
            .iter()
            .filter(|(_, owner)| **owner == lane.index())
            .map(|(cell, _)| *cell)
            .collect()
    }

    pub fn lane_chunks(&self, lane: LaneId) -> BTreeSet<(i32, i32)> {
        self.lane_blocks(lane).into_iter().map(chunk_of).collect()
    }

    fn is_enabled(&self, cell: &Cell, enabled: &[bool; LANE_COUNT]) -> bool {
        self.lane_of
            .get(cell)
            .is_none_or(|lane| enabled.get(*lane).copied().unwrap_or(false))
    }

    pub fn chunks(&self) -> BTreeSet<(i32, i32)> {
        self.blocks.keys().map(|cell| chunk_of(*cell)).collect()
    }

    pub fn anchor(&self, lane: LaneId) -> Option<Point> {
        self.anchors.get(lane.index()).copied().flatten()
    }

    pub const fn lobby(&self) -> Point {
        self.lobby
    }

    pub fn is_void_fall(&self, y: f64) -> bool {
        y < self.void_y
    }
}

const fn chunk_of(cell: Cell) -> (i32, i32) {
    (cell.0.div_euclid(CHUNK_SIZE), cell.2.div_euclid(CHUNK_SIZE))
}

pub struct Arena {
    pub world: World,
    pub layout: Layout,
    pub reused: bool,
    enabled: Mutex<[bool; LANE_COUNT]>,
}

impl Arena {
    pub async fn create(server: &Server, layout: Layout) -> Result<Self, ArenaError> {
        let world = server.world.clone();
        world.time.set(6000);
        world.weather.set(Weather::Clear);
        world
            .game_rule
            .set(GameRuleSetting::new(GameRule::DaylightCycle, false));
        Ok(Self {
            world,
            layout,
            reused: true,
            enabled: Mutex::new([false; LANE_COUNT]),
        })
    }

    fn enabled(&self) -> [bool; LANE_COUNT] {
        *self.enabled.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn enable_lanes(&self, lanes: &[LaneId]) -> usize {
        {
            let mut enabled = self.enabled.lock().unwrap_or_else(PoisonError::into_inner);
            for lane in lanes {
                enabled[lane.index()] = true;
            }
        }
        for lane in lanes {
            for (x, z) in self.layout.lane_chunks(*lane) {
                let chunk = self.world.chunks.at(x, z);
                chunk.set_force_loaded(true);
                chunk.load();
            }
        }
        self.place_all()
    }

    pub fn clear_lanes(&self) {
        let enabled = {
            let mut flags = self.enabled.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *flags, [false; LANE_COUNT])
        };
        for lane in LaneId::ALL.into_iter().filter(|lane| enabled[lane.index()]) {
            for (x, y, z) in self.layout.lane_blocks(lane) {
                self.world.block(BlockPos { x, y, z }).set(BlockType::Air);
            }
            for (x, z) in self.layout.lane_chunks(lane) {
                self.world.chunks.at(x, z).set_force_loaded(false);
            }
        }
    }

    pub fn place_chunk(&self, chunk_x: i32, chunk_z: i32) -> usize {
        let placed = self
            .layout
            .blocks_in_chunk_for(chunk_x, chunk_z, &self.enabled());
        for ((x, y, z), kind) in &placed {
            self.world
                .block(BlockPos {
                    x: *x,
                    y: *y,
                    z: *z,
                })
                .set(*kind);
        }
        placed.len()
    }

    pub fn place_all(&self) -> usize {
        self.layout
            .chunks()
            .into_iter()
            .map(|(cx, cz)| self.place_chunk(cx, cz))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Aabb;

    fn layout() -> (Map, Layout) {
        let map = Map::test_map();
        let layout = Layout::from_map(&map);
        (map, layout)
    }

    fn lane(index: u8) -> LaneId {
        LaneId::try_from(index).expect("lane")
    }

    #[test]
    fn every_buildable_cell_has_a_surface_block_below_the_ground_level() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            for bounds in &lane_map.build_bounds {
                let y = bounds.min.y.floor() as i32 - 1;
                for x in bounds.min.x.floor() as i32..=bounds.max.x.floor() as i32 {
                    for z in bounds.min.z.floor() as i32..=bounds.max.z.floor() as i32 {
                        if !bounds.contains_cell(x, z) {
                            continue;
                        }
                        let block = layout.block_at((x, y, z));
                        assert!(
                            matches!(
                                block,
                                Some(BUILD_BLOCK_LIGHT | BUILD_BLOCK_DARK | PATH_BLOCK)
                            ),
                            "{x},{y},{z}: {block:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_surface_alternates_like_a_checkerboard() {
        let (_, layout) = layout();
        assert_eq!(layout.block_at((2, 63, 1)), Some(BUILD_BLOCK_DARK));
        assert_eq!(layout.block_at((3, 63, 1)), Some(BUILD_BLOCK_LIGHT));
        assert_eq!(layout.block_at((2, 63, 2)), Some(BUILD_BLOCK_LIGHT));
    }

    #[test]
    fn the_path_is_drawn_under_every_waypoint_and_between_them() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            let points: Vec<Vec3> = std::iter::once(lane_map.spawn_point)
                .chain(lane_map.waypoints.iter().copied())
                .collect();
            for point in &points {
                let cell = (
                    point.x.floor() as i32,
                    point.y.floor() as i32 - 1,
                    point.z.floor() as i32,
                );
                assert_eq!(layout.block_at(cell), Some(PATH_BLOCK), "{cell:?}");
            }
            let mid = points[0].lerp(points[1], 0.5);
            let cell = (mid.x.floor() as i32, 63, mid.z.floor() as i32);
            assert_eq!(layout.block_at(cell), Some(PATH_BLOCK));
        }
    }

    #[test]
    fn barrier_walls_enclose_each_lane_at_head_height() {
        let (map, layout) = layout();
        let lane_map = &map.lanes[0];
        let extent = Extent::of(lane_map);
        let (x, z) = (extent.min_x - WALL_MARGIN, extent.min_z + 3);
        for y in 64..64 + WALL_HEIGHT {
            assert_eq!(layout.block_at((x, y, z)), Some(WALL_BLOCK));
        }
        assert_eq!(layout.block_at((x, 64 + WALL_HEIGHT, z)), None);
        assert_eq!(
            layout.block_at((extent.min_x + 3, 64, extent.min_z + 3)),
            None
        );
    }

    #[test]
    fn two_lanes_never_share_a_wall_block() {
        let (map, _) = layout();
        let mut first = BTreeMap::new();
        place_lane(&mut first, &map.lanes[0]);
        let mut second = BTreeMap::new();
        place_lane(&mut second, &map.lanes[1]);
        assert!(first.keys().all(|cell| !second.contains_key(cell)));
    }

    #[test]
    fn spectate_anchors_float_above_their_lane() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            let anchor = layout.anchor(lane(lane_map.lane_id)).expect("anchor");
            assert!(anchor.y > f64::from(lane_map.spawn_point.y) + 5.0);
            assert_eq!(anchor.x, f64::from(lane_map.spectate_anchor.x));
        }
    }

    #[test]
    fn the_lobby_platform_is_far_from_every_lane_and_above_the_void_line() {
        let (map, layout) = layout();
        let lobby = layout.lobby();
        assert_eq!(
            layout.block_at((lobby.x as i32, LOBBY_Y, lobby.z as i32)),
            Some(LOBBY_BLOCK)
        );
        assert_eq!(lobby.y, f64::from(LOBBY_Y) + 1.0);
        assert!(!layout.is_void_fall(lobby.y));
        for lane_map in &map.lanes {
            let extent = Extent::of(lane_map);
            assert!(lobby.z < f64::from(extent.min_z - WALL_MARGIN));
        }
        assert!(layout.is_void_fall(0.0));
        assert!(!layout.is_void_fall(64.0));
    }

    #[test]
    fn chunks_together_equal_blocks_without_loss_or_duplicate() {
        let (_, layout) = layout();
        let mut gathered: Vec<Cell> = layout
            .chunks()
            .into_iter()
            .flat_map(|(x, z)| layout.blocks_in_chunk(x, z))
            .map(|(cell, _)| cell)
            .collect();
        let mut expected: Vec<Cell> = layout.blocks().map(|(cell, _)| cell).collect();
        gathered.sort_unstable();
        expected.sort_unstable();
        assert_eq!(gathered, expected);
    }

    #[test]
    fn blocks_land_in_their_own_chunk_negative_coordinates_included() {
        let mut map = Map::test_map();
        for lane_map in &mut map.lanes {
            lane_map.spawn_point.x -= 100.0;
            for point in &mut lane_map.waypoints {
                point.x -= 100.0;
            }
            for bounds in &mut lane_map.build_bounds {
                *bounds = Aabb {
                    min: Vec3::new(bounds.min.x - 100.0, bounds.min.y, bounds.min.z),
                    max: Vec3::new(bounds.max.x - 100.0, bounds.max.y, bounds.max.z),
                };
            }
            lane_map.spectate_anchor.x -= 100.0;
        }
        let layout = Layout::from_map(&map);
        let mut seen_negative = false;
        for (chunk_x, chunk_z) in layout.chunks() {
            for (cell, _) in layout.blocks_in_chunk(chunk_x, chunk_z) {
                assert_eq!(cell.0.div_euclid(CHUNK_SIZE), chunk_x);
                assert_eq!(cell.2.div_euclid(CHUNK_SIZE), chunk_z);
                seen_negative |= cell.0 < 0;
            }
        }
        assert!(seen_negative);
    }

    #[test]
    fn an_empty_chunk_has_no_block() {
        let (_, layout) = layout();
        assert!(layout.blocks_in_chunk(10_000, 10_000).is_empty());
    }

    #[test]
    fn only_enabled_lanes_contribute_blocks_to_a_chunk() {
        let (map, layout) = layout();
        let spawn = map.lanes[0].spawn_point;
        let (cx, cz) = (
            (spawn.x.floor() as i32).div_euclid(CHUNK_SIZE),
            (spawn.z.floor() as i32).div_euclid(CHUNK_SIZE),
        );
        let mut enabled = [false; LANE_COUNT];
        assert!(layout.blocks_in_chunk_for(cx, cz, &enabled).is_empty());
        enabled[0] = true;
        let placed = layout.blocks_in_chunk_for(cx, cz, &enabled);
        assert!(!placed.is_empty());
        assert!(
            placed
                .iter()
                .all(|(cell, _)| layout.lane_blocks(lane(0)).contains(cell))
        );
    }

    #[test]
    fn lobby_blocks_never_depend_on_enabled_lanes() {
        let (_, layout) = layout();
        let lobby = layout.lobby();
        let (cx, cz) = (
            (lobby.x.floor() as i32).div_euclid(CHUNK_SIZE),
            (lobby.z.floor() as i32).div_euclid(CHUNK_SIZE),
        );
        let none = [false; LANE_COUNT];
        assert!(!layout.blocks_in_chunk_for(cx, cz, &none).is_empty());
    }
}
