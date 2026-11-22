use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

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
pub struct OrchestratorConfig {
    pub bind_addr: SocketAddr,
    pub backend: BackendMode,
    pub api_token: Option<String>,
    pub lobby_server_name: String,
    pub queue_min_players: usize,
    pub max_players_per_server: u32,
    pub min_replicas: usize,
    pub max_replicas: usize,
    pub standby_servers: usize,
    pub idle_ttl: Duration,
    pub starting_timeout: Duration,
    pub transfer_timeout: Duration,
    pub empty_grace: Duration,
    pub ending_timeout: Duration,
    pub heartbeat_timeout: Duration,
    pub reconcile_interval: Duration,
    pub k8s_namespace: String,
    pub pod_template_path: Option<PathBuf>,
    pub sheepwars_image: String,
    pub sheepwars_port: u16,
    pub forwarding_secret_name: String,
    pub orchestrator_public_url: String,
    pub static_servers: Vec<StaticServerSpec>,
    pub s3: Option<S3Config>,
    pub s3_cache_ttl: Duration,
    pub fixed_map: Option<String>,
    pub version: Option<String>,
    pub use_crd: bool,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn parse_usize_env(name: &str, default: usize) -> usize {
    non_empty_env(name)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_u64_env(name: &str, default: u64) -> u64 {
    non_empty_env(name)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn detect_namespace() -> String {
    if let Some(ns) = non_empty_env("K8S_NAMESPACE") {
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
        let raw_bind =
            non_empty_env("ORCHESTRATOR_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind_addr: SocketAddr = raw_bind
            .parse()
            .map_err(|err| ConfigError::InvalidBind(raw_bind, err))?;

        let backend = match non_empty_env("ORCHESTRATOR_BACKEND").as_deref() {
            Some("kubernetes" | "k8s") => BackendMode::Kubernetes,
            Some("static" | "local") => BackendMode::Static,
            Some(other) => return Err(ConfigError::InvalidBackend(other.to_owned())),
            None => {
                if non_empty_env("KUBERNETES_SERVICE_HOST").is_some() {
                    BackendMode::Kubernetes
                } else {
                    BackendMode::Static
                }
            }
        };

        let min_replicas = parse_usize_env("SHEEPWARS_MIN_REPLICAS", 1);
        let max_replicas = parse_usize_env("SHEEPWARS_MAX_REPLICAS", 5)
            .max(min_replicas)
            .max(1);
        let standby_servers = parse_usize_env("SHEEPWARS_STANDBY_SERVERS", 1).min(max_replicas);
        let max_players_per_server = non_empty_env("SHEEPWARS_MAX_PLAYERS")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(16)
            .max(1);
        let queue_min_players = parse_usize_env("QUEUE_MIN_PLAYERS", 1)
            .max(1)
            .min(max_players_per_server as usize);

        let static_servers = match non_empty_env("STATIC_SHEEPWARS_SERVERS") {
            Some(raw) => parse_static_servers(&raw)?,
            None => Vec::new(),
        };

        Ok(Self {
            bind_addr,
            backend,
            api_token: non_empty_env("ORCHESTRATOR_TOKEN"),
            lobby_server_name: non_empty_env("LOBBY_SERVER_NAME")
                .unwrap_or_else(|| "lobby".to_owned()),
            queue_min_players,
            max_players_per_server,
            min_replicas,
            max_replicas,
            standby_servers,
            idle_ttl: Duration::from_secs(parse_u64_env("SHEEPWARS_IDLE_TTL_SECS", 180)),
            starting_timeout: Duration::from_secs(parse_u64_env(
                "SHEEPWARS_STARTING_TIMEOUT_SECS",
                120,
            )),
            transfer_timeout: Duration::from_secs(parse_u64_env("TRANSFER_TIMEOUT_SECS", 15)),
            empty_grace: Duration::from_secs(parse_u64_env("SHEEPWARS_EMPTY_GRACE_SECS", 30)),
            ending_timeout: Duration::from_secs(parse_u64_env("SHEEPWARS_ENDING_TIMEOUT_SECS", 60)),
            heartbeat_timeout: Duration::from_secs(parse_u64_env(
                "SHEEPWARS_HEARTBEAT_TIMEOUT_SECS",
                30,
            )),
            reconcile_interval: Duration::from_millis(
                parse_u64_env("RECONCILE_INTERVAL_MS", 1000).max(200),
            ),
            k8s_namespace: detect_namespace(),
            pod_template_path: non_empty_env("SHEEPWARS_POD_TEMPLATE_PATH").map(PathBuf::from),
            sheepwars_image: non_empty_env("SHEEPWARS_IMAGE")
                .unwrap_or_else(|| "ghcr.io/ferrislabs/ferriscraft-sheepwars:latest".to_owned()),
            sheepwars_port: non_empty_env("SHEEPWARS_PORT")
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(25565),
            forwarding_secret_name: non_empty_env("SHEEPWARS_FORWARDING_SECRET_NAME")
                .unwrap_or_else(|| "ferriscraft-forwarding".to_owned()),
            orchestrator_public_url: non_empty_env("ORCHESTRATOR_PUBLIC_URL")
                .unwrap_or_else(|| "http://ferriscraft-orchestrator:8080".to_owned()),
            static_servers,
            s3: S3Config::from_env()?,
            s3_cache_ttl: Duration::from_secs(parse_u64_env("S3_CACHE_TTL_SECS", 30).max(1)),
            fixed_map: non_empty_env("SHEEPWARS_MAP"),
            version: non_empty_env("FERRISCRAFT_VERSION"),
            use_crd: non_empty_env("SHEEPWARS_CRD_ENABLED")
                .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1"),
        })
    }
}
