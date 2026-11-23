const EYE_HEIGHT: f64 = 1.62;
const MAX_DISTANCE: f64 = 120.0;

pub fn surface_cell(
    position: (f64, f64, f64),
    yaw: f32,
    pitch: f32,
    surface_y: f64,
) -> Option<(i32, i32)> {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let direction = (
        -yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    let eye_y = position.1 + EYE_HEIGHT;
    if direction.1 >= -f64::EPSILON || eye_y <= surface_y {
        return None;
    }
    let distance = (surface_y - eye_y) / direction.1;
    if distance > MAX_DISTANCE {
        return None;
    }
    let x = position.0 + direction.0 * distance;
    let z = position.2 + direction.2 * distance;
    Some((x.floor() as i32, z.floor() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_straight_down_hits_the_cell_below() {
        let cell = surface_cell((10.5, 80.0, 4.5), 0.0, 90.0, 64.0);
        assert_eq!(cell, Some((10, 4)));
    }

    #[test]
    fn a_forty_five_degree_pitch_lands_as_far_ahead_as_the_height() {
        let cell = surface_cell((0.5, 64.38, 0.5), 0.0, 45.0, 64.0);
        assert_eq!(cell, Some((0, 2)));
        let east = surface_cell((0.5, 64.38, 0.5), -90.0, 45.0, 64.0);
        assert_eq!(east, Some((2, 0)));
    }

    #[test]
    fn looking_up_or_level_hits_nothing() {
        assert_eq!(surface_cell((0.0, 80.0, 0.0), 0.0, 0.0, 64.0), None);
        assert_eq!(surface_cell((0.0, 80.0, 0.0), 0.0, -30.0, 64.0), None);
    }

    #[test]
    fn a_player_below_the_surface_hits_nothing() {
        assert_eq!(surface_cell((0.0, 60.0, 0.0), 0.0, 90.0, 64.0), None);
    }

    #[test]
    fn a_too_distant_surface_hits_nothing() {
        assert_eq!(surface_cell((0.0, 500.0, 0.0), 0.0, 89.0, 64.0), None);
    }
}
