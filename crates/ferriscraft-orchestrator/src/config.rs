use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use ferriscraft_protocol::GameKind;
use thiserror::Error;

use crate::engine::FleetPolicy;
use crate::game::{GameSpec, SHEEPWARS, TOWERWARS};
use crate::s3::{S3Config, S3Error};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("adresse d'écoute invalide ({0}) : {1}")]
    InvalidBind(String, #[source] std::net::AddrParseError),
    #[error("mode backend inconnu ({0}) : attendu 'kubernetes' ou 'static'")]
    InvalidBackend(String),
    #[error("entrée statique invalide ({0}) : format attendu 'nom=host:port'")]
    InvalidStaticServer(String),
    #[error("{0}")]
    InvalidS3(#[from] S3Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    Kubernetes,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticServerSpec {
    pub server_id: String,
    pub address: String,
}

#[derive(Debug, Clone)]
pub struct GameConfig {
    pub game: GameKind,
    pub queue_min_players: usize,
    pub max_players_per_server: u32,
    pub min_replicas: usize,
    pub max_replicas: usize,
    pub standby_servers: usize,
    pub idle_ttl: Duration,
    pub starting_timeout: Duration,
    pub empty_grace: Duration,
    pub ending_timeout: Duration,
    pub heartbeat_timeout: Duration,
    pub pod_template_path: Option<PathBuf>,
    pub image: String,
    pub port: u16,
    pub forwarding_secret_name: String,
    pub static_servers: Vec<StaticServerSpec>,
    pub fixed_map: Option<String>,
    pub use_crd: bool,
}

impl GameConfig {
    pub const fn spec(&self) -> &'static GameSpec {
        GameSpec::of(self.game)
    }

    pub fn from_lookup(
        spec: &GameSpec,
        env: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let var = |suffix: &str| env(&spec.env_name(suffix));
        let number = |suffix: &str, default: u64| {
            var(suffix)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default)
        };
        let count = |suffix: &str, default: usize| {
            var(suffix)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(default)
        };

        let min_replicas = count("MIN_REPLICAS", 1);
        let max_replicas = count("MAX_REPLICAS", 5).max(min_replicas).max(1);
        let standby_servers = count("STANDBY_SERVERS", 1).min(max_replicas);
        let max_players_per_server = var("MAX_PLAYERS")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(spec.default_max_players)
            .max(1);
        let queue_min_players = var("QUEUE_MIN_PLAYERS")
            .or_else(|| env("QUEUE_MIN_PLAYERS"))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1)
            .min(max_players_per_server as usize);

        let static_servers = match env(&format!("STATIC_{}_SERVERS", spec.env_prefix)) {
            Some(raw) => parse_static_servers(&raw)?,
            None => Vec::new(),
        };

        Ok(Self {
            game: spec.game,
            queue_min_players,
            max_players_per_server,
            min_replicas,
            max_replicas,
            standby_servers,
            idle_ttl: Duration::from_secs(number("IDLE_TTL_SECS", 180)),
            starting_timeout: Duration::from_secs(number("STARTING_TIMEOUT_SECS", 120)),
            empty_grace: Duration::from_secs(number("EMPTY_GRACE_SECS", 30)),
            ending_timeout: Duration::from_secs(number("ENDING_TIMEOUT_SECS", 60)),
            heartbeat_timeout: Duration::from_secs(number("HEARTBEAT_TIMEOUT_SECS", 30)),
            pod_template_path: var("POD_TEMPLATE_PATH").map(PathBuf::from),
            image: var("IMAGE").unwrap_or_else(|| spec.default_image.to_owned()),
            port: var("PORT")
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(25565),
            forwarding_secret_name: var("FORWARDING_SECRET_NAME")
                .unwrap_or_else(|| "ferriscraft-forwarding".to_owned()),
            static_servers,
            fixed_map: var("MAP"),
            use_crd: var("CRD_ENABLED").is_some_and(|v| is_truthy(&v)),
        })
    }
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub bind_addr: SocketAddr,
    pub backend: BackendMode,
    pub api_token: Option<String>,
    pub lobby_server_name: String,
    pub transfer_timeout: Duration,
    pub reconcile_interval: Duration,
    pub k8s_namespace: String,
    pub orchestrator_public_url: String,
    pub s3: Option<S3Config>,
    pub s3_cache_ttl: Duration,
    pub version: Option<String>,
    pub progression_database_url: Option<String>,
    pub sheepwars: GameConfig,
    pub towerwars: Option<GameConfig>,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn is_truthy(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value == "1"
}

fn detect_namespace(env: &impl Fn(&str) -> Option<String>) -> String {
    if let Some(ns) = env("K8S_NAMESPACE") {
        return ns;
    }
    if let Ok(ns) =
        std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace")
    {
        let trimmed = ns.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    "ferriscraft".to_owned()
}

pub fn parse_static_servers(raw: &str) -> Result<Vec<StaticServerSpec>, ConfigError> {
    let mut specs = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((name, addr)) = item.split_once('=') else {
            return Err(ConfigError::InvalidStaticServer(item.to_owned()));
        };
        let server_id = name.trim();
        let address = addr.trim();
        if server_id.is_empty() || address.is_empty() || !address.contains(':') {
            return Err(ConfigError::InvalidStaticServer(item.to_owned()));
        }
        specs.push(StaticServerSpec {
            server_id: server_id.to_owned(),
            address: address.to_owned(),
        });
    }
    Ok(specs)
}

impl OrchestratorConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(non_empty_env, S3Config::from_env()?)
    }

    pub fn from_lookup(
        env: impl Fn(&str) -> Option<String>,
        s3: Option<S3Config>,
    ) -> Result<Self, ConfigError> {
        let number = |name: &str, default: u64| {
            env(name)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default)
        };
        let raw_bind = env("ORCHESTRATOR_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind_addr: SocketAddr = raw_bind
            .parse()
            .map_err(|err| ConfigError::InvalidBind(raw_bind, err))?;

        let backend = match env("ORCHESTRATOR_BACKEND").as_deref() {
            Some("kubernetes" | "k8s") => BackendMode::Kubernetes,
            Some("static" | "local") => BackendMode::Static,
            Some(other) => return Err(ConfigError::InvalidBackend(other.to_owned())),
            None => {
                if env("KUBERNETES_SERVICE_HOST").is_some() {
                    BackendMode::Kubernetes
                } else {
                    BackendMode::Static
                }
            }
        };

        let towerwars_enabled = env(&TOWERWARS.env_name("ENABLED")).is_some_and(|v| is_truthy(&v));
        let towerwars = if towerwars_enabled {
            Some(GameConfig::from_lookup(&TOWERWARS, &env)?)
        } else {
            None
        };

        Ok(Self {
            bind_addr,
            backend,
            api_token: env("ORCHESTRATOR_TOKEN"),
            lobby_server_name: env("LOBBY_SERVER_NAME").unwrap_or_else(|| "lobby".to_owned()),
            transfer_timeout: Duration::from_secs(number("TRANSFER_TIMEOUT_SECS", 15)),
            reconcile_interval: Duration::from_millis(
                number("RECONCILE_INTERVAL_MS", 1000).max(200),
            ),
            k8s_namespace: detect_namespace(&env),
            orchestrator_public_url: env("ORCHESTRATOR_PUBLIC_URL")
                .unwrap_or_else(|| "http://ferriscraft-orchestrator:8080".to_owned()),
            s3,
            s3_cache_ttl: Duration::from_secs(number("S3_CACHE_TTL_SECS", 30).max(1)),
            version: env("FERRISCRAFT_VERSION"),
            progression_database_url: env("PROGRESSION_DATABASE_URL"),
            sheepwars: GameConfig::from_lookup(&SHEEPWARS, &env)?,
            towerwars,
        })
    }

    pub fn games(&self) -> impl Iterator<Item = &GameConfig> {
        std::iter::once(&self.sheepwars).chain(self.towerwars.as_ref())
    }

    pub fn policy(&self, game: &GameConfig) -> FleetPolicy {
        FleetPolicy {
            game: game.game,
            lobby_server_name: self.lobby_server_name.clone(),
            queue_min_players: game.queue_min_players,
            max_players_per_server: game.max_players_per_server,
            min_replicas: game.min_replicas,
            max_replicas: game.max_replicas,
            standby_servers: game.standby_servers,
            idle_ttl: game.idle_ttl,
            starting_timeout: game.starting_timeout,
            transfer_timeout: self.transfer_timeout,
            empty_grace: game.empty_grace,
            ending_timeout: game.ending_timeout,
            heartbeat_timeout: game.heartbeat_timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(vars: &[(&str, &str)]) -> OrchestratorConfig {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        OrchestratorConfig::from_lookup(|name| vars.get(name).cloned(), None).unwrap()
    }

    #[test]
    fn spec_tw_26_towerwars_is_disabled_by_default() {
        let config = config(&[]);
        assert!(config.towerwars.is_none());
        assert_eq!(config.games().count(), 1);
        assert_eq!(config.sheepwars.game, GameKind::SheepWars);
    }

    #[test]
    fn sheepwars_defaults_are_unchanged() {
        let game = config(&[]).sheepwars;
        assert_eq!(
            (
                game.min_replicas,
                game.max_replicas,
                game.standby_servers,
                game.max_players_per_server,
                game.queue_min_players,
                game.port
            ),
            (1, 5, 1, 16, 1, 25565)
        );
        assert_eq!(
            game.image,
            "ghcr.io/ferrislabs/ferriscraft-sheepwars:latest"
        );
        assert_eq!(game.idle_ttl, Duration::from_secs(180));
        assert_eq!(game.forwarding_secret_name, "ferriscraft-forwarding");
        assert!(!game.use_crd);
        assert_eq!(game.fixed_map, None);
    }

    #[test]
    fn sheepwars_variables_keep_their_names() {
        let game = config(&[
            ("SHEEPWARS_MIN_REPLICAS", "2"),
            ("SHEEPWARS_MAX_REPLICAS", "1"),
            ("SHEEPWARS_STANDBY_SERVERS", "9"),
            ("SHEEPWARS_MAX_PLAYERS", "8"),
            ("QUEUE_MIN_PLAYERS", "20"),
            ("SHEEPWARS_MAP", "steampunk"),
            ("SHEEPWARS_CRD_ENABLED", "true"),
            ("SHEEPWARS_POD_TEMPLATE_PATH", "/etc/sw.yaml"),
            ("STATIC_SHEEPWARS_SERVERS", "sw-1=127.0.0.1:25570"),
        ])
        .sheepwars;
        assert_eq!(game.min_replicas, 2);
        assert_eq!(game.max_replicas, 2);
        assert_eq!(game.standby_servers, 2);
        assert_eq!(game.max_players_per_server, 8);
        assert_eq!(game.queue_min_players, 8);
        assert_eq!(game.fixed_map.as_deref(), Some("steampunk"));
        assert!(game.use_crd);
        assert_eq!(game.pod_template_path, Some(PathBuf::from("/etc/sw.yaml")));
        assert_eq!(game.static_servers.len(), 1);
    }

    #[test]
    fn spec_tw_26_towerwars_enabled_reads_its_own_variables() {
        let config = config(&[
            ("TOWERWARS_ENABLED", "true"),
            ("TOWERWARS_IMAGE", "registry/tw:1"),
            ("TOWERWARS_PORT", "25570"),
            ("TOWERWARS_MIN_REPLICAS", "0"),
            ("TOWERWARS_MAX_REPLICAS", "3"),
            ("TOWERWARS_STANDBY_SERVERS", "2"),
            ("TOWERWARS_MAX_PLAYERS", "12"),
            ("TOWERWARS_POD_TEMPLATE_PATH", "/etc/tw.yaml"),
            ("TOWERWARS_MAP", "test"),
            ("SHEEPWARS_MAX_REPLICAS", "7"),
        ]);
        let tower = config.towerwars.as_ref().unwrap();
        assert_eq!(tower.game, GameKind::TowerWars);
        assert_eq!(tower.image, "registry/tw:1");
        assert_eq!(tower.port, 25570);
        assert_eq!(
            (
                tower.min_replicas,
                tower.max_replicas,
                tower.standby_servers
            ),
            (0, 3, 2)
        );
        assert_eq!(tower.max_players_per_server, 12);
        assert_eq!(tower.pod_template_path, Some(PathBuf::from("/etc/tw.yaml")));
        assert_eq!(tower.fixed_map.as_deref(), Some("test"));
        assert_eq!(config.sheepwars.max_replicas, 7);
        assert_eq!(config.games().count(), 2);
    }

    #[test]
    fn towerwars_variables_are_ignored_while_disabled() {
        let config = config(&[("TOWERWARS_ENABLED", "false"), ("TOWERWARS_IMAGE", "x")]);
        assert!(config.towerwars.is_none());
    }

    #[test]
    fn towerwars_queue_minimum_falls_back_to_the_shared_one() {
        let shared = config(&[("TOWERWARS_ENABLED", "1"), ("QUEUE_MIN_PLAYERS", "3")]);
        assert_eq!(shared.towerwars.as_ref().unwrap().queue_min_players, 3);
        let own = config(&[
            ("TOWERWARS_ENABLED", "1"),
            ("QUEUE_MIN_PLAYERS", "3"),
            ("TOWERWARS_QUEUE_MIN_PLAYERS", "4"),
        ]);
        assert_eq!(own.towerwars.as_ref().unwrap().queue_min_players, 4);
    }

    #[test]
    fn each_game_gets_its_own_policy() {
        let config = config(&[
            ("TOWERWARS_ENABLED", "true"),
            ("TOWERWARS_MAX_REPLICAS", "2"),
            ("LOBBY_SERVER_NAME", "hub"),
        ]);
        let sheep = config.policy(&config.sheepwars);
        let tower = config.policy(config.towerwars.as_ref().unwrap());
        assert_eq!(sheep.game, GameKind::SheepWars);
        assert_eq!(tower.game, GameKind::TowerWars);
        assert_eq!((sheep.max_replicas, tower.max_replicas), (5, 2));
        assert_eq!(tower.lobby_server_name, "hub");
    }

    #[test]
    fn invalid_backend_is_rejected() {
        let vars = HashMap::from([("ORCHESTRATOR_BACKEND".to_owned(), "nomad".to_owned())]);
        let result = OrchestratorConfig::from_lookup(|name| vars.get(name).cloned(), None);
        assert!(matches!(result, Err(ConfigError::InvalidBackend(_))));
    }
}
