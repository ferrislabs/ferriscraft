use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::{GameMode, TICKS_PER_SECOND};
use crate::map::Map;

const DEFAULT_SERVER_ID: &str = "towerwars-1";
const DEFAULT_COUNTDOWN_SECS: u32 = 15;
const DEFAULT_END_DELAY_SECS: u32 = 10;
const DEFAULT_HEARTBEAT_MS: u64 = 2000;
const MIN_HEARTBEAT_MS: u64 = 500;
const DEFAULT_MIN_PLAYERS: usize = 1;
const DEFAULT_MAX_PLAYERS: usize = 6;

#[derive(Debug, Clone)]
pub struct TowerWarsConfig {
    pub server_id: String,
    pub orchestrator_url: Option<String>,
    pub orchestrator_token: Option<String>,
    pub heartbeat_interval: Duration,
    pub ephemeral_pod: bool,
    pub map_path: Option<PathBuf>,
    pub mode: GameMode,
    pub seed: Option<u64>,
    pub min_players: usize,
    pub max_players: usize,
    pub countdown_ticks: u32,
    pub end_delay_ticks: u32,
    pub version: Option<String>,
}

impl Default for TowerWarsConfig {
    fn default() -> Self {
        Self {
            server_id: DEFAULT_SERVER_ID.to_owned(),
            orchestrator_url: None,
            orchestrator_token: None,
            heartbeat_interval: Duration::from_millis(DEFAULT_HEARTBEAT_MS),
            ephemeral_pod: false,
            map_path: None,
            mode: GameMode::Solo,
            seed: None,
            min_players: DEFAULT_MIN_PLAYERS,
            max_players: DEFAULT_MAX_PLAYERS,
            countdown_ticks: DEFAULT_COUNTDOWN_SECS * TICKS_PER_SECOND,
            end_delay_ticks: DEFAULT_END_DELAY_SECS * TICKS_PER_SECOND,
            version: None,
        }
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_mode(raw: &str) -> GameMode {
    match raw.to_ascii_lowercase().as_str() {
        "teams" => GameMode::Teams,
        _ => GameMode::Solo,
    }
}

impl TowerWarsConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| {
            lookup(name)
                .map(|raw| raw.trim().to_owned())
                .filter(|raw| !raw.is_empty())
        };
        let orchestrator_url =
            get("ORCHESTRATOR_URL").map(|url| url.trim_end_matches('/').to_owned());
        let mode = get("TOWERWARS_MODE").map_or(GameMode::Solo, |raw| parse_mode(&raw));
        let min_players = get("TOWERWARS_MIN_PLAYERS")
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MIN_PLAYERS)
            .max(1);
        let max_players = get("TOWERWARS_MAX_PLAYERS")
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_PLAYERS)
            .min(mode.max_players())
            .max(min_players);
        let secs = |name: &str, default: u32| {
            get(name)
                .and_then(|raw| raw.parse::<u32>().ok())
                .unwrap_or(default)
                .saturating_mul(TICKS_PER_SECOND)
        };
        Self {
            version: get("FERRISCRAFT_VERSION"),
            server_id: get("SERVER_ID").unwrap_or_else(|| DEFAULT_SERVER_ID.to_owned()),
            ephemeral_pod: get("TOWERWARS_EPHEMERAL")
                .and_then(|raw| parse_bool(&raw))
                .unwrap_or(orchestrator_url.is_some()),
            orchestrator_token: get("ORCHESTRATOR_TOKEN"),
            heartbeat_interval: Duration::from_millis(
                get("TOWERWARS_HEARTBEAT_INTERVAL_MS")
                    .and_then(|raw| raw.parse::<u64>().ok())
                    .unwrap_or(DEFAULT_HEARTBEAT_MS)
                    .max(MIN_HEARTBEAT_MS),
            ),
            map_path: get("TOWERWARS_MAP_PATH").map(PathBuf::from),
            seed: get("TOWERWARS_SEED").and_then(|raw| raw.parse::<u64>().ok()),
            countdown_ticks: secs("TOWERWARS_COUNTDOWN_SECS", DEFAULT_COUNTDOWN_SECS),
            end_delay_ticks: secs("TOWERWARS_END_DELAY_SECS", DEFAULT_END_DELAY_SECS),
            orchestrator_url,
            mode,
            min_players,
            max_players,
        }
    }
}

pub fn map_from(source: io::Result<String>, path: &Path) -> Map {
    match source {
        Ok(raw) => Map::from_json(&raw).unwrap_or_else(|error| {
            tracing::error!(
                path = %path.display(),
                "Carte invalide, utilisation de la carte de test : {error}"
            );
            Map::test_map()
        }),
        Err(error) => {
            tracing::error!(
                path = %path.display(),
                "Carte illisible, utilisation de la carte de test : {error}"
            );
            Map::test_map()
        }
    }
}

pub fn load_map(path: Option<&Path>) -> Map {
    match path {
        Some(path) => map_from(std::fs::read_to_string(path), path),
        None => Map::test_map(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(vars: &[(&str, &str)]) -> TowerWarsConfig {
        let table: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        TowerWarsConfig::from_lookup(|name| table.get(name).cloned())
    }

    #[test]
    fn defaults_follow_the_contract() {
        let config = config(&[]);
        assert_eq!(config.server_id, "towerwars-1");
        assert_eq!(config.min_players, 1);
        assert_eq!(config.max_players, 6);
        assert_eq!(config.countdown_ticks, 15 * 20);
        assert_eq!(config.end_delay_ticks, 10 * 20);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(2));
        assert_eq!(config.mode, GameMode::Solo);
        assert_eq!(config.seed, None);
        assert!(!config.ephemeral_pod);
        assert!(config.orchestrator_url.is_none());
        assert!(config.map_path.is_none());
    }

    #[test]
    fn an_orchestrator_makes_the_pod_ephemeral_unless_overridden() {
        assert!(config(&[("ORCHESTRATOR_URL", "http://orch/")]).ephemeral_pod);
        assert!(
            !config(&[
                ("ORCHESTRATOR_URL", "http://orch"),
                ("TOWERWARS_EPHEMERAL", "false")
            ])
            .ephemeral_pod
        );
        assert!(config(&[("TOWERWARS_EPHEMERAL", "yes")]).ephemeral_pod);
    }

    #[test]
    fn the_orchestrator_url_loses_its_trailing_slash() {
        let config = config(&[
            ("ORCHESTRATOR_URL", "http://orch:8080/"),
            ("ORCHESTRATOR_TOKEN", "secret"),
            ("SERVER_ID", "tw-7"),
        ]);
        assert_eq!(config.orchestrator_url.as_deref(), Some("http://orch:8080"));
        assert_eq!(config.orchestrator_token.as_deref(), Some("secret"));
        assert_eq!(config.server_id, "tw-7");
    }

    #[test]
    fn modes_and_seed_are_parsed() {
        let teams = config(&[
            ("TOWERWARS_MODE", "Teams"),
            ("TOWERWARS_SEED", "42"),
            ("TOWERWARS_MAX_PLAYERS", "12"),
        ]);
        assert_eq!(teams.mode, GameMode::Teams);
        assert_eq!(teams.seed, Some(42));
        assert_eq!(teams.max_players, 12);
        assert_eq!(config(&[("TOWERWARS_MODE", "nope")]).mode, GameMode::Solo);
    }

    #[test]
    fn player_bounds_stay_within_what_the_mode_allows() {
        let low = config(&[("TOWERWARS_MIN_PLAYERS", "0")]);
        assert_eq!(low.min_players, 1);
        let high = config(&[("TOWERWARS_MAX_PLAYERS", "40")]);
        assert_eq!(high.max_players, 6);
        let inverted = config(&[
            ("TOWERWARS_MIN_PLAYERS", "6"),
            ("TOWERWARS_MAX_PLAYERS", "5"),
        ]);
        assert_eq!(inverted.min_players, 6);
        assert_eq!(inverted.max_players, 6);
    }

    #[test]
    fn heartbeat_is_floored_and_unparsable_values_fall_back() {
        assert_eq!(
            config(&[("TOWERWARS_HEARTBEAT_INTERVAL_MS", "10")]).heartbeat_interval,
            Duration::from_millis(500)
        );
        assert_eq!(
            config(&[("TOWERWARS_COUNTDOWN_SECS", "abc")]).countdown_ticks,
            300
        );
        assert_eq!(
            config(&[("TOWERWARS_COUNTDOWN_SECS", "3")]).countdown_ticks,
            60
        );
    }

    #[test]
    fn a_missing_map_file_gives_the_test_map() {
        let map = map_from(Err(io::ErrorKind::NotFound.into()), Path::new("map.json"));
        assert_eq!(map, Map::test_map());
    }

    #[test]
    fn an_invalid_map_gives_the_test_map() {
        let map = map_from(Ok("{ nope".to_owned()), Path::new("map.json"));
        assert_eq!(map, Map::test_map());
    }

    #[test]
    fn a_valid_map_is_used() {
        let mut custom = Map::test_map();
        custom.lanes[0].spectate_anchor.y += 5.0;
        let loaded = map_from(Ok(custom.to_json()), Path::new("map.json"));
        assert_eq!(loaded, custom);
    }

    #[test]
    fn no_path_gives_the_embedded_test_map() {
        assert_eq!(load_map(None), Map::test_map());
    }

    #[test]
    fn the_exported_map_matches_the_embedded_test_map() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/map.json");
        let raw = std::fs::read_to_string(&path).expect("read examples/map.json");
        assert_eq!(raw.trim_end(), Map::test_map().to_json());
        assert_eq!(Map::from_json(&raw).expect("valid map"), Map::test_map());
    }
}
