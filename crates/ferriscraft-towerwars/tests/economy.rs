mod common;

use common::{LANE_SPACING, player, run, solo_match};
use ferriscraft_towerwars::domain::{Gold, LaneId, MobKind, SimError};
use ferriscraft_towerwars::sim::{Event, Match, Phase};

fn lane(index: u8) -> LaneId {
    LaneId::try_from(index).expect("lane")
}

fn charges(game: &Match, index: u128, kind: MobKind) -> u8 {
    let slot = kind.stock_index().expect("sendable");
    game.world()
        .player(player(index))
        .expect("player")
        .stock
        .charges[slot]
}

#[test]
fn spec_tw_1_every_player_starts_with_full_stock() {
    let game = solo_match(6);
    for index in 0..6 {
        assert_eq!(game.gold(player(index)), Some(Gold::new(100)));
        assert_eq!(game.income(player(index)), Some(10));
        for kind in MobKind::SENDABLE {
            assert_eq!(charges(&game, index, kind), 10);
        }
    }
    assert!(
        LaneId::ALL
            .iter()
            .all(|id| game.lane(*id).hp.current() == 20)
    );
}

#[test]
fn spec_tw_2_income_is_paid_every_120_ticks() {
    let mut game = solo_match(6);
    run(&mut game, 119);
    assert_eq!(game.gold(player(0)), Some(Gold::new(100)));
    run(&mut game, 1);
    for index in 0..6 {
        assert_eq!(game.gold(player(index)), Some(Gold::new(110)));
    }
    let paid = game
        .drain_events()
        .iter()
        .filter(|event| matches!(event, Event::IncomePaid { amount: 10, .. }))
        .count();
    assert_eq!(paid, 6);
}

#[test]
fn spec_tw_3_sending_charges_gold_and_targets_enemy_lanes() {
    let mut game = solo_match(6);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    assert_eq!(game.gold(player(0)), Some(Gold::new(85)));
    assert_eq!(game.income(player(0)), Some(12));
    run(&mut game, 3);
    assert!(game.lane(lane(0)).mobs.is_empty());
    for index in 1..6 {
        let mobs = &game.lane(lane(index)).mobs;
        assert_eq!(mobs.len(), 1);
        assert_eq!(mobs[0].kind, MobKind::Silverfish);
        assert_eq!(mobs[0].hp, 35.0);
    }
}

#[test]
fn spec_tw_3_mobs_are_staggered_and_skip_empty_lanes() {
    let mut game = solo_match(4);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 3), Ok(()));
    run(&mut game, 1);
    assert_eq!(game.lane(lane(1)).mobs.len(), 1);
    run(&mut game, 2);
    assert_eq!(game.lane(lane(1)).mobs.len(), 2);
    run(&mut game, 2);
    assert_eq!(game.lane(lane(1)).mobs.len(), 3);
    assert!(game.lane(lane(5)).mobs.is_empty());
}

#[test]
fn spec_tw_4_stock_is_limited_and_recharges() {
    let mut game = solo_match(6);
    assert_eq!(
        game.send(player(0), MobKind::Silverfish, 11),
        Err(SimError::NoCharge)
    );
    assert_eq!(game.send(player(0), MobKind::Silverfish, 6), Ok(()));
    assert_eq!(charges(&game, 0, MobKind::Silverfish), 4);
    run(&mut game, 29);
    assert_eq!(charges(&game, 0, MobKind::Silverfish), 4);
    run(&mut game, 1);
    assert_eq!(charges(&game, 0, MobKind::Silverfish), 5);
    assert_eq!(charges(&game, 0, MobKind::Zombie), 10);
}

#[test]
fn spec_tw_4_send_errors_follow_the_documented_order() {
    let mut game = solo_match(4);
    assert_eq!(
        game.send(player(9), MobKind::Silverfish, 1),
        Err(SimError::UnknownPlayer)
    );
    assert_eq!(
        game.send(player(0), MobKind::MediumSlime, 1),
        Err(SimError::NotSendable)
    );
    assert_eq!(
        game.send(player(0), MobKind::Wither, 1),
        Err(SimError::LockedTier)
    );
    assert_eq!(
        game.send(player(0), MobKind::Skeleton, 2),
        Err(SimError::NotEnoughGold)
    );
    assert_eq!(game.send(player(0), MobKind::Skeleton, 0), Ok(()));
    assert_eq!(game.gold(player(0)), Some(Gold::new(100)));
}

#[test]
fn spec_tw_5_tiers_unlock_on_schedule() {
    let mut game = solo_match(6);
    run(&mut game, 2_999);
    assert_eq!(
        game.send(player(0), MobKind::Spider, 1),
        Err(SimError::LockedTier)
    );
    run(&mut game, 1);
    assert_eq!(game.send(player(0), MobKind::Spider, 1), Ok(()));
    let unlocks: Vec<Event> = game
        .drain_events()
        .into_iter()
        .filter(|event| matches!(event, Event::TierUnlocked { .. }))
        .collect();
    assert_eq!(unlocks, vec![Event::TierUnlocked { tier: 2 }]);
}

#[test]
fn spec_tw_6_mobs_move_along_the_path() {
    let mut game = solo_match(4);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    run(&mut game, 20);
    let mob = &game.lane(lane(1)).mobs[0];
    assert!((mob.progress - 3.2).abs() < 1e-3);
    run(&mut game, 270);
    let lane_one = game.lane(lane(1));
    let mob = &lane_one.mobs[0];
    assert!(mob.progress > 40.0);
    let position = lane_one.mob_position(mob);
    assert!(position.z > LANE_SPACING + 0.5);
}

#[test]
fn spec_tw_7_a_mob_reaching_the_base_costs_one_hp_and_no_gold() {
    let mut game = solo_match(4);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    run(&mut game, 330);
    for index in 1..4 {
        assert_eq!(game.lane(lane(index)).hp.current(), 19);
        assert!(game.lane(lane(index)).mobs.is_empty());
        assert_eq!(game.gold(player(u128::from(index))), Some(Gold::new(120)));
    }
    assert_eq!(game.lane(lane(0)).hp.current(), 20);
}

#[test]
fn spec_tw_19_sudden_death_strengthens_new_mobs() {
    let mut game = solo_match(4);
    run(&mut game, 18_000);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    run(&mut game, 1);
    let mob = &game.lane(lane(1)).mobs[0];
    assert_eq!(mob.max_hp, 52.5);
    assert!((mob.progress - 0.16 * 1.25).abs() < 1e-5);
    run(&mut game, 1_200);
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    run(&mut game, 1);
    let newest = game.lane(lane(1)).mobs.last().expect("mob");
    assert_eq!(newest.max_hp, 70.0);
    assert!((newest.progress - 0.16 * 1.5).abs() < 1e-5);
    let levels: Vec<u32> = game
        .drain_events()
        .iter()
        .filter_map(|event| match event {
            Event::SuddenDeath { level } => Some(*level),
            _ => None,
        })
        .collect();
    assert_eq!(levels, vec![1, 2]);
}

#[test]
fn spec_tw_20_an_eliminated_lane_receives_nothing_and_the_last_one_wins() {
    let mut game = solo_match(4);
    game.forfeit(player(1));
    run(&mut game, 1);
    assert!(game.lane(lane(1)).eliminated);
    assert_eq!(
        game.send(player(1), MobKind::Silverfish, 1),
        Err(SimError::PlayerLeft)
    );
    assert_eq!(game.send(player(0), MobKind::Silverfish, 1), Ok(()));
    run(&mut game, 3);
    assert!(game.lane(lane(1)).mobs.is_empty());
    assert_eq!(game.lane(lane(2)).mobs.len(), 1);
    game.forfeit(player(2));
    game.forfeit(player(3));
    run(&mut game, 1);
    assert_eq!(
        game.phase(),
        Phase::Ended {
            winner: Some(lane(0))
        }
    );
    assert_eq!(
        game.send(player(0), MobKind::Silverfish, 1),
        Err(SimError::MatchEnded)
    );
}

fn scripted_events() -> Vec<Event> {
    let mut game = solo_match(6);
    for tick in 0..24_000u64 {
        if tick % 400 == 0 {
            let sender = player(u128::from(tick / 400 % 6));
            let kind = MobKind::SENDABLE[(tick / 400 % 8) as usize];
            let _ = game.send(sender, kind, 2);
        }
        game.tick();
    }
    game.drain_events()
}

#[test]
fn spec_tw_23_the_same_script_yields_the_same_events() {
    let first = scripted_events();
    let second = scripted_events();
    assert!(!first.is_empty());
    assert_eq!(first, second);
}
