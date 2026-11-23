use crate::domain::{Aabb, LANE_COUNT, Vec3};

use super::{LaneMap, Map, MapError, Path};

const MIN_LENGTH: f32 = 75.0;
const MAX_LENGTH: f32 = 95.0;
const MIN_TURNS: usize = 4;
const MAX_TURNS: usize = 6;
const CLEARANCE: f32 = 0.5;
const MAX_BOX_SPAN: f32 = 512.0;

pub fn validate(map: &Map) -> Result<(), MapError> {
    let mut seen = [false; LANE_COUNT];
    if map.lanes.len() != LANE_COUNT {
        return Err(MapError::LaneCount);
    }
    for lane in &map.lanes {
        match seen.get_mut(usize::from(lane.lane_id)) {
            Some(slot) if !*slot => *slot = true,
            _ => return Err(MapError::LaneCount),
        }
    }
    map.lanes.iter().try_for_each(validate_lane)
}

fn invalid(lane: &LaneMap, reason: impl Into<String>) -> MapError {
    MapError::InvalidLane {
        lane: lane.lane_id,
        reason: reason.into(),
    }
}

fn finite(point: Vec3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn validate_lane(lane: &LaneMap) -> Result<(), MapError> {
    let mut points = Vec::with_capacity(lane.waypoints.len() + 1);
    points.push(lane.spawn_point);
    points.extend_from_slice(&lane.waypoints);
    if !points.iter().copied().all(finite) || !finite(lane.spectate_anchor) {
        return Err(invalid(lane, "coordonnées non finies"));
    }
    let path =
        Path::new(points).map_err(|error| invalid(lane, format!("chemin invalide : {error}")))?;
    let length = path.length();
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        return Err(invalid(
            lane,
            format!("longueur du chemin {length:.1} hors de 75 à 95 blocs"),
        ));
    }
    let turns = path.turns();
    if !(MIN_TURNS..=MAX_TURNS).contains(&turns) {
        return Err(invalid(
            lane,
            format!("{turns} virages, il en faut de 4 à 6"),
        ));
    }
    if lane.build_bounds.is_empty() {
        return Err(invalid(lane, "build_bounds ne doit pas être vide"));
    }
    for bounds in &lane.build_bounds {
        validate_box(lane, bounds, path.points())?;
    }
    Ok(())
}

fn validate_box(lane: &LaneMap, bounds: &Aabb, path: &[Vec3]) -> Result<(), MapError> {
    if !finite(bounds.min) || !finite(bounds.max) {
        return Err(invalid(
            lane,
            "zone de construction aux coordonnées non finies",
        ));
    }
    if bounds.min.x > bounds.max.x || bounds.min.y > bounds.max.y || bounds.min.z > bounds.max.z {
        return Err(invalid(
            lane,
            "zone de construction avec min supérieur à max",
        ));
    }
    if bounds.max.x - bounds.min.x > MAX_BOX_SPAN || bounds.max.z - bounds.min.z > MAX_BOX_SPAN {
        return Err(invalid(lane, "zone de construction trop grande"));
    }
    let (x_from, x_to) = (bounds.min.x.floor() as i32, bounds.max.x.ceil() as i32);
    let (z_from, z_to) = (bounds.min.z.floor() as i32, bounds.max.z.ceil() as i32);
    for x in x_from..=x_to {
        for z in z_from..=z_to {
            if bounds.contains_cell(x, z) && on_path(path, x as f32 + 0.5, z as f32 + 0.5) {
                return Err(invalid(
                    lane,
                    format!("la case de construction ({x}, {z}) est sur le chemin"),
                ));
            }
        }
    }
    Ok(())
}

fn on_path(path: &[Vec3], x: f32, z: f32) -> bool {
    path.windows(2)
        .any(|pair| segment_distance_xz(pair[0], pair[1], x, z) < CLEARANCE)
}

fn segment_distance_xz(a: Vec3, b: Vec3, x: f32, z: f32) -> f32 {
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    let squared = dx * dx + dz * dz;
    let t = if squared <= f32::EPSILON {
        0.0
    } else {
        (((x - a.x) * dx + (z - a.z) * dz) / squared).clamp(0.0, 1.0)
    };
    let (px, pz) = (a.x + dx * t, a.z + dz * t);
    ((x - px) * (x - px) + (z - pz) * (z - pz)).sqrt()
}
