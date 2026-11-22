use crate::domain::TeamId;
use crate::sheep::{Action, ParticleKind, Vec3};

pub const MAX_TRACKED_ARROWS: usize = 24;
pub const MAX_ARROW_AGE_TICKS: u32 = 20 * 10;
pub const MAX_BURSTS_PER_PLAN: usize = 3;
pub const MAX_TRAIL_ACTIONS_PER_TICK: usize = 48;
pub const CELEBRATION_TICKS: u32 = 20 * 8;
pub const CELEBRATION_PULSE_TICKS: u32 = 10;

const HEAD_CLEARANCE: f64 = 2.2;

fn particle(kind: ParticleKind, at: Vec3, count: i32) -> Action {
    Action::Particle { kind, at, count }
}

pub fn sheep_trail(team: TeamId, at: Vec3) -> Vec<Action> {
    vec![
        particle(ParticleKind::TeamDust(team), at, 4),
        particle(ParticleKind::EndRod, at, 1),
    ]
}

pub fn arrow_trail(team: TeamId, at: Vec3) -> Vec<Action> {
    vec![particle(ParticleKind::TeamDust(team), at, 2)]
}

pub fn explosion_burst(actions: &[Action], team: TeamId) -> Vec<Action> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Explode { at, .. } => Some(*at),
            _ => None,
        })
        .take(MAX_BURSTS_PER_PLAN)
        .flat_map(|at| {
            [
                particle(ParticleKind::TeamDust(team), at, 40),
                particle(ParticleKind::Firework, at, 12),
            ]
        })
        .collect()
}

pub fn victory_pulse(team: TeamId, head: Vec3) -> Vec<Action> {
    let above = Vec3 {
        x: head.x,
        y: head.y + HEAD_CLEARANCE,
        z: head.z,
    };
    vec![
        particle(ParticleKind::Firework, above, 18),
        particle(ParticleKind::Totem, above, 8),
        particle(ParticleKind::TeamDust(team), above, 24),
    ]
}

pub fn capped(mut actions: Vec<Action>, limit: usize) -> Vec<Action> {
    actions.truncate(limit);
    actions
}

pub const fn celebrating(age: u32) -> bool {
    age < CELEBRATION_TICKS
}

pub const fn celebration_pulses(age: u32) -> bool {
    age.is_multiple_of(CELEBRATION_PULSE_TICKS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Vec3 = Vec3 {
        x: 4.0,
        y: 70.0,
        z: -9.0,
    };

    fn only_visual(actions: &[Action]) -> bool {
        actions
            .iter()
            .all(|action| matches!(action, Action::Particle { .. } | Action::Sound { .. }))
    }

    #[test]
    fn every_cosmetic_is_purely_visual() {
        let plan = vec![Action::Explode {
            at: AT,
            power: 3.0,
            breaks_blocks: true,
        }];
        for actions in [
            sheep_trail(TeamId::Red, AT),
            arrow_trail(TeamId::Blue, AT),
            explosion_burst(&plan, TeamId::Red),
            victory_pulse(TeamId::Blue, AT),
        ] {
            assert!(!actions.is_empty());
            assert!(only_visual(&actions));
        }
    }

    #[test]
    fn trails_follow_the_projectile_in_the_color_of_its_team() {
        for team in TeamId::ALL {
            let sheep = sheep_trail(team, AT);
            assert!(
                sheep
                    .iter()
                    .all(|a| matches!(a, Action::Particle { at, .. } if *at == AT))
            );
            assert!(sheep.iter().any(|a| matches!(
                a,
                Action::Particle { kind: ParticleKind::TeamDust(t), .. } if *t == team
            )));
            let arrow = arrow_trail(team, AT);
            assert!(arrow.iter().any(|a| matches!(
                a,
                Action::Particle { kind: ParticleKind::TeamDust(t), .. } if *t == team
            )));
        }
    }

    #[test]
    fn an_explosion_burst_is_added_for_each_blast_up_to_the_cap() {
        let blast = |x: f64| Action::Explode {
            at: Vec3 { x, ..AT },
            power: 2.0,
            breaks_blocks: false,
        };
        assert!(explosion_burst(&[], TeamId::Red).is_empty());
        let none = vec![Action::Sound {
            kind: crate::sheep::SoundKind::Heal,
            at: AT,
        }];
        assert!(explosion_burst(&none, TeamId::Red).is_empty());
        assert_eq!(explosion_burst(&[blast(0.0)], TeamId::Red).len(), 2);
        let many: Vec<Action> = (0..10).map(|i| blast(f64::from(i))).collect();
        assert_eq!(
            explosion_burst(&many, TeamId::Red).len(),
            MAX_BURSTS_PER_PLAN * 2
        );
    }

    #[test]
    fn the_burst_is_centered_on_the_blast_and_uses_the_team_color() {
        let plan = vec![Action::Explode {
            at: AT,
            power: 2.0,
            breaks_blocks: false,
        }];
        let burst = explosion_burst(&plan, TeamId::Blue);
        assert!(
            burst
                .iter()
                .all(|a| matches!(a, Action::Particle { at, .. } if *at == AT))
        );
        assert!(burst.iter().any(|a| matches!(
            a,
            Action::Particle {
                kind: ParticleKind::TeamDust(TeamId::Blue),
                ..
            }
        )));
    }

    #[test]
    fn the_victory_pulse_bursts_above_the_winner() {
        let pulse = victory_pulse(TeamId::Red, AT);
        assert!(pulse.iter().all(|a| matches!(
            a,
            Action::Particle { at, .. } if at.y > AT.y + 2.0 && at.x == AT.x && at.z == AT.z
        )));
        assert!(pulse.iter().any(|a| matches!(
            a,
            Action::Particle {
                kind: ParticleKind::Firework,
                ..
            }
        )));
    }

    #[test]
    fn the_particle_budget_is_bounded() {
        let many: Vec<Action> = (0..200)
            .flat_map(|_| sheep_trail(TeamId::Red, AT))
            .collect();
        assert_eq!(
            capped(many, MAX_TRAIL_ACTIONS_PER_TICK).len(),
            MAX_TRAIL_ACTIONS_PER_TICK
        );
        let few = sheep_trail(TeamId::Red, AT);
        assert_eq!(capped(few.clone(), MAX_TRAIL_ACTIONS_PER_TICK), few);
        const { assert!(MAX_TRACKED_ARROWS <= 32) };
    }

    #[test]
    fn the_celebration_pulses_every_half_second_then_stops() {
        assert!(celebrating(0));
        assert!(celebrating(CELEBRATION_TICKS - 1));
        assert!(!celebrating(CELEBRATION_TICKS));
        assert!(celebration_pulses(0));
        assert!(!celebration_pulses(1));
        assert!(celebration_pulses(CELEBRATION_PULSE_TICKS));
        assert_eq!(CELEBRATION_PULSE_TICKS, 10);
    }
}
