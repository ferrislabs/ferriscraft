use ferriscraft_towerwars::domain::{
    Branch, GameMode, LaneId, Level, MobKind, PlayerId, TowerKind,
};
use ferriscraft_towerwars::map::Map;
use ferriscraft_towerwars::sim::{Event, Match, Seat};
use uuid::Uuid;

const MAX_TICKS: u64 = 25 * 60 * 20;
const BUILD_ROTATION: [TowerKind; 4] = [
    TowerKind::Archer,
    TowerKind::Cannon,
    TowerKind::Mage,
    TowerKind::Ice,
];

#[derive(Clone, Copy, Debug)]
struct BotStyle {
    every: u64,
    send_share: f32,
}

fn pid(index: u8) -> PlayerId {
    PlayerId(Uuid::from_u128(u128::from(index) + 1))
}

fn lane(index: u8) -> LaneId {
    LaneId::try_from(index).expect("lane")
}

fn best_cells(game: &Match, at: LaneId) -> Vec<(i32, i32)> {
    let lane_ref = game.lane(at);
    let path = lane_ref.path();
    let mut samples = Vec::new();
    let mut progress = 0.0;
    while progress < path.length() {
        samples.push(path.sample(progress).position);
        progress += 0.5;
    }
    let mut cells: Vec<((i32, i32), usize)> = Vec::new();
    for x in -50..200 {
        for z in -50..200 {
            if lane_ref.is_buildable((x, z)) {
                let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
                let covered = samples
                    .iter()
                    .filter(|p| (p.x - cx).hypot(p.z - cz) <= 8.0)
                    .count();
                cells.push(((x, z), covered));
            }
        }
    }
    cells.sort_by_key(|cell| std::cmp::Reverse(cell.1));
    cells.into_iter().map(|(cell, _)| cell).collect()
}

fn level_rank(level: Level) -> u8 {
    match level {
        Level::T1 => 1,
        Level::T2 => 2,
        Level::T3 => 3,
        Level::T4(_) => 4,
    }
}

fn build(
    game: &mut Match,
    me: PlayerId,
    at: LaneId,
    cells: &[(i32, i32)],
    built: &mut usize,
    limits: (usize, u8),
) {
    loop {
        let gold = game.gold(me).map_or(0, |gold| gold.get());
        let towers: Vec<_> = game
            .lane(at)
            .towers()
            .iter()
            .map(|tower| (tower.id, tower.level))
            .collect();
        let upgradable = towers
            .iter()
            .filter(|(_, level)| level_rank(*level) < limits.1)
            .min_by_key(|(_, level)| level_rank(*level))
            .copied();
        let want_new = towers.len() < 8 || upgradable.is_none();
        if want_new && towers.len() < limits.0 && *built < cells.len() {
            let kind = BUILD_ROTATION[*built % BUILD_ROTATION.len()];
            if gold < kind.stats(Level::T1).cost {
                return;
            }
            let _ = game.place(me, kind, cells[*built]);
            *built += 1;
        } else if let Some((id, level)) = upgradable {
            let branch = matches!(level, Level::T3).then_some(Branch::A);
            if game.upgrade(me, id, branch).is_err() {
                return;
            }
        } else {
            return;
        }
    }
}

fn send(game: &mut Match, bot: PlayerId, share: f32) {
    let gold = game.gold(bot).map_or(0, |gold| gold.get()) as f32;
    let budget = (gold * share) as u32;
    let mut menu: Vec<MobKind> = MobKind::SENDABLE.to_vec();
    menu.sort_by_key(|kind| std::cmp::Reverse(kind.spec().cost));
    for kind in menu {
        if kind.spec().cost <= budget && game.send(bot, kind, 1).is_ok() {
            return;
        }
    }
}

fn spam(game: &mut Match, me: PlayerId) {
    let mut menu: Vec<MobKind> = MobKind::SENDABLE.to_vec();
    menu.sort_by_key(|kind| std::cmp::Reverse(kind.spec().cost));
    for kind in menu {
        while game.send(me, kind, 1).is_ok() {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Human {
    Defender,
    Spammer,
}

#[derive(Default)]
struct Outcome {
    human_dead_at: Option<u64>,
    winner: Option<String>,
    end: u64,
    max_income: u32,
    max_gold: u32,
    max_mobs: usize,
    income_at: Vec<u32>,
}

const BOT: BotStyle = BotStyle {
    every: 200,
    send_share: 0.3,
};

fn run(map: &Map, human: Human, seed: u64) -> Outcome {
    let seats: Vec<Seat> = (0..4u8)
        .map(|index| Seat {
            player: pid(index),
            lane: lane(index),
        })
        .collect();
    let mut game = Match::new(GameMode::Solo, seed, map, &seats).expect("match");
    let cells: Vec<Vec<(i32, i32)>> = (0..4u8).map(|i| best_cells(&game, lane(i))).collect();
    let mut built = [0usize; 4];
    let mut outcome = Outcome::default();
    for _ in 0..MAX_TICKS {
        let elapsed = game.elapsed();
        if elapsed.is_multiple_of(10) {
            build(
                &mut game,
                pid(0),
                lane(0),
                &cells[0],
                &mut built[0],
                (80, 4),
            );
            if human == Human::Spammer && elapsed > 1200 {
                spam(&mut game, pid(0));
            }
        }
        if elapsed.is_multiple_of(BOT.every) {
            for bot in 1..4u8 {
                send(&mut game, pid(bot), BOT.send_share);
                let index = usize::from(bot);
                build(
                    &mut game,
                    pid(bot),
                    lane(bot),
                    &cells[index],
                    &mut built[index],
                    bot_limits(),
                );
            }
        }
        game.tick();
        for event in game.drain_events() {
            match event {
                Event::LaneEliminated { lane: at } if at == lane(0) => {
                    outcome.human_dead_at = Some(game.elapsed());
                }
                Event::Won { lane: at } => {
                    outcome.winner = Some(match at {
                        Some(at) if at == lane(0) => "joueur".to_owned(),
                        Some(_) => "un bot".to_owned(),
                        None => "égalité".to_owned(),
                    });
                }
                _ => {}
            }
        }
        let income = game.income(pid(0)).unwrap_or(0);
        outcome.max_income = outcome.max_income.max(income);
        outcome.max_gold = outcome
            .max_gold
            .max(game.gold(pid(0)).map_or(0, |g| g.get()));
        let mobs = game
            .world()
            .lanes
            .iter()
            .map(|l| l.mobs.len())
            .max()
            .unwrap_or(0);
        outcome.max_mobs = outcome.max_mobs.max(mobs);
        if game.elapsed().is_multiple_of(6000) {
            outcome.income_at.push(income);
        }
        if game.is_ended() || outcome.human_dead_at.is_some() {
            break;
        }
    }
    outcome.end = game.elapsed();
    outcome
}

fn bot_limits() -> (usize, u8) {
    let towers = std::env::var("BOT_TOWERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let rank = std::env::var("BOT_RANK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    (towers, rank)
}

fn clock(ticks: u64) -> String {
    format!("{}:{:02}", ticks / 1200, ticks / 20 % 60)
}

fn main() {
    let map = Map::test_map();
    for human in [Human::Defender, Human::Spammer] {
        println!("== {human:?}");
        for seed in [1, 2, 3] {
            let o = run(&map, human, seed);
            println!(
                "  graine {seed} : fin {} gagnant {:?} | income max {} (toutes les 5 min {:?}) | or max {} | mobs max sur une voie {}",
                clock(o.end),
                o.winner.unwrap_or_else(|| "aucun".into()),
                o.max_income,
                o.income_at,
                o.max_gold,
                o.max_mobs
            );
        }
    }
}
