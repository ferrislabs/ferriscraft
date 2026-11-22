use std::time::Instant;

use ferriscraft_protocol::ProxyEvent;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyState {
    Starting,
    Ready,
    Draining,
}

#[derive(Debug, Clone)]
pub struct LobbyInstance {
    pub id: String,
    pub address: Option<String>,
    pub state: LobbyState,
    pub player_count: u32,
    pub max_players: u32,
    pub(crate) player_uuids: Vec<Uuid>,
    pub(crate) version: Option<String>,
    pub(crate) created_at: Instant,
    pub(crate) last_heartbeat: Instant,
    pub(crate) registered_on_proxy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DrainError {
    #[error("UnknownServer")]
    UnknownServer,
    #[error("NotALobby")]
    NotALobby,
    #[error("NoOtherLobby")]
    NoOtherLobby,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainResult {
    pub remaining_players: u32,
    pub events: Vec<ProxyEvent>,
}
