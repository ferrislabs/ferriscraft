use std::collections::HashMap;
use std::sync::RwLock;

use rspigot_plugin::{
    Criteria, DisplaySlot, Error, Objective, Player, Scoreboard, Server, TabList, TeamColor, Uuid,
};

use crate::auth::{AuthManager, Rank};

const TEAM_DEFS: [(&str, &str, &str, TeamColor); 7] = [
    ("00_admin_fk", "§c§lADMIN §f", " §a✔", TeamColor::Red),
    ("01_admin", "§c§lADMIN §f", "", TeamColor::Red),
    ("10_mod", "§9§lMOD §f", " §a✔", TeamColor::Blue),
    ("20_dev", "§b§lDEV §f", " §a✔", TeamColor::Aqua),
    ("30_vip", "§6§lVIP §f", " §a✔", TeamColor::Gold),
    ("40_member", "§a§lFK §f", " §a✔", TeamColor::Green),
    ("90_guest", "§7", "", TeamColor::Gray),
];

#[derive(Debug, Clone, Copy, Default)]
pub struct HubMetrics {
    pub online: u32,
    pub max_players: u32,
    pub tps_1m: f64,
    pub sheepwars_queued: usize,
    pub agents_working: usize,
    pub agents_total: usize,
}

#[derive(Debug, Clone)]
pub struct PlayerHudInput {
    pub rank: Rank,
    pub ferriskey_username: Option<String>,
    pub ping_ms: i32,
    pub metrics: HubMetrics,
}

pub fn build_sidebar_lines(input: &PlayerHudInput) -> Vec<String> {
    let fk_line = match &input.ferriskey_username {
        Some(user) => {
            let short: String = user.chars().take(14).collect();
            format!("§fFerrisKey : §a✔ {short}")
        }
        None => "§fFerrisKey : §e/login".to_owned(),
    };
    let tps = input.metrics.tps_1m.clamp(0.0, 20.0);
    let tps_color = if tps >= 19.0 {
        "§a"
    } else if tps >= 16.0 {
        "§e"
    } else {
        "§c"
    };
    let ping = input.ping_ms.max(0);

    vec![
        "§1".to_owned(),
        format!("§fGrade : {}", input.rank.sidebar_label()),
        fk_line,
        "§2".to_owned(),
        format!(
            "§fEn ligne : §a{}§7/§f{}",
            input.metrics.online, input.metrics.max_players
        ),
        format!("§fSheepWars : §d{} prêt(s)", input.metrics.sheepwars_queued),
        format!(
            "§fCapitainerie : §b{}§7/§f{}",
            input.metrics.agents_working, input.metrics.agents_total
        ),
        "§3".to_owned(),
        format!("§fPing : §a{ping}ms §8| {tps_color}{tps:.1} TPS"),
        "§4".to_owned(),
        "§6play.ferriscraft.gg".to_owned(),
    ]
}

pub fn build_tab_list(input: &PlayerHudInput) -> TabList {
    let fk_status = match &input.ferriskey_username {
        Some(user) => format!("§a✔ {user}"),
        None => "§eNon lié (/login)".to_owned(),
    };
    let tps = input.metrics.tps_1m.clamp(0.0, 20.0);
    let ping = input.ping_ms.max(0);

    let header = format!(
        "\n§6§l✦ FERRISCRAFT §8• §d§lHUB MINI-JEUX ✦\n§7Île Nordique §8• §fJoueurs : §a{}§7/§f{}\n",
        input.metrics.online, input.metrics.max_players
    );
    let footer = format!(
        "\n§7Grade : {} §8• §7Compte : {fk_status}\n§7Ping : §a{ping} ms §8• §7TPS : §a{tps:.1} §8• §7SheepWars : §d{} prêt(s)\n§6/menu §8• §d/sheepwars §8• §e/options §8• §b/login\n",
        input.rank.sidebar_label(),
        input.metrics.sheepwars_queued
    );
    TabList::new(header, footer)
}

struct BoardEntry {
    board: Scoreboard,
    objective: Objective,
    last_lines: Vec<String>,
    last_tab: Option<TabList>,
}

#[derive(Default)]
pub struct HudManager {
    boards: RwLock<HashMap<Uuid, BoardEntry>>,
    team_by_player: RwLock<HashMap<String, &'static str>>,
}

impl HudManager {
    pub async fn attach_player(&self, server: &Server, player: &Player) -> Result<(), Error> {
        self.detach_player(player.uuid, &player.name);

        let board = server.scoreboard_manager.new_scoreboard().await?;
        let objective =
            board.register_objective("fc_hub", &Criteria::dummy(), "§6§lFERRISCRAFT §8• §dHUB");
        board.set_display_slot(DisplaySlot::Sidebar, Some(&objective));

        for (id, prefix, suffix, color) in TEAM_DEFS {
            let team = board.register_team(id);
            team.set_prefix(prefix);
            team.set_suffix(suffix);
            team.set_color(Some(color));
        }

        // Populate existing players' teams on this fresh board so TabList renders immediately.
        let existing_teams: Vec<(String, &'static str)> = self
            .team_by_player
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(name, team)| (name.clone(), *team))
            .collect();
        for (name, team_id) in existing_teams {
            board.team(team_id).add_entry(name);
        }

        player.scoreboard.set(&board);
        self.boards
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                player.uuid,
                BoardEntry {
                    board,
                    objective,
                    last_lines: Vec::new(),
                    last_tab: None,
                },
            );
        Ok(())
    }

    pub fn detach_player(&self, uuid: Uuid, player_name: &str) {
        let removed_board = self
            .boards
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
        if let Some(entry) = removed_board {
            entry.board.discard();
        }

        let old_team = self
            .team_by_player
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(player_name);
        if let Some(team_id) = old_team {
            let boards = self.boards.read().unwrap_or_else(|e| e.into_inner());
            for entry in boards.values() {
                entry.board.team(team_id).remove_entry(player_name);
            }
        }
    }

    pub fn sync_player_team(&self, player_name: &str, rank: Rank, verified: bool) {
        let target_team = rank.team_name(verified);
        let previous = self
            .team_by_player
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player_name.to_owned(), target_team);

        if previous == Some(target_team) {
            return;
        }

        let boards = self.boards.read().unwrap_or_else(|e| e.into_inner());
        for entry in boards.values() {
            if let Some(old) = previous {
                entry.board.team(old).remove_entry(player_name);
            }
            entry.board.team(target_team).add_entry(player_name);
        }
    }

    pub fn refresh_player(&self, player: &Player, input: &PlayerHudInput) {
        self.sync_player_team(&player.name, input.rank, input.ferriskey_username.is_some());

        let lines = build_sidebar_lines(input);
        let tab = build_tab_list(input);

        let mut boards = self.boards.write().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = boards.get_mut(&player.uuid) {
            let total = lines.len();
            if entry.last_lines.len() != total {
                for old in &entry.last_lines {
                    entry.board.reset_scores(old);
                }
                for (idx, line) in lines.iter().enumerate() {
                    let score = (total - idx) as i32;
                    entry.objective.score(line).set(score);
                }
            } else {
                for (idx, (old, new)) in entry.last_lines.iter().zip(&lines).enumerate() {
                    if old != new {
                        let score = (total - idx) as i32;
                        entry.board.reset_scores(old);
                        entry.objective.score(new).set(score);
                    }
                }
            }
            entry.last_lines = lines;

            if entry.last_tab.as_ref() != Some(&tab) {
                player.tab_list.set(tab.clone());
                entry.last_tab = Some(tab);
            }
        } else {
            player.tab_list.set(tab);
        }
    }

    pub async fn refresh_all(&self, server: &Server, auth: &AuthManager, metrics: HubMetrics) {
        let Ok(players) = server.get_players().await else {
            return;
        };
        for player in players {
            let (ping_ms, is_op) = match player.get_state().await {
                Ok(state) => (state.ping, state.op),
                Err(_) => (0, false),
            };
            let claims = auth.claims_of(player.uuid);
            let has_admin = player.permissions.has("ferriscraft.admin");
            let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
            let input = PlayerHudInput {
                rank,
                ferriskey_username: claims.map(|c| c.username),
                ping_ms,
                metrics,
            };
            self.refresh_player(&player, &input);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sidebar_lines_are_all_unique_and_reflect_ferriskey_status() {
        let input = PlayerHudInput {
            rank: Rank::Dev,
            ferriskey_username: Some("nathael".to_owned()),
            ping_ms: 18,
            metrics: HubMetrics {
                online: 12,
                max_players: 100,
                tps_1m: 20.0,
                sheepwars_queued: 3,
                agents_working: 2,
                agents_total: 4,
            },
        };
        let lines = build_sidebar_lines(&input);
        let unique: HashSet<&String> = lines.iter().collect();
        assert_eq!(
            lines.len(),
            unique.len(),
            "each scoreboard entry must be unique"
        );
        assert!(lines.iter().any(|l| l.contains("✔ nathael")));
        assert!(lines.iter().any(|l| l.contains("SheepWars : §d3")));

        let tab = build_tab_list(&input);
        assert!(tab.header.contains("FERRISCRAFT"));
        assert!(tab.footer.contains("✔ nathael"));
    }
}
