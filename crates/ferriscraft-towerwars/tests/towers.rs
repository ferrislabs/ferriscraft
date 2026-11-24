mod common;

use common::{buildable_cell, player, run, solo_match, straight_map};
use ferriscraft_towerwars::domain::{
    Branch, GameMode, Gold, LaneId, Level, Priority, SimError, TowerEffect, TowerKind, Vec3,
};
use ferriscraft_towerwars::sim::{Event, Match, Seat};

const COSTS: [(TowerKind, [u32; 4]); 7] = [
    (TowerKind::Archer, [50, 100, 250, 600]),
    (TowerKind::Ice, [75, 150, 350, 800]),
    (TowerKind::Mage, [100, 220, 480, 1100]),
    (TowerKind::Cannon, [125, 275, 600, 1400]),
    (TowerKind::Tesla, [150, 320, 700, 1600]),
    (TowerKind::Slime, [120, 260, 550, 1250]),
    (TowerKind::Beacon, [200, 450, 950, 2000]),
];

fn lane(index: u8) -> LaneId {
    LaneId::try_from(index).expect("lane")
}

fn teams_match() -> Match {
    let seats: Vec<Seat> = [(0u128, 0u8), (1, 0), (2, 1), (3, 2), (4, 3), (5, 4)]
        .iter()
        .map(|(index, lane_index)| Seat {
            player: player(*index),
            lane: lane(*lane_index),
        })
        .collect();
    Match::new(GameMode::Teams, 7, &straight_map(), &seats).expect("match")
}

#[test]
fn every_kind_and_level_costs_the_catalog_price() {
    for (kind, costs) in COSTS {
        assert_eq!(kind.stats(Level::T1).cost, costs[0], "{kind:?} T1");
        assert_eq!(kind.stats(Level::T2).cost, costs[1], "{kind:?} T2");
        assert_eq!(kind.stats(Level::T3).cost, costs[2], "{kind:?} T3");
        assert_eq!(
            kind.stats(Level::T4(Branch::A)).cost,
            costs[3],
            "{kind:?} A"
        );
        assert_eq!(
            kind.stats(Level::T4(Branch::B)).cost,
            costs[3],
            "{kind:?} B"
        );
    }
}

#[test]
fn totals_invested_per_path_match_the_design() {
    let total = |kind: TowerKind, level: Level| -> u32 {
        level
            .history()
            .iter()
            .map(|step| kind.stats(*step).cost)
            .sum()
    };
    assert_eq!(total(TowerKind::Archer, Level::T3), 400);
    assert_eq!(total(TowerKind::Archer, Level::T4(Branch::A)), 1000);
    assert_eq!(total(TowerKind::Ice, Level::T4(Branch::B)), 1375);
    assert_eq!(total(TowerKind::Mage, Level::T4(Branch::A)), 1900);
    assert_eq!(total(TowerKind::Cannon, Level::T4(Branch::B)), 2400);
    assert_eq!(total(TowerKind::Tesla, Level::T4(Branch::A)), 2770);
    assert_eq!(total(TowerKind::Slime, Level::T4(Branch::B)), 2180);
    assert_eq!(total(TowerKind::Beacon, Level::T4(Branch::B)), 3600);
}

#[test]
fn catalog_combat_numbers_follow_the_design() {
    let archer = TowerKind::Archer;
    let t1 = archer.stats(Level::T1);
    assert_eq!((t1.damage, t1.cooldown_ticks, t1.range), (12.0, 20, 8.0));
    let marksman = archer.stats(Level::T4(Branch::A));
    assert_eq!(
        (marksman.damage, marksman.cooldown_ticks, marksman.range),
        (180.0, 24, 14.0)
    );
    assert_eq!(
        marksman.effect,
        TowerEffect::Crit {
            chance: 0.25,
            multiplier: 2.5
        }
    );
    assert_eq!(archer.stats(Level::T4(Branch::B)).cooldown_ticks, 5);
    assert_eq!(
        TowerKind::Mage.stats(Level::T1).effect,
        TowerEffect::Burn {
            dps: 5.0,
            ticks: 60
        }
    );
    assert_eq!(
        TowerKind::Tesla.stats(Level::T4(Branch::B)).effect,
        TowerEffect::Chain {
            targets: 4,
            bounce: 4.5,
            stun_every: Some((3, 8))
        }
    );
    assert_eq!(
        TowerKind::Beacon.stats(Level::T1).effect,
        TowerEffect::Beacon {
            attack_speed: 0.10,
            range: 0.0,
            damage: 0.0,
            radius: 4.5
        }
    );
    assert_eq!(
        TowerKind::Beacon.stats(Level::T4(Branch::B)).effect,
        TowerEffect::GoldMine {
            income: 15,
            attack_speed: 0.15,
            radius: 5.5
        }
    );
}

#[test]
fn placing_spends_gold_and_creates_a_first_priority_tower() {
    let mut game = solo_match(6);
    let owner = player(0);
    let cell = buildable_cell(0, 5);
    let id = game.place(owner, TowerKind::Archer, cell).expect("place");
    assert_eq!(game.gold(owner), Some(Gold::new(50)));
    let tower = game.lane(lane(0)).tower(id).expect("tower");
    assert_eq!(tower.priority, Priority::First);
    assert_eq!(tower.invested, 50);
    assert_eq!(tower.cooldown_ticks, 0);
    assert_eq!(tower.owner, owner);
    assert_eq!(
        tower.world_pos,
        Vec3::new(cell.0 as f32 + 0.5, 64.0, cell.1 as f32 + 0.5)
    );
    assert!(game.drain_events().iter().any(|event| matches!(
        event,
        Event::TowerPlaced { tower, kind: TowerKind::Archer, .. } if *tower == id
    )));
}

#[test]
fn an_out_of_bounds_or_occupied_cell_is_refused_without_spending_gold() {
    let mut game = solo_match(6);
    let owner = player(0);
    let result = game.place(owner, TowerKind::Archer, (500, 500));
    assert_eq!(result, Err(SimError::OutOfBuildBounds));
    assert_eq!(game.gold(owner), Some(Gold::new(100)));
    let cell = buildable_cell(0, 0);
    game.place(owner, TowerKind::Archer, cell).expect("place");
    let result = game.place(owner, TowerKind::Archer, cell);
    assert_eq!(result, Err(SimError::OccupiedCell));
    assert_eq!(game.gold(owner), Some(Gold::new(50)));
}

#[test]
fn another_lanes_cells_are_out_of_bounds() {
    let mut game = solo_match(6);
    let result = game.place(player(0), TowerKind::Archer, buildable_cell(1, 0));
    assert_eq!(result, Err(SimError::OutOfBuildBounds));
}

#[test]
fn placing_without_enough_gold_is_refused() {
    let mut game = solo_match(6);
    let owner = player(0);
    let result = game.place(owner, TowerKind::Tesla, buildable_cell(0, 0));
    assert_eq!(result, Err(SimError::NotEnoughGold));
    assert_eq!(game.gold(owner), Some(Gold::new(100)));
    assert!(game.lane(lane(0)).towers().is_empty());
}

#[test]
fn an_unknown_player_cannot_place() {
    let mut game = solo_match(6);
    let result = game.place(player(40), TowerKind::Archer, buildable_cell(0, 0));
    assert_eq!(result, Err(SimError::UnknownPlayer));
}

#[test]
fn selling_refunds_eighty_percent_and_frees_the_cell() {
    let mut game = solo_match(6);
    let owner = player(0);
    let cell = buildable_cell(0, 0);
    let id = game.place(owner, TowerKind::Ice, cell).expect("place");
    assert_eq!(game.sell(owner, id), Ok(60));
    assert_eq!(game.gold(owner), Some(Gold::new(85)));
    assert!(game.lane(lane(0)).tower(id).is_none());
    assert!(game.place(owner, TowerKind::Archer, cell).is_ok());
    assert!(
        game.drain_events()
            .iter()
            .any(|event| matches!(event, Event::TowerSold { refund: 60, .. }))
    );
}

#[test]
fn upgrading_without_gold_leaves_the_tower_unchanged() {
    let mut game = solo_match(6);
    let owner = player(0);
    let id = game
        .place(owner, TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    assert_eq!(game.upgrade(owner, id, None), Err(SimError::NotEnoughGold));
    assert_eq!(
        game.lane(lane(0)).tower(id).expect("tower").level,
        Level::T1
    );
    assert_eq!(game.gold(owner), Some(Gold::new(50)));
}

#[test]
fn a_branch_cannot_be_chosen_before_the_third_level() {
    let mut game = solo_match(6);
    let owner = player(0);
    let id = game
        .place(owner, TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    assert_eq!(
        game.upgrade(owner, id, Some(Branch::A)),
        Err(SimError::BranchChoice)
    );
}

#[test]
fn priority_is_changed_by_the_owner_only() {
    let mut game = solo_match(6);
    let id = game
        .place(player(0), TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    game.set_priority(player(0), id, Priority::Strongest)
        .expect("priority");
    let tower = game.lane(lane(0)).tower(id).expect("tower");
    assert_eq!(tower.priority, Priority::Strongest);
    assert_eq!(
        game.set_priority(player(1), id, Priority::Last),
        Err(SimError::NotOwner)
    );
    assert_eq!(
        game.set_priority(
            player(0),
            ferriscraft_towerwars::domain::TowerId(99),
            Priority::Last
        ),
        Err(SimError::UnknownTower)
    );
}

#[test]
fn another_players_tower_cannot_be_sold_or_upgraded() {
    let mut game = solo_match(6);
    let id = game
        .place(player(0), TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    assert_eq!(game.sell(player(1), id), Err(SimError::NotOwner));
    assert_eq!(game.upgrade(player(1), id, None), Err(SimError::NotOwner));
    assert_eq!(game.gold(player(1)), Some(Gold::new(100)));
}

#[test]
fn teammates_share_the_lane_but_not_the_gold() {
    let mut game = teams_match();
    let (first, second) = (player(0), player(1));
    let id = game
        .place(first, TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    assert_eq!(game.gold(first), Some(Gold::new(50)));
    game.set_priority(second, id, Priority::Weakest)
        .expect("priority");
    assert_eq!(game.sell(second, id), Ok(40));
    assert_eq!(game.gold(second), Some(Gold::new(140)));
    assert_eq!(game.gold(first), Some(Gold::new(50)));
}

#[test]
fn a_forfeited_and_eliminated_player_refuses_tower_commands() {
    let mut game = solo_match(6);
    let id = game
        .place(player(0), TowerKind::Archer, buildable_cell(0, 0))
        .expect("place");
    game.forfeit(player(0));
    run(&mut game, 1);
    assert!(game.lane(lane(0)).eliminated);
    let result = game.place(player(0), TowerKind::Archer, buildable_cell(0, 1));
    assert_eq!(result, Err(SimError::PlayerLeft));
    assert_eq!(game.sell(player(0), id), Err(SimError::PlayerLeft));
}
