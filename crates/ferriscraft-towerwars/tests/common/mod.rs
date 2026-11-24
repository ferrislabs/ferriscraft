#![allow(dead_code)]

use ferriscraft_towerwars::domain::{Aabb, GameMode, LaneId, PlayerId, Vec3};
use ferriscraft_towerwars::map::{LaneMap, Map};
use ferriscraft_towerwars::sim::{Match, Seat};
use uuid::Uuid;

pub const LANE_SPACING: f32 = 20.0;

pub fn straight_map() -> Map {
    let lanes = (0..6u8)
        .map(|index| {
            let z = f32::from(index) * LANE_SPACING;
            LaneMap {
                lane_id: index,
                spawn_point: Vec3::new(0.0, 64.0, z),
                waypoints: vec![Vec3::new(40.0, 64.0, z), Vec3::new(40.0, 64.0, z + 10.0)],
                build_bounds: vec![Aabb {
                    min: Vec3::new(0.0, 64.0, z + 2.0),
                    max: Vec3::new(39.0, 64.0, z + 6.0),
                }],
                spectate_anchor: Vec3::new(20.0, 70.0, z + 5.0),
            }
        })
        .collect();
    Map { lanes }
}

pub fn player(index: u128) -> PlayerId {
    PlayerId(Uuid::from_u128(index + 1))
}

pub fn solo_seats(count: usize) -> Vec<Seat> {
    (0..count)
        .map(|index| Seat {
            player: player(index as u128),
            lane: LaneId::try_from(index as u8).expect("lane"),
        })
        .collect()
}

pub fn solo_match(count: usize) -> Match {
    Match::new(GameMode::Solo, 7, &straight_map(), &solo_seats(count)).expect("match")
}

pub fn run(game: &mut Match, ticks: u64) {
    for _ in 0..ticks {
        game.tick();
    }
}

pub fn buildable_cell(lane: u8, index: i32) -> (i32, i32) {
    let z = (f32::from(lane) * LANE_SPACING) as i32 + 2;
    (index % 39, z + index / 39)
}
