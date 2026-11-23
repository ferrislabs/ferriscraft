use crate::domain::{Aabb, LANE_COUNT, Vec3};

use super::{LaneMap, Map};

const GROUND: f32 = 64.0;
const ROW_LENGTH: f32 = 25.0;
const ROW_GAP: f32 = 8.0;
const LANE_STRIDE_X: f32 = 40.0;
const LANE_STRIDE_Z: f32 = 30.0;
const LANES_PER_ROW: usize = 3;

pub fn test_map() -> Map {
    Map {
        lanes: (0..LANE_COUNT).map(lane).collect(),
    }
}

fn lane(index: usize) -> LaneMap {
    let origin_x = (index % LANES_PER_ROW) as f32 * LANE_STRIDE_X;
    let origin_z = (index / LANES_PER_ROW) as f32 * LANE_STRIDE_Z;
    let at = |x: f32, z: f32| Vec3::new(origin_x + x + 0.5, GROUND, origin_z + z + 0.5);
    let strip = |x_min: f32, x_max: f32, z_min: f32, z_max: f32| Aabb {
        min: Vec3::new(origin_x + x_min, GROUND, origin_z + z_min),
        max: Vec3::new(origin_x + x_max, GROUND + 1.0, origin_z + z_max),
    };
    LaneMap {
        lane_id: index as u8,
        spawn_point: at(0.0, 0.0),
        waypoints: vec![
            at(ROW_LENGTH, 0.0),
            at(ROW_LENGTH, ROW_GAP),
            at(0.0, ROW_GAP),
            at(0.0, ROW_GAP * 2.0),
            at(ROW_LENGTH, ROW_GAP * 2.0),
        ],
        build_bounds: vec![
            strip(0.0, ROW_LENGTH, 1.0, ROW_GAP),
            strip(1.0, ROW_LENGTH + 1.0, ROW_GAP + 1.0, ROW_GAP * 2.0),
        ],
        spectate_anchor: Vec3::new(
            origin_x + ROW_LENGTH / 2.0 + 0.5,
            GROUND + 16.0,
            origin_z + ROW_GAP + 0.5,
        ),
    }
}
