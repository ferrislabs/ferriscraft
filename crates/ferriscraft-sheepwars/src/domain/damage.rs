pub const MAX_NON_LETHAL: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage(f32);

impl Damage {
    pub fn capped(points: f32) -> Self {
        Self(points.clamp(0.0, MAX_NON_LETHAL))
    }

    pub const fn lethal() -> Self {
        Self(f32::INFINITY)
    }

    pub const fn is_lethal(self) -> bool {
        self.0.is_infinite()
    }

    pub fn at_distance(self, distance: f64, radius: f64) -> f32 {
        if radius <= f64::EPSILON || distance >= radius {
            return 0.0;
        }
        if self.is_lethal() {
            return f32::INFINITY;
        }
        let share = (1.0 - distance.max(0.0) / radius) as f32;
        self.0 * share
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_never_reaches_a_full_health_bar() {
        assert_eq!(Damage::capped(99.0), Damage::capped(MAX_NON_LETHAL));
        assert!(Damage::capped(99.0).at_distance(0.0, 5.0) < 20.0);
        assert!(!Damage::capped(99.0).is_lethal());
    }

    #[test]
    fn negative_points_clamp_to_zero() {
        assert_eq!(Damage::capped(-4.0).at_distance(0.0, 5.0), 0.0);
    }

    #[test]
    fn lethal_is_the_only_unbounded_damage() {
        assert!(Damage::lethal().is_lethal());
        assert!(Damage::lethal().at_distance(4.9, 5.0).is_infinite());
    }

    #[test]
    fn damage_falls_off_linearly_and_stops_at_the_radius() {
        let damage = Damage::capped(10.0);
        assert_eq!(damage.at_distance(0.0, 5.0), 10.0);
        assert_eq!(damage.at_distance(2.5, 5.0), 5.0);
        assert_eq!(damage.at_distance(5.0, 5.0), 0.0);
        assert_eq!(damage.at_distance(9.0, 5.0), 0.0);
    }
}
