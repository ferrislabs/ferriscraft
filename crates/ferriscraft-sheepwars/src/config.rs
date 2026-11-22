use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::arena::ArenaSettings;
use crate::domain::{RespawnRule, RoundRules};

pub const DEFAULT_SETTINGS_FILE: &str = "arena.json";
const TICKS_PER_SECOND: u32 = 20;

#[derive(Debug, Clone)]
pub struct SheepWarsConfig {
    pub server_id: String,
    pub orchestrator_url: Option<String>,
    pub orchestrator_token: Option<String>,
    pub heartbeat_interval: Duration,
    pub ephemeral_pod: bool,
    pub arena_path: PathBuf,
    pub rules: RoundRules,
    pub end_delay_ticks: u32,
}

impl Default for SheepWarsConfig {
    fn default() -> Self {
        Self {
            server_id: "sheepwars-1".to_owned(),
            orchestrator_url: None,
            orchestrator_token: None,
            heartbeat_interval: Duration::from_secs(2),
            ephemeral_pod: false,
            arena_path: PathBuf::from(DEFAULT_SETTINGS_FILE),
            rules: RoundRules::default(),
            end_delay_ticks: 20 * 10,
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|val| !val.is_empty())
}

fn parse_u32_env(name: &str) -> Option<u32> {
    non_empty_env(name).and_then(|val| val.parse::<u32>().ok())
}

fn parse_usize_env(name: &str) -> Option<usize> {
    non_empty_env(name).and_then(|val| val.parse::<usize>().ok())
}

fn parse_bool_env(name: &str) -> Option<bool> {
    non_empty_env(name).and_then(|val| match val.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

impl SheepWarsConfig {
    pub fn from_env() -> Self {
        let server_id = non_empty_env("SERVER_ID").unwrap_or_else(|| "sheepwars-1".to_owned());
        let orchestrator_url =
            non_empty_env("ORCHESTRATOR_URL").map(|url| url.trim_end_matches('/').to_owned());
        let orchestrator_token = non_empty_env("ORCHESTRATOR_TOKEN");

        let heartbeat_ms = non_empty_env("SHEEPWARS_HEARTBEAT_INTERVAL_MS")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2000)
            .max(500);
        let heartbeat_interval = Duration::from_millis(heartbeat_ms);

        let ephemeral_pod =
            parse_bool_env("SHEEPWARS_EPHEMERAL").unwrap_or(orchestrator_url.is_some());

        let arena_path = non_empty_env("SHEEPWARS_ARENA_PATH")
            .map_or_else(|| PathBuf::from(DEFAULT_SETTINGS_FILE), PathBuf::from);

        let min_players = parse_usize_env("SHEEPWARS_MIN_PLAYERS")
            .unwrap_or(2)
            .max(1);
        let max_players = parse_usize_env("SHEEPWARS_MAX_PLAYERS")
            .unwrap_or(16)
            .max(min_players);
        let countdown_secs = parse_u32_env("SHEEPWARS_COUNTDOWN_SECS").unwrap_or(15);
        let sheep_interval_secs = parse_u32_env("SHEEPWARS_SHEEP_INTERVAL_SECS").unwrap_or(12);
        let lives = parse_u32_env("SHEEPWARS_LIVES").unwrap_or(3).max(1);
        let end_delay_secs = parse_u32_env("SHEEPWARS_END_DELAY_SECS").unwrap_or(10);

        Self {
            server_id,
            orchestrator_url,
            orchestrator_token,
            heartbeat_interval,
            ephemeral_pod,
            arena_path,
            rules: RoundRules {
                min_players,
                max_players,
                countdown_ticks: countdown_secs.saturating_mul(TICKS_PER_SECOND),
                sheep_interval_ticks: sheep_interval_secs.saturating_mul(TICKS_PER_SECOND),
                respawn: RespawnRule::Lives(lives),
            },
            end_delay_ticks: end_delay_secs.saturating_mul(TICKS_PER_SECOND),
        }
    }
}

pub fn settings_from(source: io::Result<String>, path: &Path) -> ArenaSettings {
    match source {
        Ok(json) => ArenaSettings::from_json(&json).unwrap_or_else(|error| {
            tracing::error!(
                path = %path.display(),
                "Configuration d'arène invalide, utilisation de l'arène par défaut : {error}"
            );
            ArenaSettings::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => ArenaSettings::default(),
        Err(error) => {
            tracing::error!(
                path = %path.display(),
                "Impossible de lire la configuration d'arène, utilisation de l'arène par défaut : {error}"
            );
            ArenaSettings::default()
        }
    }
}

pub fn load_settings(path: &Path) -> ArenaSettings {
    settings_from(std::fs::read_to_string(path), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_the_default() {
        let settings = settings_from(Err(io::ErrorKind::NotFound.into()), Path::new("arena.json"));
        assert_eq!(settings, ArenaSettings::default());
    }

    #[test]
    fn unreadable_file_gives_the_default() {
        let settings = settings_from(
            Err(io::ErrorKind::PermissionDenied.into()),
            Path::new("arena.json"),
        );
        assert_eq!(settings, ArenaSettings::default());
    }

    #[test]
    fn invalid_json_gives_the_default() {
        let settings = settings_from(Ok("{ nope".to_owned()), Path::new("arena.json"));
        assert_eq!(settings, ArenaSettings::default());
    }

    #[test]
    fn valid_json_is_used() {
        let custom = ArenaSettings {
            world_name: "custom".to_owned(),
            ..ArenaSettings::default()
        };
        let json = serde_json::to_string(&custom).expect("serializes");
        assert_eq!(
            settings_from(Ok(json), Path::new("arena.json")).world_name,
            "custom"
        );
    }

    #[test]
    fn workspace_sheepwars_arena_json_parses() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../server-sheepwars/arena.json");
        let raw = std::fs::read_to_string(&path).expect("read server-sheepwars/arena.json");
        let parsed = ArenaSettings::from_json(&raw).expect("valid prebuilt arena.json");
        assert!(parsed.prebuilt);
        assert_eq!(parsed.world_name, "minecraft:overworld");
    }
}
