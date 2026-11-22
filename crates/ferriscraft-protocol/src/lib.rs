use std::collections::BTreeMap;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerKind {
    Lobby,
    #[default]
    SheepWars,
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
    #[serde(default)]
    pub age_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub outdated: bool,
    #[serde(default)]
    pub kind: ServerKind,
    #[serde(default)]
    pub draining: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub server_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub kind: ServerKind,
    pub phase: ServerPhase,
    pub player_count: u32,
    pub max_players: u32,
    #[serde(default)]
    pub player_uuids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectate_server: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator_version: Option<String>,
    #[serde(default)]
    pub lobbies: Vec<ServerSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_version: Option<String>,
}

pub const PROXY_VERSION_HEADER: &str = "x-ferriscraft-version";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainResponse {
    pub remaining_players: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyServerEntry {
    pub name: String,
    pub address: String,
    pub phase: ServerPhase,
    #[serde(default)]
    pub kind: ServerKind,
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
        #[serde(default)]
        kind: ServerKind,
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
pub struct SpectateRequest {
    pub player_uuid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseUpdateRequest {
    pub phase: ServerPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    SheepWars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    Win,
    Loss,
    Draw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRank {
    Admin,
    Mod,
    Dev,
    Vip,
    Member,
    Guest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchParticipant {
    pub uuid: Uuid,
    pub name: String,
    pub outcome: MatchOutcome,
    /// Played until the end of the round or until eliminated.
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchReport {
    pub match_id: Uuid,
    pub game: GameKind,
    pub server_id: String,
    pub forced_start: bool,
    pub duration_secs: u32,
    pub participants: Vec<MatchParticipant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelProgress {
    pub level: u32,
    pub xp_into_level: u64,
    pub xp_for_next: u64,
    pub total_xp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerReward {
    pub uuid: Uuid,
    pub xp: u64,
    pub coins: u64,
    pub game: LevelProgress,
    pub game_level_up: bool,
    pub network: LevelProgress,
    pub network_level_up: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchReportAck {
    pub match_id: Uuid,
    pub counted: bool,
    pub duplicate: bool,
    pub rewards: Vec<PlayerReward>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticSlot {
    LobbyGadget,
    LobbyTrail,
    SheepTrail,
    VictoryEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticId {
    GadgetFeatherDash,
    GadgetFireworkRocket,
    GadgetWindCharge,
    GadgetSheepCannon,
    GadgetVortexWand,
    GadgetEnderGrapple,
    TrailFlame,
    TrailHeart,
    TrailMagic,
    TrailEmerald,
    SheepTrailFlame,
    SheepTrailHearts,
    SheepTrailNotes,
    SheepTrailRainbow,
    VictoryFireworks,
    VictoryLightning,
    /// Added by a newer version: ignored.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameStats {
    pub game: GameKind,
    pub progress: LevelProgress,
    pub played: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub uuid: Uuid,
    pub name: String,
    pub rank: PlayerRank,
    pub coins: u64,
    pub network: LevelProgress,
    pub games: Vec<GameStats>,
    pub unlocked: Vec<CosmeticId>,
    pub equipped: BTreeMap<CosmeticSlot, CosmeticId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerIdentityUpdate {
    pub name: String,
    pub rank: PlayerRank,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseRequest {
    pub cosmetic: CosmeticId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipRequest {
    pub slot: CosmeticSlot,
    pub cosmetic: Option<CosmeticId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminCoinGrant {
    pub amount: u64,
    pub admin: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_cosmetic_from_newer_version_deserializes_as_unknown() {
        let id: CosmeticId = serde_json::from_str("\"pet_dragon\"").unwrap();
        assert_eq!(id, CosmeticId::Unknown);
    }

    #[test]
    fn cosmetic_ids_use_snake_case_on_the_wire() {
        let json = serde_json::to_string(&CosmeticId::SheepTrailHearts).unwrap();
        assert_eq!(json, "\"sheep_trail_hearts\"");
    }

    #[test]
    fn profile_round_trips_with_equipped_map() {
        let profile = PlayerProfile {
            uuid: Uuid::nil(),
            name: "Steve".into(),
            rank: PlayerRank::Vip,
            coins: 120,
            network: LevelProgress {
                level: 1,
                xp_into_level: 150,
                xp_for_next: 1000,
                total_xp: 150,
            },
            games: vec![],
            unlocked: vec![CosmeticId::TrailEmerald],
            equipped: BTreeMap::from([(CosmeticSlot::LobbyTrail, CosmeticId::TrailEmerald)]),
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"lobby_trail\":\"trail_emerald\""), "{json}");
        let back: PlayerProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn heartbeat_from_an_older_server_has_no_version() {
        let json = r#"{"server_id":"sw-1","phase":"waiting","player_count":0,"max_players":16}"#;
        let heartbeat: HeartbeatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(heartbeat.version, None);
    }

    #[test]
    fn server_summary_from_an_older_orchestrator_parses() {
        let json = r#"{"server_id":"sw-1","address":"10.0.0.1:25565","phase":"in_game","player_count":2,"max_players":16,"ready":true,"heartbeat_age_secs":1}"#;
        let summary: ServerSummary = serde_json::from_str(json).unwrap();
        assert_eq!(
            (summary.version, summary.outdated, summary.age_secs),
            (None, false, 0)
        );
    }

    #[test]
    fn spec_plc_6_heartbeat_without_kind_is_sheepwars() {
        let json = r#"{"server_id":"sw-1","phase":"waiting","player_count":0,"max_players":16}"#;
        let heartbeat: HeartbeatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(heartbeat.kind, ServerKind::SheepWars);
    }

    #[test]
    fn lobby_kind_and_upsert_round_trip() {
        let event = ProxyEvent::ServerUpsert {
            name: "ferriscraft-lobby-abc".into(),
            address: "10.0.0.3:25565".into(),
            phase: ServerPhase::Waiting,
            kind: ServerKind::Lobby,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"lobby\""), "{json}");
        let old = r#"{"type":"server_upsert","name":"sw-1","address":"a","phase":"waiting"}"#;
        let parsed: ProxyEvent = serde_json::from_str(old).unwrap();
        assert!(matches!(parsed, ProxyEvent::ServerUpsert { kind: ServerKind::SheepWars, .. }));
    }

    #[test]
    fn match_report_uses_snake_case_enums() {
        let report = MatchReport {
            match_id: Uuid::nil(),
            game: GameKind::SheepWars,
            server_id: "sheepwars-1".into(),
            forced_start: false,
            duration_secs: 300,
            participants: vec![MatchParticipant {
                uuid: Uuid::nil(),
                name: "Alex".into(),
                outcome: MatchOutcome::Draw,
                completed: true,
            }],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"game\":\"sheep_wars\""), "{json}");
        assert!(json.contains("\"outcome\":\"draw\""), "{json}");
    }
}
