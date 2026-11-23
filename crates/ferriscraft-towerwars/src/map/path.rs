use crate::domain::Vec3;

use super::MapError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathPoint {
    pub position: Vec3,
    pub yaw: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    points: Vec<Vec3>,
    cumulative: Vec<f32>,
}

impl Path {
    pub fn new(points: Vec<Vec3>) -> Result<Self, MapError> {
        if points.len() < 2 {
            return Err(MapError::PathTooShort);
        }
        let mut cumulative = Vec::with_capacity(points.len());
        let mut total = 0.0;
        cumulative.push(total);
        for pair in points.windows(2) {
            let step = pair[0].distance(pair[1]);
            if step <= f32::EPSILON {
                return Err(MapError::DuplicateWaypoint);
            }
            total += step;
            cumulative.push(total);
        }
        Ok(Self { points, cumulative })
    }

    pub fn length(&self) -> f32 {
        self.cumulative.last().copied().unwrap_or(0.0)
    }

    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    pub fn turns(&self) -> usize {
        self.points
            .windows(3)
            .filter(|triple| {
                let (a, b, c) = (triple[0], triple[1], triple[2]);
                let cross = (b.x - a.x) * (c.z - b.z) - (b.z - a.z) * (c.x - b.x);
                cross.abs() > 1e-3
            })
            .count()
    }

    pub fn sample(&self, progress: f32) -> PathPoint {
        let clamped = progress.clamp(0.0, self.length());
        let segment = self
            .cumulative
            .partition_point(|reached| *reached <= clamped)
            .clamp(1, self.points.len() - 1);
        let (from, to) = (self.points[segment - 1], self.points[segment]);
        let span = self.cumulative[segment] - self.cumulative[segment - 1];
        let t = (clamped - self.cumulative[segment - 1]) / span;
        let (dx, dz) = (to.x - from.x, to.z - from.z);
        PathPoint {
            position: from.lerp(to, t),
            yaw: (-dx).atan2(dz).to_degrees(),
        }
    }
}
