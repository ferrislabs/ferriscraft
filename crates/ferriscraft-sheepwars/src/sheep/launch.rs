use super::impact::Vec3;

pub const DEFAULT_THROW_POWER: f64 = 2.2;
pub const LAUNCH_ARC_LIFT: f64 = 0.14;
pub const STEEP_DOWN_PITCH_DEG: f64 = 35.0;
pub const SPAWN_FORWARD_OFFSET: f64 = 0.9;
pub const PROJECTILE_DRAG: f64 = 0.99;
pub const PROJECTILE_GRAVITY: f64 = 0.045;

pub fn look_direction(yaw_degrees: f64, pitch_degrees: f64) -> Vec3 {
    let yaw = yaw_degrees.to_radians();
    let pitch = pitch_degrees.to_radians();
    Vec3 {
        x: -yaw.sin() * pitch.cos(),
        y: -pitch.sin(),
        z: yaw.cos() * pitch.cos(),
    }
}

pub fn spawn_point(eye: Vec3, yaw_degrees: f64, pitch_degrees: f64) -> Vec3 {
    let dir = look_direction(yaw_degrees, pitch_degrees);
    Vec3 {
        x: eye.x + dir.x * SPAWN_FORWARD_OFFSET,
        y: (eye.y + dir.y * SPAWN_FORWARD_OFFSET).max(eye.y - 0.3),
        z: eye.z + dir.z * SPAWN_FORWARD_OFFSET,
    }
}

pub fn launch_velocity(yaw_degrees: f64, pitch_degrees: f64, power: f64) -> Vec3 {
    let dir = look_direction(yaw_degrees, pitch_degrees);
    Vec3 {
        x: dir.x * power,
        y: dir.y * power,
        z: dir.z * power,
    }
}

pub fn throw_velocity(yaw_degrees: f64, pitch_degrees: f64, power: f64) -> Vec3 {
    let mut velocity = launch_velocity(yaw_degrees, pitch_degrees, power);
    if pitch_degrees <= STEEP_DOWN_PITCH_DEG {
        velocity.y += LAUNCH_ARC_LIFT;
    }
    velocity
}

pub fn step_ballistic(velocity: Vec3) -> Vec3 {
    Vec3 {
        x: velocity.x * PROJECTILE_DRAG,
        y: velocity.y * PROJECTILE_DRAG - PROJECTILE_GRAVITY,
        z: velocity.z * PROJECTILE_DRAG,
    }
}

pub fn clamp_speed(velocity: Vec3, max_speed: f64) -> Vec3 {
    let speed =
        (velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z).sqrt();
    if speed <= max_speed || speed <= f64::EPSILON {
        return velocity;
    }
    let scale = max_speed / speed;
    Vec3 {
        x: velocity.x * scale,
        y: velocity.y * scale,
        z: velocity.z * scale,
    }
}

pub fn steer(from: Vec3, target: Vec3, speed: f64) -> Vec3 {
    let (dx, dy, dz) = (target.x - from.x, target.y - from.y, target.z - from.z);
    let length = (dx * dx + dy * dy + dz * dz).sqrt();
    if length <= f64::EPSILON {
        return Vec3::ZERO;
    }
    Vec3 {
        x: dx / length * speed,
        y: dy / length * speed,
        z: dz / length * speed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn looking_south_level_throws_along_positive_z() {
        let v = launch_velocity(0.0, 0.0, 2.0);
        assert!(close(v.x, 0.0) && close(v.y, 0.0) && close(v.z, 2.0));
    }

    #[test]
    fn yaw_90_throws_along_negative_x() {
        let v = launch_velocity(90.0, 0.0, 1.0);
        assert!(close(v.x, -1.0) && close(v.z, 0.0));
    }

    #[test]
    fn looking_up_throws_upward() {
        let v = launch_velocity(0.0, -90.0, 1.0);
        assert!(close(v.y, 1.0));
    }

    #[test]
    fn power_scales_the_length() {
        let v = launch_velocity(37.0, -20.0, 1.5);
        let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        assert!(close(length, 1.5));
    }

    #[test]
    fn throw_velocity_adds_arc_unless_aimed_steeply_down() {
        let level = throw_velocity(0.0, 0.0, DEFAULT_THROW_POWER);
        assert!(close(level.y, LAUNCH_ARC_LIFT));
        let down = throw_velocity(0.0, 60.0, DEFAULT_THROW_POWER);
        assert!(down.y < -DEFAULT_THROW_POWER * 0.8);
    }

    #[test]
    fn spawn_point_never_clips_deep_into_floor() {
        let eye = Vec3 {
            x: 0.0,
            y: 65.62,
            z: 0.0,
        };
        let spawned = spawn_point(eye, 0.0, 89.0);
        assert!(spawned.y >= eye.y - 0.3 - 1e-9);
    }

    #[test]
    fn ballistic_trajectory_reaches_across_island_gap() {
        let mut pos = Vec3 {
            x: 0.0,
            y: 157.62,
            z: 0.0,
        };
        let mut vel = throw_velocity(0.0, -20.0, DEFAULT_THROW_POWER);
        for _ in 0..45 {
            pos.x += vel.x;
            pos.y += vel.y;
            pos.z += vel.z;
            vel = step_ballistic(vel);
            if pos.y <= 156.0 {
                break;
            }
        }
        assert!(pos.z > 60.0, "expected >=60 blocks range, got {}", pos.z);
    }

    #[test]
    fn steering_points_at_the_target_with_the_given_speed() {
        let v = steer(
            Vec3::ZERO,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
            0.8,
        );
        assert!(close(v.z, 0.8) && close(v.x, 0.0));
        assert_eq!(steer(Vec3::ZERO, Vec3::ZERO, 1.0), Vec3::ZERO);
    }
}
