mod generate;
mod layout;
mod path;
mod validate;

use thiserror::Error;

pub use layout::{LaneMap, Map};
pub use path::{Path, PathPoint};
pub use validate::validate;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MapError {
    #[error("le chemin doit avoir au moins deux points")]
    PathTooShort,
    #[error("deux points consécutifs du chemin sont confondus")]
    DuplicateWaypoint,
    #[error("map illisible : {0}")]
    Parse(String),
    #[error("la map doit définir 6 voies numérotées de 0 à 5")]
    LaneCount,
    #[error("voie {lane} : {reason}")]
    InvalidLane { lane: u8, reason: String },
}

impl Map {
    pub fn from_json(raw: &str) -> Result<Self, MapError> {
        let map: Self =
            serde_json::from_str(raw).map_err(|error| MapError::Parse(error.to_string()))?;
        validate::validate(&map)?;
        Ok(map)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn test_map() -> Self {
        generate::test_map()
    }

    pub fn lane(&self, id: crate::domain::LaneId) -> Option<&LaneMap> {
        self.lanes.iter().find(|lane| lane.lane_id == id.get())
    }

    pub fn path_of(&self, id: crate::domain::LaneId) -> Result<Path, MapError> {
        let lane = self.lane(id).ok_or(MapError::LaneCount)?;
        let mut points = Vec::with_capacity(lane.waypoints.len() + 1);
        points.push(lane.spawn_point);
        points.extend_from_slice(&lane.waypoints);
        Path::new(points)
    }
}
