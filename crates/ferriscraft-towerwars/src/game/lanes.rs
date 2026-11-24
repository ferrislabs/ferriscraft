use crate::domain::{GameMode, LaneId, PlayerId};
use crate::sim::Seat;

pub fn assign_seats(mode: GameMode, players: &[PlayerId]) -> Vec<Seat> {
    let per_lane = mode.owners_per_lane();
    players
        .iter()
        .enumerate()
        .filter_map(|(index, player)| {
            let lane = u8::try_from(index / per_lane).ok()?;
            Some(Seat {
                player: *player,
                lane: LaneId::try_from(lane).ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn players(count: u128) -> Vec<PlayerId> {
        (0..count)
            .map(|i| PlayerId(Uuid::from_u128(i + 1)))
            .collect()
    }

    fn lanes(seats: &[Seat]) -> Vec<u8> {
        seats.iter().map(|seat| seat.lane.get()).collect()
    }

    #[test]
    fn solo_gives_each_player_the_next_lane_in_join_order() {
        let seats = assign_seats(GameMode::Solo, &players(5));
        assert_eq!(lanes(&seats), vec![0, 1, 2, 3, 4]);
        assert_eq!(seats[2].player, players(5)[2]);
    }

    #[test]
    fn teams_pair_players_on_a_lane() {
        let seats = assign_seats(GameMode::Teams, &players(5));
        assert_eq!(lanes(&seats), vec![0, 0, 1, 1, 2]);
    }

    #[test]
    fn players_beyond_the_six_lanes_are_left_out() {
        assert_eq!(assign_seats(GameMode::Solo, &players(8)).len(), 6);
        assert_eq!(assign_seats(GameMode::Teams, &players(14)).len(), 12);
    }

    #[test]
    fn every_assignment_for_a_legal_roster_builds_a_match() {
        use crate::map::Map;
        use crate::sim::Match;
        for count in 4..=6 {
            let seats = assign_seats(GameMode::Solo, &players(count));
            assert!(Match::new(GameMode::Solo, 1, &Map::test_map(), &seats).is_ok());
        }
        for count in 4..=12 {
            let seats = assign_seats(GameMode::Teams, &players(count));
            assert!(Match::new(GameMode::Teams, 1, &Map::test_map(), &seats).is_ok());
        }
    }
}
