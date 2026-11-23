use crate::domain::{LaneId, PlayerId};
use crate::hud::lane_names;
use crate::sim::{Event, Match};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    Everyone,
    Lane(LaneId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub audience: Audience,
    pub text: String,
}

pub fn victory_subtitle(
    game: &Match,
    winner: Option<LaneId>,
    name_of: &impl Fn(PlayerId) -> Option<String>,
) -> String {
    match winner {
        Some(lane) => format!("Voie de {}", lane_names(game.lane(lane), name_of)),
        None => "Personne ne gagne".to_owned(),
    }
}

pub fn notices(
    events: &[Event],
    game: &Match,
    name_of: &impl Fn(PlayerId) -> Option<String>,
) -> Vec<Notice> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TierUnlocked { tier } => Some(Notice {
                audience: Audience::Everyone,
                text: format!("§eLes mobs de tier §f{tier}§e sont débloqués !"),
            }),
            Event::SuddenDeath { level } => Some(Notice {
                audience: Audience::Everyone,
                text: format!(
                    "§4§lMort subite §c(niveau {level}) : les mobs sont plus rapides et plus gros !"
                ),
            }),
            Event::LaneEliminated { lane } => Some(Notice {
                audience: Audience::Everyone,
                text: format!(
                    "§cLa voie de §f{}§c est éliminée !",
                    lane_names(game.lane(*lane), name_of)
                ),
            }),
            Event::LaneDamaged { lane, hp } => Some(Notice {
                audience: Audience::Lane(*lane),
                text: format!("§cUn mob a atteint votre portail : §f{hp} ❤§c restants."),
            }),
            Event::LaneHealed { lane, hp } => Some(Notice {
                audience: Audience::Lane(*lane),
                text: format!("§aVotre voie récupère des PV : §f{hp} ❤§a."),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::GameMode;
    use crate::map::Map;
    use crate::sim::Seat;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn lane(index: u8) -> LaneId {
        LaneId::try_from(index).expect("lane")
    }

    fn game() -> Match {
        let seats: Vec<Seat> = (0..4u8)
            .map(|index| Seat {
                player: pid(u128::from(index)),
                lane: lane(index),
            })
            .collect();
        Match::new(GameMode::Solo, 3, &Map::test_map(), &seats).expect("match")
    }

    fn names(id: PlayerId) -> Option<String> {
        Some(format!("P{}", id.0.as_u128()))
    }

    #[test]
    fn tiers_and_sudden_death_are_announced_to_everyone() {
        let game = game();
        let out = notices(
            &[
                Event::TierUnlocked { tier: 2 },
                Event::SuddenDeath { level: 1 },
            ],
            &game,
            &names,
        );
        assert_eq!(out.len(), 2);
        assert!(
            out.iter()
                .all(|notice| notice.audience == Audience::Everyone)
        );
        assert!(out[0].text.contains('2'));
    }

    #[test]
    fn an_elimination_names_the_owner() {
        let game = game();
        let out = notices(&[Event::LaneEliminated { lane: lane(2) }], &game, &names);
        assert!(out[0].text.contains("P3"), "{}", out[0].text);
    }

    #[test]
    fn damage_and_healing_go_to_the_lane_only() {
        let game = game();
        let out = notices(
            &[
                Event::LaneDamaged {
                    lane: lane(1),
                    hp: 19,
                },
                Event::LaneHealed {
                    lane: lane(1),
                    hp: 20,
                },
            ],
            &game,
            &names,
        );
        assert!(
            out.iter()
                .all(|notice| notice.audience == Audience::Lane(lane(1)))
        );
        assert!(out[0].text.contains("19"));
    }

    #[test]
    fn noisy_events_produce_no_notice() {
        let game = game();
        let out = notices(
            &[Event::IncomePaid {
                player: pid(0),
                amount: 10,
            }],
            &game,
            &names,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn the_victory_subtitle_names_the_winner_or_the_draw() {
        let game = game();
        assert_eq!(victory_subtitle(&game, Some(lane(0)), &names), "Voie de P1");
        assert_eq!(victory_subtitle(&game, None, &names), "Personne ne gagne");
    }
}
