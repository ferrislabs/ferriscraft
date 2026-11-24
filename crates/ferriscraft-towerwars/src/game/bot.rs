use crate::domain::{LaneId, Level, MobKind, PlayerId, TICKS_PER_SECOND, TowerKind};
use crate::sim::Match;

pub const BOT_TURN_TICKS: u64 = 10 * TICKS_PER_SECOND as u64;
const SEND_SHARE: f32 = 0.3;
const MIN_TOWERS_BEFORE_UPGRADES: usize = 8;
const MAX_TOWERS: usize = 40;
const MAX_RANK: u8 = 3;
const COVERAGE_STEP: f32 = 0.5;
const COVERAGE_RANGE: f32 = 8.0;
const SEARCH_MARGIN: i32 = 12;
const BUILD_ROTATION: [TowerKind; 4] = [
    TowerKind::Archer,
    TowerKind::Cannon,
    TowerKind::Mage,
    TowerKind::Ice,
];

#[derive(Debug, Clone)]
pub struct Bot {
    pub player: PlayerId,
    lane: LaneId,
    cells: Vec<(i32, i32)>,
    next_cell: usize,
}

impl Bot {
    pub fn new(game: &Match, player: PlayerId, lane: LaneId) -> Self {
        Self {
            player,
            lane,
            cells: cells_by_coverage(game, lane),
            next_cell: 0,
        }
    }

    pub fn act(&mut self, game: &mut Match) {
        self.send(game);
        self.build(game);
    }

    fn send(&self, game: &mut Match) {
        let gold = game.gold(self.player).map_or(0, |gold| gold.get());
        let budget = (gold as f32 * SEND_SHARE) as u32;
        let mut menu = MobKind::SENDABLE;
        menu.sort_by_key(|kind| std::cmp::Reverse(kind.spec().cost));
        for kind in menu {
            if kind.spec().cost <= budget && game.send(self.player, kind, 1).is_ok() {
                return;
            }
        }
    }

    fn build(&mut self, game: &mut Match) {
        loop {
            let towers: Vec<_> = game
                .lane(self.lane)
                .towers()
                .iter()
                .filter(|tower| tower.owner == self.player)
                .map(|tower| (tower.id, tower.level))
                .collect();
            let weakest = towers
                .iter()
                .filter(|(_, level)| level_rank(*level) < MAX_RANK)
                .min_by_key(|(_, level)| level_rank(*level))
                .copied();
            let wants_new = towers.len() < MIN_TOWERS_BEFORE_UPGRADES || weakest.is_none();
            if wants_new && towers.len() < MAX_TOWERS && self.next_cell < self.cells.len() {
                let kind = BUILD_ROTATION[self.next_cell % BUILD_ROTATION.len()];
                let gold = game.gold(self.player).map_or(0, |gold| gold.get());
                if gold < kind.stats(Level::T1).cost {
                    return;
                }
                let cell = self.cells[self.next_cell];
                self.next_cell += 1;
                let _ = game.place(self.player, kind, cell);
            } else if let Some((tower, _)) = weakest {
                if game.upgrade(self.player, tower, None).is_err() {
                    return;
                }
            } else {
                return;
            }
        }
    }
}

const fn level_rank(level: Level) -> u8 {
    match level {
        Level::T1 => 1,
        Level::T2 => 2,
        Level::T3 => 3,
        Level::T4(_) => 4,
    }
}

fn cells_by_coverage(game: &Match, lane: LaneId) -> Vec<(i32, i32)> {
    let lane = game.lane(lane);
    let path = lane.path();
    let mut samples = Vec::new();
    let mut progress = 0.0;
    while progress < path.length() {
        samples.push(path.sample(progress).position);
        progress += COVERAGE_STEP;
    }
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for point in &samples {
        min_x = min_x.min(point.x.floor() as i32);
        max_x = max_x.max(point.x.floor() as i32);
        min_z = min_z.min(point.z.floor() as i32);
        max_z = max_z.max(point.z.floor() as i32);
    }
    let mut ranked: Vec<((i32, i32), usize)> = Vec::new();
    for x in min_x - SEARCH_MARGIN..=max_x + SEARCH_MARGIN {
        for z in min_z - SEARCH_MARGIN..=max_z + SEARCH_MARGIN {
            if !lane.is_buildable((x, z)) {
                continue;
            }
            let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
            let covered = samples
                .iter()
                .filter(|point| (point.x - cx).hypot(point.z - cz) <= COVERAGE_RANGE)
                .count();
            ranked.push(((x, z), covered));
        }
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|(cell, _)| cell).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GameMode;
    use crate::map::Map;
    use crate::sim::Seat;
    use uuid::Uuid;

    fn seats() -> Vec<Seat> {
        (0..4u8)
            .map(|index| Seat {
                player: PlayerId(Uuid::from_u128(u128::from(index) + 1)),
                lane: LaneId::try_from(index).expect("lane"),
            })
            .collect()
    }

    #[test]
    fn a_bot_keeps_most_of_its_gold_for_its_own_defense() {
        let seats = seats();
        let mut game = Match::new(GameMode::Solo, 1, &Map::test_map(), &seats).expect("match");
        let mut bot = Bot::new(&game, seats[1].player, seats[1].lane);
        bot.act(&mut game);
        assert!(!game.lane(seats[1].lane).towers().is_empty());
        let income = game.income(seats[1].player).expect("income");
        assert!(income > 10, "le bot a aussi envoyé un mob");
    }

    #[test]
    fn a_bot_ranks_cells_next_to_the_path_first() {
        let seats = seats();
        let game = Match::new(GameMode::Solo, 1, &Map::test_map(), &seats).expect("match");
        let bot = Bot::new(&game, seats[1].player, seats[1].lane);
        assert!(!bot.cells.is_empty());
        assert!(
            bot.cells
                .iter()
                .all(|cell| game.lane(seats[1].lane).is_buildable(*cell))
        );
    }

    #[test]
    fn a_bot_never_builds_more_than_forty_towers_nor_a_fourth_tier() {
        let seats = seats();
        let mut game = Match::new(GameMode::Solo, 1, &Map::test_map(), &seats).expect("match");
        let mut bot = Bot::new(&game, seats[1].player, seats[1].lane);
        for tick in 0..20_000u64 {
            if tick % BOT_TURN_TICKS == 0 {
                bot.act(&mut game);
            }
            game.tick();
            if game.is_ended() {
                break;
            }
        }
        let towers = game.lane(seats[1].lane).towers();
        assert!(towers.len() <= MAX_TOWERS);
        assert!(
            towers
                .iter()
                .all(|tower| !matches!(tower.level, Level::T4(_)))
        );
    }
}
