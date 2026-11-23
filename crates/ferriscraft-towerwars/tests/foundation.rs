mod common;

use common::{player, run, solo_match, solo_seats, straight_map};
use ferriscraft_towerwars::domain::{GameMode, Gold, LaneId, SimError};
use ferriscraft_towerwars::sim::{Match, MatchError};

#[test]
fn a_new_match_starts_every_player_with_the_starting_resources() {
    let game = solo_match(6);
    for index in 0..6 {
        let id = player(index);
        assert_eq!(game.gold(id), Some(Gold::new(100)));
        assert_eq!(game.income(id), Some(10));
    }
    for lane in LaneId::ALL {
        assert_eq!(game.lane(lane).hp.current(), 20);
    }
}

#[test]
fn solo_needs_at_least_four_players() {
    let result = Match::new(GameMode::Solo, 1, &straight_map(), &solo_seats(3));
    assert!(matches!(
        result,
        Err(MatchError::Sim(SimError::NotEnoughPlayers))
    ));
}

#[test]
fn lanes_without_an_owner_start_eliminated() {
    let game = solo_match(4);
    assert!(!game.lane(LaneId::try_from(0).unwrap()).eliminated);
    assert!(game.lane(LaneId::try_from(5).unwrap()).eliminated);
}

#[test]
fn ticking_advances_the_clock() {
    let mut game = solo_match(6);
    run(&mut game, 5);
    assert_eq!(game.elapsed(), 5);
}

#[test]
fn lane_ids_above_five_are_rejected() {
    assert_eq!(LaneId::try_from(6), Err(SimError::InvalidLane(6)));
}
