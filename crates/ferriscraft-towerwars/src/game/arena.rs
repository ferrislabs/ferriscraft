use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use rspigot_plugin::{
    BlockPos, BlockType, Error, GameRule, GameRuleSetting, Server, Weather, World,
};
use thiserror::Error;

use super::imperial::{self, Stadium};
use crate::domain::{LANE_COUNT, LaneId, Vec3};
use crate::map::{LaneMap, Map};

const CHUNK_SIZE: i32 = 16;
const TRENCH_FLOOR_REACH: i32 = 0;
const TRENCH_REACH: i32 = 1;
const RIM_WIDTH: i32 = 2;
const WALKWAY_MARGIN: i32 = 3;
const PANEL_LIFT: f64 = 7.0;
const OUT_OF_BOUNDS_MARGIN: i32 = 4;
const LOBBY_Y: i32 = 100;
const LOBBY_RADIUS: i32 = 4;
const LOBBY_DISTANCE: i32 = 25;
const VOID_DEPTH: f64 = 40.0;
const BLOCK_CENTER: f64 = 0.5;

pub const PATH_BLOCK: BlockType = BlockType::PolishedAndesite;
pub const PATH_EDGE_BLOCK: BlockType = BlockType::Water;
pub const TRENCH_BED_BLOCK: BlockType = BlockType::Stone;
pub const SPAWN_BLOCK: BlockType = BlockType::LimeConcrete;
pub const EXIT_BLOCK: BlockType = BlockType::RedConcrete;
pub const BUILD_BLOCK_LIGHT: BlockType = BlockType::RedTerracotta;
pub const BUILD_BLOCK_DARK: BlockType = BlockType::GreenTerracotta;
pub const PLATEAU_BLOCK: BlockType = BlockType::MossBlock;
pub const SOIL_BLOCK: BlockType = BlockType::StoneBricks;
pub const RIM_BLOCK: BlockType = BlockType::ChiseledStoneBricks;
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
    panels: [Option<Point>; LANE_COUNT],
    lobby: Point,
    void_y: f64,
    bounds: Option<Extent>,
}

fn floor_i(value: f32) -> i32 {
    value.floor() as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
}

impl Extent {
    const EMPTY: Self = Self {
        min_x: i32::MAX,
        max_x: i32::MIN,
        min_z: i32::MAX,
        max_z: i32::MIN,
    };

    fn of(lane: &LaneMap) -> Self {
        let mut extent = Self::EMPTY;
        let points = std::iter::once(&lane.spawn_point).chain(lane.waypoints.iter());
        for point in points {
            extent.include(floor_i(point.x), floor_i(point.z));
        }
        for bounds in &lane.build_bounds {
            extent.include(floor_i(bounds.min.x), floor_i(bounds.min.z));
            extent.include(floor_i(bounds.max.x - 0.5), floor_i(bounds.max.z - 0.5));
        }
        extent
    }

    fn include(&mut self, x: i32, z: i32) {
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_z = self.min_z.min(z);
        self.max_z = self.max_z.max(z);
    }

    fn merge(&mut self, other: Self) {
        self.include(other.min_x, other.min_z);
        self.include(other.max_x, other.max_z);
    }

    pub const fn grown(self, by: i32) -> Self {
        Self {
            min_x: self.min_x - by,
            max_x: self.max_x + by,
            min_z: self.min_z - by,
            max_z: self.max_z + by,
        }
    }

    pub fn ring(&self, x: i32, z: i32) -> i32 {
        let dx = (self.min_x - x).max(x - self.max_x).max(0);
        let dz = (self.min_z - z).max(z - self.max_z).max(0);
        dx.max(dz)
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        self.ring(x, z) == 0
    }

    pub fn ring_inner(&self, x: i32, z: i32) -> i32 {
        (x - self.min_x)
            .min(self.max_x - x)
            .min(z - self.min_z)
            .min(self.max_z - z)
    }
}

fn path_cells(lane: &LaneMap) -> Vec<(i32, i32)> {
    std::iter::once(&lane.spawn_point)
        .chain(lane.waypoints.iter())
        .map(|point| (floor_i(point.x), floor_i(point.z)))
        .collect()
}

fn distance_to_path(path: &[(i32, i32)], x: i32, z: i32) -> i32 {
    path.windows(2)
        .map(|pair| {
            let (low_x, high_x) = (pair[0].0.min(pair[1].0), pair[0].0.max(pair[1].0));
            let (low_z, high_z) = (pair[0].1.min(pair[1].1), pair[0].1.max(pair[1].1));
            let dx = (low_x - x).max(x - high_x).max(0);
            let dz = (low_z - z).max(z - high_z).max(0);
            dx.max(dz)
        })
        .min()
        .unwrap_or(i32::MAX)
}

fn is_buildable(lane: &LaneMap, x: i32, z: i32) -> bool {
    lane.build_bounds
        .iter()
        .any(|bounds| bounds.contains_cell(x, z))
}

fn trench_y(lane: &LaneMap) -> i32 {
    floor_i(lane.spawn_point.y) - 1
}

fn plateau_y(lane: &LaneMap) -> i32 {
    lane.build_bounds
        .first()
        .map_or(trench_y(lane) + 1, |bounds| floor_i(bounds.min.y) - 1)
}

fn place_lane(blocks: &mut BTreeMap<Cell, BlockType>, lane: &LaneMap) {
    let extent = Extent::of(lane);
    let path = path_cells(lane);
    let (trench_y, plateau_y) = (trench_y(lane), plateau_y(lane));
    let (spawn, exit) = (path[0], path[path.len() - 1]);
    for x in extent.min_x - RIM_WIDTH..=extent.max_x + RIM_WIDTH {
        for z in extent.min_z - RIM_WIDTH..=extent.max_z + RIM_WIDTH {
            if extent.ring(x, z) > 0 {
                for y in trench_y..=plateau_y {
                    blocks.insert((x, y, z), SOIL_BLOCK);
                }
                blocks.insert((x, plateau_y + 1, z), RIM_BLOCK);
                continue;
            }
            let reach = distance_to_path(&path, x, z);
            if reach <= TRENCH_REACH {
                let block = if (x, z) == spawn {
                    SPAWN_BLOCK
                } else if (x, z) == exit {
                    EXIT_BLOCK
                } else if reach <= TRENCH_FLOOR_REACH {
                    PATH_BLOCK
                } else {
                    PATH_EDGE_BLOCK
                };
                blocks.insert((x, trench_y, z), block);
                blocks.insert((x, trench_y - 1, z), TRENCH_BED_BLOCK);
                continue;
            }
            if !is_buildable(lane, x, z) {
                blocks.insert((x, trench_y, z), SOIL_BLOCK);
                blocks.insert((x, trench_y + 1, z), PLATEAU_BLOCK);
                continue;
            }
            let surface = if (x + z).rem_euclid(2) == 0 {
                BUILD_BLOCK_LIGHT
            } else {
                BUILD_BLOCK_DARK
            };
            blocks.insert((x, plateau_y, z), surface);
            for y in trench_y..plateau_y {
                blocks.insert((x, y, z), SOIL_BLOCK);
            }
        }
    }
}

fn place_stadium(
    blocks: &mut BTreeMap<Cell, BlockType>,
    lanes: &[Extent],
    floor_y: i32,
    decor: bool,
) -> Option<Stadium> {
    let mut inside = *lanes.first()?;
    for extent in lanes {
        inside.merge(*extent);
    }
    let inside = inside.grown(RIM_WIDTH + WALKWAY_MARGIN);
    let footprints: Vec<Extent> = lanes.iter().map(|extent| extent.grown(RIM_WIDTH)).collect();
    Some(imperial::build(blocks, inside, &footprints, floor_y, decor))
}

impl Layout {
    pub fn from_map(map: &Map) -> Self {
        Self::from_map_with(map, true)
    }

    pub fn from_map_with(map: &Map, decor: bool) -> Self {
        let mut blocks = BTreeMap::new();
        let mut lane_of = BTreeMap::new();
        let mut anchors = [None; LANE_COUNT];
        let mut panels = [None; LANE_COUNT];
        let mut extents = Vec::new();
        let mut anchor_x = 0.0;
        let mut floor_y = i32::MAX;
        for lane in &map.lanes {
            let before: BTreeSet<Cell> = blocks.keys().copied().collect();
            place_lane(&mut blocks, lane);
            for cell in blocks.keys().filter(|cell| !before.contains(*cell)) {
                lane_of.insert(*cell, usize::from(lane.lane_id));
            }
            let extent = Extent::of(lane);
            let index = usize::from(lane.lane_id);
            if let Some(slot) = anchors.get_mut(index) {
                *slot = Some(Point::from(lane.spectate_anchor));
            }
            if let Some(slot) = panels.get_mut(index) {
                *slot = Some(Point {
                    x: f64::from(lane.spectate_anchor.x),
                    y: f64::from(plateau_y(lane)) + 1.0 + PANEL_LIFT,
                    z: f64::from(extent.min_z - RIM_WIDTH) + BLOCK_CENTER,
                });
            }
            extents.push(extent);
            anchor_x += f64::from(lane.spectate_anchor.x);
            floor_y = floor_y.min(trench_y(lane) + 1);
        }
        let stadium = place_stadium(&mut blocks, &extents, floor_y, decor);
        let min_z = stadium.as_ref().map_or(0, |stadium| stadium.outer.min_z);
        let lanes = map.lanes.len().max(1) as f64;
        let center_x = (anchor_x / lanes).floor() as i32;
        let center_z = min_z.saturating_sub(LOBBY_DISTANCE);
        for x in center_x - LOBBY_RADIUS..=center_x + LOBBY_RADIUS {
            for z in center_z - LOBBY_RADIUS..=center_z + LOBBY_RADIUS {
                blocks.insert((x, LOBBY_Y, z), LOBBY_BLOCK);
            }
        }
        let lowest = map.lanes.iter().map(trench_y).min().unwrap_or(floor_y);
        Self {
            blocks,
            lane_of,
            anchors,
            panels,
            lobby: Point {
                x: f64::from(center_x) + BLOCK_CENTER,
                y: f64::from(LOBBY_Y) + 1.0,
                z: f64::from(center_z) + BLOCK_CENTER,
            },
            void_y: f64::from(lowest) - VOID_DEPTH,
            bounds: stadium.as_ref().map(|stadium| {
                let mut bounds = stadium.outer.grown(OUT_OF_BOUNDS_MARGIN);
                let reach = LOBBY_RADIUS + OUT_OF_BOUNDS_MARGIN;
                bounds.include(center_x - reach, center_z - reach);
                bounds.include(center_x + reach, center_z + reach);
                bounds
            }),
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

    pub fn lane_blocks(&self, lane: LaneId) -> Vec<Cell> {
        self.lane_of
            .iter()
            .filter(|(_, owner)| **owner == lane.index())
            .map(|(cell, _)| *cell)
            .collect()
    }

    pub fn chunks(&self) -> BTreeSet<(i32, i32)> {
        self.blocks.keys().map(|cell| chunk_of(*cell)).collect()
    }

    pub fn anchor(&self, lane: LaneId) -> Option<Point> {
        self.anchors.get(lane.index()).copied().flatten()
    }

    pub fn panel(&self, lane: LaneId) -> Option<Point> {
        self.panels.get(lane.index()).copied().flatten()
    }

    pub const fn lobby(&self) -> Point {
        self.lobby
    }

    pub fn is_outside(&self, x: f64, z: f64) -> bool {
        self.bounds
            .is_some_and(|bounds| !bounds.contains(x.floor() as i32, z.floor() as i32))
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
    placed: Mutex<BTreeSet<(i32, i32)>>,
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
            placed: Mutex::new(BTreeSet::new()),
        })
    }

    pub async fn prepare(&self, prebuilt: bool) -> usize {
        let chunks = self.layout.chunks();
        for (x, z) in &chunks {
            let chunk = self.world.chunks.at(*x, *z);
            chunk.set_force_loaded(true);
            chunk.load();
        }
        if prebuilt {
            self.placed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(chunks);
            return 0;
        }
        let mut placed = 0;
        for (x, z) in chunks {
            if self
                .world
                .chunks
                .at(x, z)
                .is_loaded()
                .await
                .unwrap_or(false)
            {
                placed += self.place_once(x, z);
            }
        }
        placed
    }

    pub fn place_once(&self, chunk_x: i32, chunk_z: i32) -> usize {
        let fresh = self
            .placed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((chunk_x, chunk_z));
        if fresh {
            self.place_chunk(chunk_x, chunk_z)
        } else {
            0
        }
    }

    pub fn pending_chunks(&self) -> usize {
        let placed = self.placed.lock().unwrap_or_else(PoisonError::into_inner);
        self.layout
            .chunks()
            .iter()
            .filter(|chunk| !placed.contains(chunk))
            .count()
    }

    fn place_chunk(&self, chunk_x: i32, chunk_z: i32) -> usize {
        let placed = self.layout.blocks_in_chunk(chunk_x, chunk_z);
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
                            matches!(block, Some(BUILD_BLOCK_LIGHT | BUILD_BLOCK_DARK)),
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
        assert_eq!(layout.block_at((2, 64, 4)), Some(BUILD_BLOCK_LIGHT));
        assert_eq!(layout.block_at((3, 64, 4)), Some(BUILD_BLOCK_DARK));
        assert_eq!(layout.block_at((2, 64, 3)), Some(BUILD_BLOCK_DARK));
    }

    #[test]
    fn the_path_runs_between_walls_two_blocks_high() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            let points: Vec<Vec3> = std::iter::once(lane_map.spawn_point)
                .chain(lane_map.waypoints.iter().copied())
                .collect();
            let trench_y = points[0].y.floor() as i32 - 1;
            for point in &points {
                let cell = (point.x.floor() as i32, trench_y, point.z.floor() as i32);
                assert!(
                    matches!(
                        layout.block_at(cell),
                        Some(PATH_BLOCK | SPAWN_BLOCK | EXIT_BLOCK)
                    ),
                    "{cell:?}"
                );
                let above = (cell.0, trench_y + 1, cell.2);
                assert_eq!(layout.block_at(above), None, "{above:?}");
            }
            let mid = points[0].lerp(points[1], 0.5);
            let (x, z) = (mid.x.floor() as i32, mid.z.floor() as i32);
            assert_eq!(layout.block_at((x, trench_y, z)), Some(PATH_BLOCK));
            assert_eq!(layout.block_at((x, trench_y, z + 1)), Some(PATH_EDGE_BLOCK));
            for dz in 2..=4 {
                assert!(matches!(
                    layout.block_at((x, trench_y + 2, z + dz)),
                    Some(BUILD_BLOCK_LIGHT | BUILD_BLOCK_DARK)
                ));
            }
        }
    }

    #[test]
    fn the_entrance_and_the_exit_are_marked() {
        let (map, layout) = layout();
        let lane_map = &map.lanes[0];
        let spawn = lane_map.spawn_point;
        let exit = lane_map.waypoints[lane_map.waypoints.len() - 1];
        let y = spawn.y.floor() as i32 - 1;
        assert_eq!(
            layout.block_at((spawn.x.floor() as i32, y, spawn.z.floor() as i32)),
            Some(SPAWN_BLOCK)
        );
        assert_eq!(
            layout.block_at((exit.x.floor() as i32, y, exit.z.floor() as i32)),
            Some(EXIT_BLOCK)
        );
    }

    fn stadium(map: &Map) -> Extent {
        let mut all = Extent::of(&map.lanes[0]);
        for lane_map in &map.lanes {
            all.merge(Extent::of(lane_map));
        }
        all.grown(RIM_WIDTH + WALKWAY_MARGIN)
    }

    #[test]
    fn every_lane_has_a_rim_above_its_walls() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            let extent = Extent::of(lane_map);
            let z = extent.min_z + 4;
            assert_eq!(layout.block_at((extent.min_x - 1, 65, z)), Some(RIM_BLOCK));
            assert_eq!(layout.block_at((extent.max_x + 2, 65, z)), Some(RIM_BLOCK));
        }
    }

    #[test]
    fn walkways_and_channels_join_the_lanes_without_any_wall_between_them() {
        let (map, layout) = layout();
        let first = Extent::of(&map.lanes[0]);
        let second = Extent::of(&map.lanes[1]);
        let z = first.min_z + 4;
        let gap: Vec<Option<BlockType>> = (first.max_x + RIM_WIDTH + 1..second.min_x - RIM_WIDTH)
            .map(|x| layout.block_at((x, 63, z)))
            .collect();
        assert_eq!(gap.len(), 3);
        assert_eq!(gap[1], Some(imperial::WATER));
        for side in [gap[0], gap[2]] {
            assert!(matches!(
                side,
                Some(imperial::WALKWAY | imperial::WALKWAY_ACCENT)
            ));
        }
        for x in first.max_x + RIM_WIDTH + 1..second.min_x - RIM_WIDTH {
            for y in 64..70 {
                assert_eq!(layout.block_at((x, y, z)), None, "{x},{y}");
            }
        }
    }

    #[test]
    fn every_water_block_is_enclosed_so_it_never_flows() {
        let (_, layout) = layout();
        let solid_or_water = |cell: Cell| layout.block_at(cell).is_some();
        for ((x, y, z), block) in layout.blocks() {
            if block != BlockType::Water {
                continue;
            }
            for neighbour in [
                (x + 1, y, z),
                (x - 1, y, z),
                (x, y, z + 1),
                (x, y, z - 1),
                (x, y - 1, z),
            ] {
                assert!(
                    solid_or_water(neighbour),
                    "eau en ({x}, {y}, {z}) ouverte vers {neighbour:?}"
                );
            }
        }
    }

    #[test]
    fn three_tier_palaces_face_the_arena_behind_a_terrace() {
        let (map, layout) = layout();
        let inside = stadium(&map);
        let z = inside.min_z + 7;
        assert_eq!(
            layout.block_at((inside.min_x - 1, 64, z)),
            Some(imperial::WALKWAY)
        );
        let facade = inside.min_x - imperial::TERRACE_ROWS - 1;
        assert!(layout.block_at((facade, 67, z)).is_some());
        let ridge_x = inside.min_x - imperial::TERRACE_ROWS - 7;
        assert_eq!(layout.block_at((ridge_x, 89, z)), Some(imperial::RIDGE));
    }

    #[test]
    fn four_corner_towers_rise_above_everything() {
        let (map, layout) = layout();
        let inside = stadium(&map);
        let reach = imperial::TERRACE_ROWS + imperial::PALACE_DEPTH;
        let corners = [
            (inside.min_x - reach, inside.min_z - reach),
            (inside.max_x + 1, inside.max_z + 1),
        ];
        for (x0, z0) in corners {
            let top = (x0..x0 + reach)
                .flat_map(|x| (z0..z0 + reach).map(move |z| (x, z)))
                .flat_map(|(x, z)| (64..160).map(move |y| (x, y, z)))
                .filter(|cell| layout.block_at(*cell) == Some(imperial::RIDGE))
                .map(|(_, y, _)| y)
                .max();
            assert!(top.is_some_and(|y| y > 100), "{top:?}");
        }
    }

    #[test]
    fn leaving_the_stadium_is_detected_but_not_the_lobby() {
        let (map, layout) = layout();
        let inside = stadium(&map);
        let (mid_x, mid_z) = (
            f64::from(inside.min_x + inside.max_x) / 2.0,
            f64::from(inside.min_z + inside.max_z) / 2.0,
        );
        assert!(!layout.is_outside(mid_x, mid_z));
        assert!(layout.is_outside(mid_x + 1_000.0, mid_z));
        let lobby = layout.lobby();
        assert!(!layout.is_outside(lobby.x, lobby.z));
    }

    #[test]
    fn a_covered_bridge_with_a_pavilion_crosses_the_central_plaza() {
        let (map, layout) = layout();
        let inside = stadium(&map);
        let z = (inside.min_z + inside.max_z).div_euclid(2);
        let deck = 63 + imperial::BRIDGE_LIFT;
        let ridge = deck + 6 + imperial::BRIDGE_HALF + 1;
        for x in [inside.min_x, inside.max_x] {
            assert_eq!(layout.block_at((x, deck, z)), Some(imperial::DECK), "{x}");
            assert_eq!(layout.block_at((x, ridge, z)), Some(imperial::RIDGE), "{x}");
        }
        let middle = (inside.min_x + inside.max_x).div_euclid(2);
        let spire = (deck..deck + 40)
            .any(|y| y > ridge + 5 && layout.block_at((middle, y, z)) == Some(imperial::RIDGE));
        assert!(spire);
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
            assert!(
                lobby.z
                    < f64::from(
                        extent.min_z
                            - RIM_WIDTH
                            - WALKWAY_MARGIN
                            - imperial::TERRACE_ROWS
                            - imperial::PALACE_DEPTH
                    )
            );
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
    fn every_lane_is_built_even_without_a_player() {
        let (_, layout) = layout();
        for lane_id in LaneId::ALL {
            assert!(!layout.lane_blocks(lane_id).is_empty(), "{lane_id:?}");
        }
    }

    #[test]
    fn a_panel_floats_above_the_rim_of_every_lane() {
        let (map, layout) = layout();
        for lane_map in &map.lanes {
            let panel = layout.panel(lane(lane_map.lane_id)).expect("panel");
            let extent = Extent::of(lane_map);
            assert!(panel.y > 70.0);
            assert!(panel.z < f64::from(extent.min_z));
            assert!(panel.x > f64::from(extent.min_x) && panel.x < f64::from(extent.max_x));
        }
    }
}
