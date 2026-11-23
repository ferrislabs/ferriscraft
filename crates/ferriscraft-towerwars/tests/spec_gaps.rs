mod common;

use common::{buildable_cell, player, run, solo_match, solo_seats, straight_map};
use ferriscraft_towerwars::domain::{
    GameMode, Gold, LaneId, MobKind, Priority, SimError, TowerKind,
};
use ferriscraft_towerwars::map::Map;
use ferriscraft_towerwars::sim::{Event, Match, Phase, Seat};

const MATCH_TICKS: u64 = 24_000;
const SPAM_CAP: u64 = 6_000;

fn lane(index: u8) -> LaneId {
    LaneId::try_from(index).expect("lane")
}

fn teams_match() -> Match {
    let seats: Vec<Seat> = [(0u128, 0u8), (1, 0), (2, 1), (3, 2), (4, 3)]
        .iter()
        .map(|(index, lane_index)| Seat {
            player: player(*index),
            lane: lane(*lane_index),
        })
        .collect();
    Match::new(GameMode::Teams, 7, &straight_map(), &seats).expect("match")
}

fn count(events: &[Event], wanted: impl Fn(&Event) -> bool) -> usize {
    events.iter().filter(|event| wanted(event)).count()
}

fn spam_until_ended(game: &mut Match, senders: &[u128]) {
    for _ in 0..SPAM_CAP {
        for sender in senders {
            let _ = game.send(player(*sender), MobKind::Silverfish, 1);
        }
        game.tick();
        if game.is_ended() {
            return;
        }
    }
}

#[test]
fn a_player_who_left_cannot_act_on_the_lane_of_a_teammate() {
    let mut game = teams_match();
    let tower = game
        .place(player(0), TowerKind::Archer, buildable_cell(0, 3))
        .expect("place");
    game.forfeit(player(1));
    assert_eq!(game.sell(player(1), tower), Err(SimError::PlayerLeft));
    assert_eq!(
        game.place(player(1), TowerKind::Archer, buildable_cell(0, 4)),
        Err(SimError::PlayerLeft)
    );
    assert_eq!(
        game.upgrade(player(1), tower, None),
        Err(SimError::PlayerLeft)
    );
    assert_eq!(
        game.set_priority(player(1), tower, Priority::Last),
        Err(SimError::PlayerLeft)
    );
    assert_eq!(
        game.send(player(1), MobKind::Silverfish, 1),
        Err(SimError::PlayerLeft)
    );
    assert_eq!(game.lane(lane(0)).towers().len(), 1);
    assert_eq!(game.gold(player(1)), Some(Gold::new(100)));
    assert_eq!(game.income(player(1)), Some(10));
    assert_eq!(game.sell(player(0), tower), Ok(40));
}

#[test]
fn simultaneous_final_eliminations_by_forfeit_are_a_draw() {
    let mut game = solo_match(4);
    for index in 0..4 {
        game.forfeit(player(index));
    }
    run(&mut game, 1);
    assert_eq!(game.phase(), Phase::Ended { winner: None });
    let events = game.drain_events();
    assert!(events.contains(&Event::Won { lane: None }));
    assert_eq!(
        count(&events, |e| matches!(e, Event::LaneEliminated { .. })),
        4
    );
}

#[test]
fn a_single_survivor_wins_even_when_others_fall_on_the_same_tick() {
    let mut game = solo_match(4);
    for index in 1..4 {
        game.forfeit(player(index));
    }
    run(&mut game, 1);
    assert_eq!(
        game.phase(),
        Phase::Ended {
            winner: Some(lane(0))
        }
    );
    assert!(game.drain_events().contains(&Event::Won {
        lane: Some(lane(0))
    }));
}

#[test]
fn spec_tw_20_a_lane_worn_down_by_leaks_is_eliminated() {
    let mut game = solo_match(4);
    for _ in 0..SPAM_CAP {
        for sender in [0u128, 2, 3] {
            let _ = game.send(player(sender), MobKind::Silverfish, 1);
        }
        game.tick();
        if game.lane(lane(1)).eliminated {
            break;
        }
    }
    assert!(game.lane(lane(1)).eliminated);
    assert_eq!(game.lane(lane(1)).hp.current(), 0);
    assert!(!game.lane(lane(0)).eliminated);
    assert!(!game.is_ended());
    let events = game.drain_events();
    assert!(events.contains(&Event::LaneEliminated { lane: lane(1) }));
    let leaks = events
        .iter()
        .filter_map(|event| match event {
            Event::MobLeaked {
                lane: l, damage, ..
            } if *l == lane(1) => Some(u32::from(*damage)),
            _ => None,
        })
        .sum::<u32>();
    assert!(leaks >= 20, "leaks on lane 1: {leaks}");
}

#[test]
fn spec_tw_20_lanes_worn_down_on_the_same_tick_draw() {
    let mut game = solo_match(4);
    spam_until_ended(&mut game, &[0, 1, 2, 3]);
    assert_eq!(game.phase(), Phase::Ended { winner: None });
    let events = game.drain_events();
    assert_eq!(
        count(&events, |e| matches!(e, Event::LaneEliminated { .. })),
        4
    );
    assert_eq!(events.last(), Some(&Event::Won { lane: None }));
}

#[test]
fn spec_tw_22_teams_share_the_lane_hp_but_not_income() {
    let mut game = teams_match();
    for index in 0..4 {
        assert_eq!(game.lane(lane(index)).hp.current(), 25);
    }
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    assert_eq!(game.gold(player(0)), Some(Gold::new(85)));
    assert_eq!(game.income(player(0)), Some(12));
    assert_eq!(game.gold(player(1)), Some(Gold::new(100)));
    assert_eq!(game.income(player(1)), Some(10));
    run(&mut game, 5);
    assert!(game.lane(lane(0)).mobs.is_empty());
    for index in 1..4 {
        assert_eq!(game.lane(lane(index)).mobs.len(), 1, "lane {index}");
    }
    run(&mut game, 115);
    assert_eq!(game.gold(player(0)), Some(Gold::new(97)));
    assert_eq!(game.gold(player(1)), Some(Gold::new(110)));
    let events = game.drain_events();
    for (index, amount) in [(0u128, 12u32), (1, 10)] {
        let paid = Event::IncomePaid {
            player: player(index),
            amount,
        };
        assert_eq!(count(&events, |e| *e == paid), 1);
    }
}

#[test]
fn spec_tw_5_tiers_three_and_four_unlock_on_schedule_and_are_announced_once() {
    let mut game = solo_match(4);
    run(&mut game, 6_599);
    assert_eq!(
        game.send(player(0), MobKind::GiantSlime, 1),
        Err(SimError::LockedTier)
    );
    run(&mut game, 1);
    assert_eq!(
        game.send(player(0), MobKind::GiantSlime, 1),
        Err(SimError::NotEnoughGold)
    );
    run(&mut game, 4_199);
    assert_eq!(
        game.send(player(0), MobKind::Warden, 1),
        Err(SimError::LockedTier)
    );
    run(&mut game, 1);
    assert_eq!(
        game.send(player(0), MobKind::Warden, 1),
        Err(SimError::NotEnoughGold)
    );
    run(&mut game, 500);
    let tiers: Vec<u8> = game
        .drain_events()
        .iter()
        .filter_map(|event| match event {
            Event::TierUnlocked { tier } => Some(*tier),
            _ => None,
        })
        .collect();
    assert_eq!(tiers, vec![2, 3, 4]);
}

#[test]
fn invariant_6_progress_never_decreases_and_steps_by_speed_over_twenty() {
    let mut game = solo_match(4);
    game.send(player(0), MobKind::Silverfish, 1).expect("send");
    run(&mut game, 1);
    let mut previous = game.lane(lane(1)).mobs[0].progress;
    for _ in 0..200 {
        game.tick();
        let current = game.lane(lane(1)).mobs[0].progress;
        assert!(current >= previous, "progress went back");
        assert!(
            (current - previous - 3.2 / 20.0).abs() < 1e-4,
            "step {}",
            current - previous
        );
        previous = current;
    }
}

fn scripted_match(seed: u64) -> (Match, Vec<Event>) {
    let seats = solo_seats(6);
    let mut game = Match::new(GameMode::Solo, seed, &Map::test_map(), &seats).expect("match");
    let mut events = Vec::new();
    for tick in 0..MATCH_TICKS {
        script_step(&mut game, tick);
        game.tick();
        events.extend(game.drain_events());
    }
    (game, events)
}

fn script_step(game: &mut Match, tick: u64) {
    if tick % 120 == 60 {
        for index in 0..6u128 {
            build(game, index);
        }
    }
    if tick.is_multiple_of(600) && tick < 17_000 {
        let slot = (tick / 600) as usize;
        let sender = player((slot % 6) as u128);
        let kind = [MobKind::Silverfish, MobKind::Zombie, MobKind::Skeleton][slot % 3];
        let _ = game.send(sender, kind, 1);
    }
}

fn build(game: &mut Match, index: u128) {
    let who = player(index);
    let lane_id = lane(index as u8);
    let origin_x = (index % 3) as i32 * 40;
    let origin_z = (index / 3) as i32 * 30;
    for z in 1..8 {
        for x in 0..25 {
            let cell = (origin_x + x, origin_z + z);
            if game.lane(lane_id).tower_at(cell).is_some() {
                continue;
            }
            let kind = if x % 2 == 0 {
                TowerKind::Archer
            } else {
                TowerKind::Cannon
            };
            if game.place(who, kind, cell).is_ok() {
                return;
            }
        }
    }
}

#[test]
fn spec_tw_23_a_full_length_match_reaches_sudden_death_and_is_reproducible() {
    let (first_game, first) = scripted_match(11);
    assert_eq!(first_game.elapsed(), MATCH_TICKS);
    assert!(!first_game.is_ended(), "ended: {:?}", first_game.phase());
    let (second_game, second) = scripted_match(11);
    assert_eq!(second_game.elapsed(), MATCH_TICKS);
    assert_eq!(first, second);
    assert!(first.iter().any(|e| matches!(e, Event::SuddenDeath { .. })));
    assert!(first.contains(&Event::TierUnlocked { tier: 4 }));
    assert!(first.iter().any(|e| matches!(e, Event::MobKilled { .. })));
    let (_, other) = scripted_match(12);
    assert_ne!(first, other);
}
