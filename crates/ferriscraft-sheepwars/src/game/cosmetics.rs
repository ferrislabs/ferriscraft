use std::collections::BTreeMap;

use ferriscraft_protocol::{CosmeticId, CosmeticSlot, GameKind, PlayerProfile};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheepTrail {
    Flame,
    Hearts,
    Notes,
    Rainbow,
}

impl SheepTrail {
    pub const fn from_id(id: CosmeticId) -> Option<Self> {
        match id {
            CosmeticId::SheepTrailFlame => Some(Self::Flame),
            CosmeticId::SheepTrailHearts => Some(Self::Hearts),
            CosmeticId::SheepTrailNotes => Some(Self::Notes),
            CosmeticId::SheepTrailRainbow => Some(Self::Rainbow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictoryEffect {
    Fireworks,
    Lightning,
}

impl VictoryEffect {
    pub const fn from_id(id: CosmeticId) -> Option<Self> {
        match id {
            CosmeticId::VictoryFireworks => Some(Self::Fireworks),
            CosmeticId::VictoryLightning => Some(Self::Lightning),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loadout {
    pub level: u32,
    pub sheep_trail: Option<SheepTrail>,
    pub victory_effect: Option<VictoryEffect>,
}

impl Default for Loadout {
    fn default() -> Self {
        Self {
            level: 1,
            sheep_trail: None,
            victory_effect: None,
        }
    }
}

impl Loadout {
    pub fn from_profile(profile: &PlayerProfile) -> Self {
        let level = profile
            .games
            .iter()
            .find(|stats| stats.game == GameKind::SheepWars)
            .map_or(1, |stats| stats.progress.level);
        Self {
            level,
            sheep_trail: equipped(&profile.equipped, CosmeticSlot::SheepTrail)
                .and_then(SheepTrail::from_id),
            victory_effect: equipped(&profile.equipped, CosmeticSlot::VictoryEffect)
                .and_then(VictoryEffect::from_id),
        }
    }
}

fn equipped(map: &BTreeMap<CosmeticSlot, CosmeticId>, slot: CosmeticSlot) -> Option<CosmeticId> {
    map.get(&slot).copied()
}

pub fn equipped_sheep_trail(trail: SheepTrail, at: Vec3, age: u32) -> Vec<Action> {
    match trail {
        SheepTrail::Flame => vec![particle(ParticleKind::Flame, at, 4)],
        SheepTrail::Hearts => vec![particle(ParticleKind::Heart, at, 2)],
        SheepTrail::Notes => vec![particle(ParticleKind::Note, at, 2)],
        SheepTrail::Rainbow => {
            let step = u8::try_from(age % 6).unwrap_or(0);
            vec![particle(ParticleKind::Rainbow(step), at, 6)]
        }
    }
}

pub fn equipped_victory_pulse(effect: VictoryEffect, head: Vec3) -> Vec<Action> {
    let above = |lift: f64| Vec3 {
        x: head.x,
        y: head.y + HEAD_CLEARANCE + lift,
        z: head.z,
    };
    match effect {
        VictoryEffect::Fireworks => vec![
            particle(ParticleKind::Firework, above(0.0), 40),
            particle(ParticleKind::EndRod, above(0.0), 12),
        ],
        VictoryEffect::Lightning => vec![
            particle(ParticleKind::ElectricSpark, above(0.0), 20),
            particle(ParticleKind::ElectricSpark, above(1.0), 20),
            particle(ParticleKind::ElectricSpark, above(2.0), 20),
        ],
    }
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

    fn profile(level: Option<u32>, equipped: &[(CosmeticSlot, CosmeticId)]) -> PlayerProfile {
        use ferriscraft_protocol::{GameStats, LevelProgress, PlayerRank};
        let progress = |level| LevelProgress {
            level,
            xp_into_level: 0,
            xp_for_next: 100,
            total_xp: 0,
        };
        PlayerProfile {
            uuid: uuid::Uuid::nil(),
            name: "Alex".into(),
            rank: PlayerRank::Member,
            coins: 0,
            network: progress(1),
            games: level
                .map(|level| GameStats {
                    game: GameKind::SheepWars,
                    progress: progress(level),
                    played: 0,
                    wins: 0,
                    losses: 0,
                    draws: 0,
                })
                .into_iter()
                .collect(),
            unlocked: Vec::new(),
            equipped: equipped.iter().copied().collect(),
        }
    }

    #[test]
    fn a_profile_gives_the_sheepwars_level_and_equipped_cosmetics() {
        let loadout = Loadout::from_profile(&profile(
            Some(12),
            &[
                (CosmeticSlot::SheepTrail, CosmeticId::SheepTrailHearts),
                (CosmeticSlot::VictoryEffect, CosmeticId::VictoryLightning),
                (CosmeticSlot::LobbyTrail, CosmeticId::TrailFlame),
            ],
        ));
        assert_eq!(loadout.level, 12);
        assert_eq!(loadout.sheep_trail, Some(SheepTrail::Hearts));
        assert_eq!(loadout.victory_effect, Some(VictoryEffect::Lightning));
    }

    #[test]
    fn an_empty_or_unknown_profile_keeps_todays_behaviour() {
        assert_eq!(
            Loadout::from_profile(&profile(None, &[])),
            Loadout::default()
        );
        let loadout = Loadout::from_profile(&profile(
            None,
            &[
                (CosmeticSlot::SheepTrail, CosmeticId::Unknown),
                (CosmeticSlot::VictoryEffect, CosmeticId::SheepTrailFlame),
            ],
        ));
        assert_eq!(loadout, Loadout::default());
        assert_eq!(loadout.level, 1);
    }

    #[test]
    fn equipped_cosmetics_are_purely_visual_and_bounded() {
        for trail in [
            SheepTrail::Flame,
            SheepTrail::Hearts,
            SheepTrail::Notes,
            SheepTrail::Rainbow,
        ] {
            let actions = equipped_sheep_trail(trail, AT, 7);
            assert!(!actions.is_empty() && only_visual(&actions));
            assert!(
                actions
                    .iter()
                    .all(|a| matches!(a, Action::Particle { at, .. } if *at == AT))
            );
        }
        for effect in [VictoryEffect::Fireworks, VictoryEffect::Lightning] {
            let actions = equipped_victory_pulse(effect, AT);
            assert!(!actions.is_empty() && only_visual(&actions));
            assert!(actions.len() <= MAX_TRAIL_ACTIONS_PER_TICK);
            assert!(actions.iter().all(|a| matches!(
                a,
                Action::Particle { at, .. } if at.y > AT.y + 2.0
            )));
        }
    }

    #[test]
    fn the_rainbow_trail_cycles_through_colours() {
        let steps: Vec<ParticleKind> = (0..6)
            .flat_map(|age| equipped_sheep_trail(SheepTrail::Rainbow, AT, age))
            .filter_map(|a| match a {
                Action::Particle { kind, .. } => Some(kind),
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 6);
        assert!(steps.windows(2).all(|pair| pair[0] != pair[1]));
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
