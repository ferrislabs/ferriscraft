mod common;

use common::{buildable_cell, player, run, solo_match};
use ferriscraft_towerwars::domain::{Gold, LaneId, MobKind, TowerKind};
use ferriscraft_towerwars::sim::{Event, Match};

fn lane(index: u8) -> LaneId {
    LaneId::try_from(index).expect("lane")
}

fn run_until_mob(game: &mut Match, lane_index: u8) {
    for _ in 0..10 {
        game.tick();
        if !game.lane(lane(lane_index)).mobs.is_empty() {
            return;
        }
    }
    panic!("no mob appeared");
}

fn first_hp(game: &Match) -> f32 {
    game.lane(lane(0)).mobs[0].hp
}

#[test]
fn spec_tw_12_an_archer_kills_a_silverfish_in_three_shots_for_one_gold() {
    let mut game = solo_match(4);
    game.place(player(0), TowerKind::Archer, buildable_cell(0, 3))
        .expect("place");
    game.send(player(1), MobKind::Silverfish, 1).expect("send");
    run_until_mob(&mut game, 0);
    assert_eq!(first_hp(&game), 23.0);
    run(&mut game, 20);
    assert_eq!(first_hp(&game), 11.0);
    assert_eq!(game.gold(player(0)), Some(Gold::new(50)));
    run(&mut game, 20);
    assert!(game.lane(lane(0)).mobs.is_empty());
    assert_eq!(game.gold(player(0)), Some(Gold::new(51)));
    let kills = game
        .drain_events()
        .into_iter()
        .filter(
            |event| matches!(event, Event::MobKilled { bounty: 1, lane: l, .. } if *l == lane(0)),
        )
        .count();
    assert_eq!(kills, 1);
}

#[test]
fn spec_tw_13_armor_reduces_an_archer_shot_but_not_a_mage_shot() {
    let mut game = solo_match(4);
    game.place(player(0), TowerKind::Archer, buildable_cell(0, 3))
        .expect("place");
    game.send(player(1), MobKind::Zombie, 1).expect("send");
    run_until_mob(&mut game, 0);
    assert!((first_hp(&game) - 84.2).abs() < 1e-3);
    let mut game = solo_match(4);
    game.place(player(0), TowerKind::Mage, buildable_cell(0, 3))
        .expect("place");
    game.send(player(1), MobKind::Zombie, 1).expect("send");
    run_until_mob(&mut game, 0);
    assert!((first_hp(&game) - 80.0).abs() < 1e-3);
}

fn scripted() -> Vec<Event> {
    let mut game = solo_match(4);
    game.place(player(0), TowerKind::Archer, buildable_cell(0, 3))
        .expect("place");
    game.place(player(1), TowerKind::Mage, buildable_cell(1, 5))
        .expect("place");
    for step in 0..40u64 {
        run(&mut game, 15);
        let kind = [MobKind::Silverfish, MobKind::Skeleton, MobKind::Zombie][step as usize % 3];
        let sender = player(2 + (step % 2) as u128);
        let _ = game.send(sender, kind, 1);
    }
    game.drain_events()
}

#[test]
fn spec_tw_23_combat_is_deterministic_for_a_seed() {
    let first = scripted();
    assert!(
        first
            .iter()
            .any(|event| matches!(event, Event::TowerFired { .. }))
    );
    assert_eq!(first, scripted());
}
