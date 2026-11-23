use serde::{Deserialize, Serialize};

use crate::domain::{Aabb, Vec3};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneMap {
    pub lane_id: u8,
    pub spawn_point: Vec3,
    pub waypoints: Vec<Vec3>,
    pub build_bounds: Vec<Aabb>,
    pub spectate_anchor: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Map {
    pub lanes: Vec<LaneMap>,
}
