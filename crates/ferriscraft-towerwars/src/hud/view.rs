use crate::domain::{
    Branch, INCOME_CYCLE_TICKS, LaneId, Level, MAX_GOLD_MINES_PER_PLAYER, PlayerId,
    SUDDEN_DEATH_TICKS, TICKS_PER_SECOND, TIER_UNLOCK_TICKS, TOWER_LIMIT, TowerEffect, TowerKind,
};
use crate::sim::{Lane, Match};

pub const SIDEBAR_TITLE: &str = "§d§lFerrisCraft §8• §e§lTowerWars";
pub const ACTION_BAR_REFRESH_TICKS: u64 = 5;
pub const MAX_NAME_CHARS: usize = 12;

const LANE_COLORS: [&str; 6] = ["§a", "§b", "§c", "§d", "§e", "§f"];
const SECONDS_PER_MINUTE: u64 = 60;
const UNKNOWN_NAME: &str = "?";

pub fn clock(ticks: u64) -> String {
    let seconds = ticks / u64::from(TICKS_PER_SECOND);
    format!(
        "{:02}:{:02}",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

pub fn mine_income(lane: &Lane, player: PlayerId) -> u32 {
    lane.towers()
        .iter()
        .filter(|tower| tower.owner == player)
        .filter(|tower| tower.kind == TowerKind::Beacon && tower.level == Level::T4(Branch::B))
        .filter_map(|tower| match tower.kind.stats(tower.level).effect {
            TowerEffect::GoldMine { income, .. } => Some(income),
            _ => None,
        })
        .take(MAX_GOLD_MINES_PER_PLAYER)
        .sum()
}

pub fn action_bar(game: &Match, player: PlayerId) -> Option<String> {
    let state = game.world().player(player)?;
    let lane = game.lane(state.lane);
    let income = state.income + mine_income(lane, player);
    Some(format!(
        "§6Or: {} ⛃  §8|  §aIncome: +{} / {}s  §8|  §bTours: {}/{}",
        state.gold.get(),
        income,
        INCOME_CYCLE_TICKS / u64::from(TICKS_PER_SECOND),
        lane.towers().len(),
        TOWER_LIMIT
    ))
}

pub fn next_tier(elapsed: u64) -> Option<(u8, u64)> {
    (2..=4u8).find_map(|tier| {
        let unlock = TIER_UNLOCK_TICKS[usize::from(tier) - 1];
        (unlock > elapsed).then(|| (tier, unlock - elapsed))
    })
}

pub fn bossbar_progress(game: &Match) -> f64 {
    let cycle = game.elapsed() % INCOME_CYCLE_TICKS;
    (cycle as f64 / INCOME_CYCLE_TICKS as f64).clamp(0.0, 1.0)
}

pub fn bossbar_title(game: &Match) -> String {
    let remaining = INCOME_CYCLE_TICKS - game.elapsed() % INCOME_CYCLE_TICKS;
    format!(
        "§dTowerWars §7- prochain income dans §f{}s",
        remaining.div_ceil(u64::from(TICKS_PER_SECOND))
    )
}

pub fn waiting_title(players: usize, target: Option<usize>) -> String {
    format!(
        "§dTowerWars §7- en attente §f{players}§7/§f{}",
        target_label(target)
    )
}

pub fn countdown_title(remaining_ticks: u32) -> String {
    format!(
        "§dTowerWars §7- début dans §f{}s",
        remaining_ticks.div_ceil(TICKS_PER_SECOND)
    )
}

fn short(name: &str) -> String {
    name.chars().take(MAX_NAME_CHARS).collect()
}

pub fn lane_names(lane: &Lane, name_of: &impl Fn(PlayerId) -> Option<String>) -> String {
    let names: Vec<String> = lane
        .owners
        .iter()
        .map(|owner| short(&name_of(*owner).unwrap_or_else(|| UNKNOWN_NAME.to_owned())))
        .collect();
    names.join(" & ")
}

fn lane_income(game: &Match, lane: &Lane) -> u32 {
    lane.owners
        .iter()
        .filter_map(|owner| {
            let state = game.world().player(*owner)?;
            Some(state.income + mine_income(lane, *owner))
        })
        .sum()
}

pub fn ranking(game: &Match, name_of: &impl Fn(PlayerId) -> Option<String>) -> Vec<String> {
    let mut lanes: Vec<&Lane> = game
        .world()
        .lanes
        .iter()
        .filter(|lane| !lane.owners.is_empty())
        .collect();
    lanes.sort_by_key(|lane| {
        (
            lane.eliminated,
            std::cmp::Reverse(lane.hp.current()),
            std::cmp::Reverse(lane_income(game, lane)),
            lane.id,
        )
    });
    lanes
        .into_iter()
        .map(|lane| {
            let color = LANE_COLORS[lane.id.index() % LANE_COLORS.len()];
            let names = lane_names(lane, name_of);
            if lane.eliminated {
                format!("{color}§m[✖] {names}")
            } else {
                format!(
                    "{color}[{} ❤] {names} — Income: {}",
                    lane.hp.current(),
                    lane_income(game, lane)
                )
            }
        })
        .collect()
}

pub fn sidebar_lines(game: &Match, name_of: &impl Fn(PlayerId) -> Option<String>) -> Vec<String> {
    let elapsed = game.elapsed();
    let mut lines = vec!["§r".to_owned(), format!("§aTemps §f{}", clock(elapsed))];
    lines.push(match next_tier(elapsed) {
        Some((tier, remaining)) => format!("§eTier {tier} dans §f{}", clock(remaining)),
        None if elapsed < SUDDEN_DEATH_TICKS => format!(
            "§cMort subite dans §f{}",
            clock(SUDDEN_DEATH_TICKS - elapsed)
        ),
        None => "§4§lMort subite".to_owned(),
    });
    lines.push("§r§r".to_owned());
    lines.extend(ranking(game, name_of));
    lines
}

fn target_label(target: Option<usize>) -> String {
    target.map_or_else(|| "?".to_owned(), |target| target.to_string())
}

pub fn lobby_lines(players: usize, target: Option<usize>, countdown: Option<u32>) -> Vec<String> {
    let status = match countdown {
        Some(remaining) => format!("§eDébut dans §f{}s", remaining.div_ceil(TICKS_PER_SECOND)),
        None => format!("§7En attente §f{players}§7/§f{}", target_label(target)),
    };
    vec!["§r".to_owned(), status, format!("§7Joueurs §f{players}")]
}

pub fn lane_color(lane: LaneId) -> &'static str {
    LANE_COLORS[lane.index() % LANE_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{GameMode, MobKind, TowerKind};
    use crate::map::Map;
    use crate::sim::Seat;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn game(count: u8) -> Match {
        let seats: Vec<Seat> = (0..count)
            .map(|index| Seat {
                player: pid(u128::from(index)),
                lane: LaneId::try_from(index).expect("lane"),
            })
            .collect();
        Match::new(GameMode::Solo, 7, &Map::test_map(), &seats).expect("match")
    }

    fn names(id: PlayerId) -> Option<String> {
        Some(format!("P{}", id.0.as_u128()))
    }

    #[test]
    fn clock_is_minutes_and_seconds() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(20 * 75), "01:15");
    }

    #[test]
    fn action_bar_shows_gold_income_and_towers() {
        let game = game(4);
        let line = action_bar(&game, pid(0)).expect("line");
        assert!(line.contains("Or: 100 ⛃"), "{line}");
        assert!(line.contains("Income: +10 / 6s"), "{line}");
        assert!(line.contains("Tours: 0/80"), "{line}");
    }

    #[test]
    fn action_bar_follows_sends_and_placements() {
        let mut game = game(4);
        game.send(pid(0), MobKind::Silverfish, 1).expect("send");
        let line = action_bar(&game, pid(0)).expect("line");
        assert!(line.contains("Or: 85 ⛃"), "{line}");
        assert!(line.contains("Income: +12 / 6s"), "{line}");
        let lane = game.lane(LaneId::try_from(0).expect("lane"));
        assert!(lane.is_buildable((2, 1)));
        game.place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        let line = action_bar(&game, pid(0)).expect("line");
        assert!(line.contains("Or: 35 ⛃"), "{line}");
        assert!(line.contains("Tours: 1/80"), "{line}");
    }

    #[test]
    fn action_bar_is_none_for_a_stranger() {
        assert!(action_bar(&game(4), pid(9)).is_none());
    }

    #[test]
    fn next_tier_counts_down_to_the_following_unlock() {
        assert_eq!(next_tier(0), Some((2, 3_000)));
        assert_eq!(next_tier(2_900), Some((2, 100)));
        assert_eq!(next_tier(3_000), Some((3, 3_600)));
        assert_eq!(next_tier(10_799), Some((4, 1)));
        assert_eq!(next_tier(10_800), None);
    }

    #[test]
    fn bossbar_tracks_the_income_cycle() {
        let mut game = game(4);
        assert_eq!(bossbar_progress(&game), 0.0);
        for _ in 0..60 {
            game.tick();
        }
        assert!((bossbar_progress(&game) - 0.5).abs() < 1e-9);
        assert!(
            bossbar_title(&game).contains("3s"),
            "{}",
            bossbar_title(&game)
        );
        for _ in 0..60 {
            game.tick();
        }
        assert_eq!(bossbar_progress(&game), 0.0);
    }

    #[test]
    fn ranking_lists_every_occupied_lane_with_hp_and_income() {
        let game = game(4);
        let lines = ranking(&game, &names);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("[20 ❤] P1 — Income: 10"), "{}", lines[0]);
    }

    #[test]
    fn ranking_puts_the_richest_first_and_the_eliminated_last() {
        let mut game = game(4);
        game.send(pid(2), MobKind::Zombie, 1).expect("send");
        let lines = ranking(&game, &names);
        assert!(lines[0].contains("P3"), "{lines:?}");
        assert!(lines[0].contains("Income: 14"), "{lines:?}");

        game.forfeit(pid(0));
        game.tick();
        let lines = ranking(&game, &names);
        assert!(lines[3].contains("✖"), "{lines:?}");
        assert!(lines[3].contains("P1"), "{lines:?}");
    }

    #[test]
    fn ranking_lines_are_unique() {
        let game = game(6);
        let mut lines = ranking(&game, &names);
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn long_names_are_cut() {
        let game = game(4);
        let lines = ranking(&game, &|_| Some("A very long player name".to_owned()));
        assert!(lines[0].contains("A very long "), "{}", lines[0]);
        assert!(!lines[0].contains("A very long p"), "{}", lines[0]);
    }

    #[test]
    fn sidebar_shows_time_next_tier_and_ranking() {
        let game = game(4);
        let lines = sidebar_lines(&game, &names);
        assert!(lines.iter().any(|line| line.contains("Temps §f00:00")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Tier 2 dans §f02:30"))
        );
        assert_eq!(lines.len(), 4 + 4);
        let mut unique = lines.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), lines.len());
    }

    #[test]
    fn sidebar_announces_sudden_death_after_the_last_tier() {
        let mut game = game(4);
        for _ in 0..10_800 {
            game.tick();
        }
        let lines = sidebar_lines(&game, &names);
        assert!(lines.iter().any(|line| line.contains("Mort subite dans")));
    }

    #[test]
    fn lobby_lines_switch_from_waiting_to_countdown() {
        assert!(lobby_lines(2, Some(4), None)[1].contains("2§7/§f4"));
        assert!(lobby_lines(4, Some(4), Some(100))[1].contains("5s"));
    }

    #[test]
    fn titles_show_the_lobby_state() {
        assert!(waiting_title(1, Some(4)).contains("1§7/§f4"));
        assert!(waiting_title(1, None).contains("1§7/§f?"));
        assert!(lobby_lines(1, None, None)[1].contains("1§7/§f?"));
        assert!(countdown_title(41).contains("3s"));
    }
}
