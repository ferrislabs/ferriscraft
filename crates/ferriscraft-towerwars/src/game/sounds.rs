use rspigot_plugin::Sound;

use crate::domain::{LaneId, PlayerId, TowerKind, Vec3};
use crate::sim::{Event, Match};

pub const MAX_SHOT_SOUNDS_PER_TICK: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hearing {
    At(Vec3),
    Lane(LaneId),
    Player(PlayerId),
    Everyone,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cue {
    pub hearing: Hearing,
    pub sound: Sound,
    pub volume: f32,
    pub pitch: f32,
}

impl Cue {
    const fn new(hearing: Hearing, sound: Sound, volume: f32, pitch: f32) -> Self {
        Self {
            hearing,
            sound,
            volume,
            pitch,
        }
    }
}

pub const fn shot_sound(kind: TowerKind) -> Option<(Sound, f32, f32)> {
    match kind {
        TowerKind::Archer => Some((Sound::EntityArrowShoot, 0.5, 1.2)),
        TowerKind::Ice => Some((Sound::EntityPlayerHurtFreeze, 0.4, 1.4)),
        TowerKind::Mage => Some((Sound::EntityBlazeShoot, 0.35, 1.5)),
        TowerKind::Cannon => Some((Sound::EntityGenericExplode, 0.35, 1.3)),
        TowerKind::Tesla => Some((Sound::BlockRespawnAnchorCharge, 0.4, 1.8)),
        TowerKind::Slime => Some((Sound::EntitySlimeSquish, 0.5, 0.9)),
        TowerKind::Beacon => None,
    }
}

pub fn cues(events: &[Event], game: &Match) -> Vec<Cue> {
    let mut out = Vec::new();
    let mut shots = 0;
    for event in events {
        match event {
            Event::TowerFired {
                lane, tower, from, ..
            } => {
                if shots >= MAX_SHOT_SOUNDS_PER_TICK {
                    continue;
                }
                let kind = game.lane(*lane).tower(*tower).map(|found| found.kind);
                if let Some((sound, volume, pitch)) = kind.and_then(shot_sound) {
                    out.push(Cue::new(Hearing::At(*from), sound, volume, pitch));
                    shots += 1;
                }
            }
            Event::TowerPlaced { lane, tower, .. } => {
                if let Some(found) = game.lane(*lane).tower(*tower) {
                    let at = Hearing::At(found.world_pos);
                    out.push(Cue::new(at, Sound::BlockStonePlace, 1.0, 0.9));
                }
            }
            Event::TowerUpgraded { lane, tower, .. } => {
                if let Some(found) = game.lane(*lane).tower(*tower) {
                    let at = Hearing::At(found.world_pos);
                    out.push(Cue::new(at, Sound::BlockAnvilUse, 0.6, 1.3));
                    out.push(Cue::new(at, Sound::EntityPlayerLevelup, 0.5, 1.6));
                }
            }
            Event::TowerSold { lane, .. } => {
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::EntityItemPickup,
                    0.8,
                    0.7,
                ));
            }
            Event::TowerStunned { lane, tower, .. } => {
                if let Some(found) = game.lane(*lane).tower(*tower) {
                    out.push(Cue::new(
                        Hearing::At(found.world_pos),
                        Sound::BlockFireExtinguish,
                        0.5,
                        1.2,
                    ));
                }
            }
            Event::MobSent { sender, .. } => {
                out.push(Cue::new(
                    Hearing::Player(*sender),
                    Sound::UiButtonClick,
                    0.6,
                    1.4,
                ));
            }
            Event::MobKilled { lane, .. } => {
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::EntityExperienceOrbPickup,
                    0.3,
                    1.2,
                ));
            }
            Event::MobLeaked { lane, .. } => {
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::EntityPlayerHurt,
                    1.0,
                    0.8,
                ));
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::BlockAnvilLand,
                    0.4,
                    0.6,
                ));
            }
            Event::MobSplit { lane, .. } => {
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::EntitySlimeSquish,
                    0.6,
                    0.7,
                ));
            }
            Event::MobBlinked { lane, .. } => {
                out.push(Cue::new(
                    Hearing::Lane(*lane),
                    Sound::EntityEndermanTeleport,
                    0.6,
                    1.0,
                ));
            }
            Event::Explosion { at, .. } => {
                out.push(Cue::new(
                    Hearing::At(*at),
                    Sound::EntityGenericExplode,
                    0.6,
                    1.0,
                ));
            }
            Event::Chain { points, .. } => {
                if let Some(first) = points.first() {
                    out.push(Cue::new(
                        Hearing::At(*first),
                        Sound::EntityLightningBoltImpact,
                        0.3,
                        1.8,
                    ));
                }
            }
            Event::IncomePaid { player, .. } => {
                out.push(Cue::new(
                    Hearing::Player(*player),
                    Sound::EntityItemPickup,
                    0.35,
                    1.6,
                ));
            }
            Event::TierUnlocked { .. } => {
                out.push(Cue::new(
                    Hearing::Everyone,
                    Sound::BlockNoteBlockPling,
                    1.0,
                    1.0,
                ));
                out.push(Cue::new(
                    Hearing::Everyone,
                    Sound::BlockBeaconActivate,
                    0.6,
                    1.4,
                ));
            }
            Event::SuddenDeath { .. } => {
                out.push(Cue::new(
                    Hearing::Everyone,
                    Sound::EntityWitherSpawn,
                    0.5,
                    1.0,
                ));
            }
            Event::LaneEliminated { .. } => {
                out.push(Cue::new(
                    Hearing::Everyone,
                    Sound::EntityLightningBoltThunder,
                    0.6,
                    1.0,
                ));
            }
            Event::Won { .. } => {
                out.push(Cue::new(
                    Hearing::Everyone,
                    Sound::UiToastChallengeComplete,
                    1.0,
                    1.0,
                ));
            }
            Event::MobSpawned { .. } | Event::LaneDamaged { .. } | Event::LaneHealed { .. } => {}
        }
    }
    out
}

pub const REFUSED: Sound = Sound::EntityVillagerNo;

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{GameMode, MobKind};
    use crate::map::Map;
    use crate::sim::Seat;

    fn pid(index: u8) -> PlayerId {
        PlayerId(Uuid::from_u128(u128::from(index) + 1))
    }

    fn game() -> Match {
        let seats: Vec<Seat> = (0..4u8)
            .map(|index| Seat {
                player: pid(index),
                lane: LaneId::try_from(index).expect("lane"),
            })
            .collect();
        Match::new(GameMode::Solo, 3, &Map::test_map(), &seats).expect("match")
    }

    #[test]
    fn every_attacking_tower_has_a_shot_sound() {
        for kind in TowerKind::ALL {
            assert_eq!(
                shot_sound(kind).is_none(),
                kind == TowerKind::Beacon,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn placing_a_tower_is_heard_where_it_stands() {
        let mut game = game();
        game.place(pid(0), TowerKind::Archer, (2, 4))
            .expect("place");
        let events = game.drain_events();
        let heard = cues(&events, &game);
        assert!(heard.iter().any(
            |cue| matches!(cue.hearing, Hearing::At(_)) && cue.sound == Sound::BlockStonePlace
        ));
    }

    #[test]
    fn sending_a_mob_clicks_for_the_sender_only() {
        let mut game = game();
        game.send(pid(1), MobKind::Silverfish, 1).expect("send");
        let events = game.drain_events();
        let heard = cues(&events, &game);
        assert!(heard.contains(&Cue::new(
            Hearing::Player(pid(1)),
            Sound::UiButtonClick,
            0.6,
            1.4
        )));
    }

    #[test]
    fn shot_sounds_are_capped_per_tick() {
        let lane = LaneId::try_from(0).expect("lane");
        let mut shooting = game();
        let tower = shooting
            .place(pid(0), TowerKind::Archer, (2, 4))
            .expect("place");
        let fired: Vec<Event> = (0..20)
            .map(|_| Event::TowerFired {
                lane,
                tower,
                from: Vec3::default(),
                to: Vec3::default(),
            })
            .collect();
        assert_eq!(cues(&fired, &shooting).len(), MAX_SHOT_SOUNDS_PER_TICK);
    }
}
