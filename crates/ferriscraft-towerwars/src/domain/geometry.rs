use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn distance(self, other: Self) -> f32 {
        let (dx, dy, dz) = (self.x - other.x, self.y - other.y, self.z - other.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    pub fn distance_xz(self, other: Self) -> f32 {
        let (dx, dz) = (self.x - other.x, self.z - other.z);
        (dx * dx + dz * dz).sqrt()
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
            z: self.z + (other.z - self.z) * t,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn contains_cell(&self, x: i32, z: i32) -> bool {
        let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
        cx >= self.min.x && cx <= self.max.x && cz >= self.min.z && cz <= self.max.z
    }
}
