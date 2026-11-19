use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerPhase {
    Starting,
    Waiting,
    Countdown,
    InGame,
    Ending,
}

impl ServerPhase {
    pub const fn accepts_players(self) -> bool {
        matches!(self, Self::Waiting | Self::Countdown)
    }

    pub const fn is_standby_capacity(self) -> bool {
        matches!(self, Self::Starting | Self::Waiting)
    }

    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Ending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSummary {
    pub server_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod_name: Option<String>,
    pub address: String,
    pub phase: ServerPhase,
    pub player_count: u32,
    pub max_players: u32,
    pub ready: bool,
    pub heartbeat_age_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub server_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub phase: ServerPhase,
    pub player_count: u32,
    pub max_players: u32,
    #[serde(default)]
    pub player_uuids: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueJoinStatus {
    Queued,
    AlreadyQueued,
    Transferring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinQueueRequest {
    pub player_uuid: Uuid,
    pub player_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinQueueResponse {
    pub status: QueueJoinStatus,
    pub position: usize,
    pub total_queued: usize,
    pub min_players: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_server: Option<String>,
    pub ready_servers: usize,
    pub active_servers: usize,
    pub max_servers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaveReason {
    UserToggle,
    LobbyQuit,
    ProxyDisconnect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveQueueRequest {
    pub player_uuid: Uuid,
    #[serde(default = "default_leave_reason")]
    pub reason: LeaveReason,
}

const fn default_leave_reason() -> LeaveReason {
    LeaveReason::UserToggle
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveQueueResponse {
    pub removed: bool,
    pub total_queued: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerQueueState {
    NotQueued,
    Queued,
    Transferring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerQueueStatus {
    pub state: PlayerQueueState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub total_queued: usize,
    pub min_players: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_server: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPlayerSummary {
    pub player_uuid: Uuid,
    pub player_name: String,
    pub position: usize,
    pub queued_for_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetStatusSnapshot {
    pub queue_len: usize,
    pub min_players: usize,
    pub max_players_per_server: u32,
    pub min_replicas: usize,
    pub max_replicas: usize,
    pub standby_servers: usize,
    pub ready_servers: usize,
    pub in_game_servers: usize,
    pub starting_servers: usize,
    pub queued_players: Vec<QueuedPlayerSummary>,
    pub servers: Vec<ServerSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyServerEntry {
    pub name: String,
    pub address: String,
    pub phase: ServerPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferCommand {
    pub transfer_id: Uuid,
    pub player_uuid: Uuid,
    pub target_server: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyStateSnapshot {
    pub lobby_server_name: String,
    pub servers: Vec<ProxyServerEntry>,
    pub pending_transfers: Vec<TransferCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProxyEvent {
    ServerUpsert {
        name: String,
        address: String,
        phase: ServerPhase,
    },
    ServerRemove {
        name: String,
    },
    TransferPlayer {
        transfer_id: Uuid,
        player_uuid: Uuid,
        target_server: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferAckRequest {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyDisconnectRequest {
    pub player_uuid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendToLobbyRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseUpdateRequest {
    pub phase: ServerPhase,
}
