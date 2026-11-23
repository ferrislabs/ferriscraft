use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use ferriscraft_protocol::{
    FleetStatusSnapshot, GameKind, HeartbeatRequest, JoinQueueRequest, JoinQueueResponse,
    LeaveQueueResponse, LeaveReason, PlayerQueueState, PlayerQueueStatus, ProxyEvent,
    ProxyServerEntry, ProxyStateSnapshot, QueueJoinStatus, QueuedPlayerSummary, ServerKind,
    ServerPhase, ServerSummary, TransferCommand,
};
use thiserror::Error;
use uuid::Uuid;

use crate::lobby::{DrainError, DrainResult, LobbyInstance, LobbyState};
use crate::queue::{FIRST_SEQ, QueueEntryState, QueueMutation, QueueRecord, normalize_match_size};

const SPECTATE_TTL: Duration = Duration::from_secs(60);
const SEQ_LANE_WIDTH: u64 = 1 << 30;

const fn seq_lane_offset(game: GameKind) -> u64 {
    match game {
        GameKind::SheepWars => 0,
        GameKind::TowerWars => SEQ_LANE_WIDTH,
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("serveur inconnu : {0}")]
    UnknownServer(String),
    #[error("serveur non joignable : {0}")]
    NotJoinable(String),
}

#[derive(Debug, Clone)]
pub struct FleetPolicy {
    pub game: GameKind,
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
}

#[derive(Debug, Clone)]
struct QueuedPlayer {
    uuid: Uuid,
    name: String,
    joined_at: Instant,
    state: QueueEntryState,
    match_size: u8,
}

impl QueuedPlayer {
    fn is_waiting(&self) -> bool {
        self.state == QueueEntryState::Waiting
    }
}

#[derive(Debug, Clone)]
struct ActiveTransfer {
    command: TransferCommand,
    created_at: Instant,
    is_lobby_return: bool,
    acked: bool,
}

#[derive(Debug, Clone)]
struct ManagedServer {
    server_id: String,
    pod_name: Option<String>,
    address: Option<String>,
    phase: ServerPhase,
    player_count: u32,
    max_players: u32,
    tcp_ready: bool,
    has_plugin_heartbeat: bool,
    created_at: Instant,
    last_heartbeat: Option<Instant>,
    last_non_empty_at: Instant,
    phase_since: Instant,
    empty_since: Option<Instant>,
    outdated: bool,
    version: Option<String>,
    registered_on_proxy: bool,
    seen_players: HashSet<Uuid>,
    last_players: Vec<Uuid>,
    arrived: HashMap<Uuid, (u64, String)>,
    expected_players: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionReason {
    RoundEnded,
    EndingTimeout,
    EmptyInGame,
    HeartbeatLost,
    StartingTimeout,
    IdleExcess,
    Outdated,
}

impl DeletionReason {
    pub const fn is_abnormal(self) -> bool {
        matches!(
            self,
            Self::EndingTimeout | Self::EmptyInGame | Self::HeartbeatLost | Self::StartingTimeout
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDeletion {
    pub server_id: String,
    pub reason: DeletionReason,
}

impl ManagedServer {
    fn set_phase(&mut self, now: Instant, phase: ServerPhase) -> bool {
        if self.phase == phase {
            return false;
        }
        self.phase = phase;
        self.phase_since = now;
        true
    }

    fn is_idle_empty(&self) -> bool {
        self.player_count == 0 && matches!(self.phase, ServerPhase::Starting | ServerPhase::Waiting)
    }

    fn heartbeat_lost(&self, now: Instant, timeout: Duration) -> bool {
        self.has_plugin_heartbeat
            && self.phase != ServerPhase::Starting
            && self
                .last_heartbeat
                .is_some_and(|at| now.saturating_duration_since(at) >= timeout)
    }

    fn ready_for_proxy(&self) -> bool {
        self.address.is_some()
            && self.tcp_ready
            && self.phase != ServerPhase::Starting
            && self.phase != ServerPhase::Ending
    }

    fn summary(&self, now: Instant, kind: ServerKind) -> ServerSummary {
        let heartbeat_age_secs = self
            .last_heartbeat
            .map_or_else(
                || now.saturating_duration_since(self.created_at),
                |t| now.saturating_duration_since(t),
            )
            .as_secs();
        ServerSummary {
            server_id: self.server_id.clone(),
            pod_name: self.pod_name.clone(),
            address: self.address.clone().unwrap_or_default(),
            phase: self.phase,
            player_count: self.player_count,
            max_players: self.max_players,
            ready: self.ready_for_proxy(),
            heartbeat_age_secs,
            age_secs: now.saturating_duration_since(self.created_at).as_secs(),
            version: self.version.clone(),
            outdated: self.outdated,
            kind,
            draining: false,
        }
    }
}

impl LobbyInstance {
    fn summary(&self, now: Instant) -> ServerSummary {
        ServerSummary {
            server_id: self.id.clone(),
            pod_name: Some(self.id.clone()),
            address: self.address.clone().unwrap_or_default(),
            phase: ServerPhase::Waiting,
            player_count: self.player_count,
            max_players: self.max_players,
            ready: self.state == LobbyState::Ready,
            heartbeat_age_secs: now.saturating_duration_since(self.last_heartbeat).as_secs(),
            age_secs: now.saturating_duration_since(self.created_at).as_secs(),
            version: self.version.clone(),
            outdated: false,
            kind: ServerKind::Lobby,
            draining: self.state == LobbyState::Draining,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickActions {
    pub proxy_events: Vec<ProxyEvent>,
    pub servers_to_create: usize,
    pub servers_to_delete: Vec<ServerDeletion>,
}

pub struct FleetEngine {
    policy: FleetPolicy,
    servers: BTreeMap<String, ManagedServer>,
    deleting: HashMap<String, Instant>,
    queue: BTreeMap<u64, QueuedPlayer>,
    transfers: HashMap<Uuid, ActiveTransfer>,
    player_transfers: HashMap<Uuid, Uuid>,
    lobbies: BTreeMap<String, LobbyInstance>,
    spectating: HashMap<Uuid, (String, Instant)>,
    mutations: Vec<QueueMutation>,
}

impl FleetEngine {
    pub fn new(policy: FleetPolicy) -> Self {
        Self {
            policy,
            servers: BTreeMap::new(),
            deleting: HashMap::new(),
            queue: BTreeMap::new(),
            transfers: HashMap::new(),
            player_transfers: HashMap::new(),
            lobbies: BTreeMap::new(),
            spectating: HashMap::new(),
            mutations: Vec::new(),
        }
    }

    pub fn policy(&self) -> &FleetPolicy {
        &self.policy
    }

    pub fn take_mutations(&mut self) -> Vec<QueueMutation> {
        std::mem::take(&mut self.mutations)
    }

    pub fn restore_mutations(&mut self, mut unpersisted: Vec<QueueMutation>) {
        unpersisted.append(&mut self.mutations);
        self.mutations = unpersisted;
    }

    pub fn restore_queue(&mut self, now: Instant, records: Vec<QueueRecord>) {
        let wall_now = chrono::Utc::now();
        for record in records
            .into_iter()
            .filter(|record| record.game == self.policy.game)
        {
            let waited = (wall_now - record.joined_at).to_std().unwrap_or_default();
            let joined_at = now.checked_sub(waited).unwrap_or(now);
            if let QueueEntryState::Dispatched {
                server_id,
                transfer_id,
            } = &record.state
            {
                let reason = self.matchmaking_reason();
                self.transfers.insert(
                    *transfer_id,
                    ActiveTransfer {
                        command: TransferCommand {
                            transfer_id: *transfer_id,
                            player_uuid: record.player_uuid,
                            target_server: server_id.clone(),
                            reason,
                        },
                        created_at: now,
                        is_lobby_return: false,
                        acked: false,
                    },
                );
                self.player_transfers
                    .insert(record.player_uuid, *transfer_id);
            }
            self.queue.insert(
                record.seq,
                QueuedPlayer {
                    uuid: record.player_uuid,
                    name: record.name,
                    joined_at,
                    state: record.state,
                    match_size: normalize_match_size(self.policy.game, record.match_size),
                },
            );
        }
    }

    pub fn needs_enqueue(&self, player_uuid: Uuid) -> bool {
        !self.holds_player(player_uuid)
    }

    pub fn holds_player(&self, player_uuid: Uuid) -> bool {
        self.has_matchmaking_transfer(player_uuid) || self.entry_seq(player_uuid).is_some()
    }

    pub fn has_server(&self, server_id: &str) -> bool {
        self.servers.contains_key(server_id)
    }

    pub fn release_player(&mut self, player_uuid: Uuid) {
        self.remove_entry(player_uuid);
        if let Some(old_tid) = self.player_transfers.remove(&player_uuid) {
            self.transfers.remove(&old_tid);
        }
    }

    fn matchmaking_reason(&self) -> String {
        format!("{}_matchmaking", self.policy.game.slug())
    }

    pub fn join_queue(&mut self, now: Instant, req: JoinQueueRequest) -> JoinQueueResponse {
        let seq = self.next_seq();
        let resp = self.join_queue_with_seq(now, req, seq);
        if resp.status == QueueJoinStatus::Queued
            && let Some(entry) = self.queue.get(&seq)
        {
            self.mutations.push(QueueMutation::Insert {
                seq,
                player_uuid: entry.uuid,
                name: entry.name.clone(),
                game: self.policy.game,
                match_size: entry.match_size,
            });
        }
        resp
    }

    pub fn join_queue_with_seq(
        &mut self,
        now: Instant,
        req: JoinQueueRequest,
        seq: u64,
    ) -> JoinQueueResponse {
        let min_players = self.min_players_for(req.player_uuid, req.match_size);
        if let Some(transfer_id) = self.player_transfers.get(&req.player_uuid)
            && let Some(active) = self.transfers.get(transfer_id)
            && !active.is_lobby_return
        {
            return self.build_join_response(
                QueueJoinStatus::Transferring,
                0,
                Some(active.command.target_server.clone()),
                min_players,
            );
        }

        if let Some(pos) = self.queue_position(req.player_uuid) {
            return self.build_join_response(
                QueueJoinStatus::AlreadyQueued,
                pos,
                None,
                min_players,
            );
        }

        let player_uuid = req.player_uuid;
        self.queue.insert(
            seq,
            QueuedPlayer {
                uuid: req.player_uuid,
                name: req.player_name,
                joined_at: now,
                state: QueueEntryState::Waiting,
                match_size: normalize_match_size(self.policy.game, req.match_size),
            },
        );
        let pos = self
            .queue_position(player_uuid)
            .unwrap_or_else(|| self.waiting_len());
        self.build_join_response(QueueJoinStatus::Queued, pos, None, min_players)
    }

    pub fn leave_queue(&mut self, player_uuid: Uuid, reason: LeaveReason) -> LeaveQueueResponse {
        let removable = self
            .queue
            .values()
            .find(|entry| entry.uuid == player_uuid)
            .is_some_and(|entry| entry.is_waiting() || reason == LeaveReason::ProxyDisconnect);
        let removed = removable && self.remove_entry(player_uuid).is_some();

        if reason == LeaveReason::ProxyDisconnect
            && let Some(transfer_id) = self.player_transfers.remove(&player_uuid)
        {
            self.transfers.remove(&transfer_id);
        }

        LeaveQueueResponse {
            removed,
            total_queued: self.waiting_len(),
        }
    }

    pub fn player_status(&self, now: Instant, player_uuid: Uuid) -> PlayerQueueStatus {
        let spectate_server = self
            .spectating
            .get(&player_uuid)
            .filter(|(_, until)| now < *until)
            .map(|(server_id, _)| server_id.clone());

        if let Some(transfer_id) = self.player_transfers.get(&player_uuid)
            && let Some(active) = self.transfers.get(transfer_id)
            && !active.is_lobby_return
        {
            return PlayerQueueStatus {
                state: PlayerQueueState::Transferring,
                position: None,
                total_queued: self.waiting_len(),
                min_players: self.min_players_for(player_uuid, 0),
                target_server: Some(active.command.target_server.clone()),
                spectate_server,
            };
        }

        if let Some(pos) = self.queue_position(player_uuid) {
            PlayerQueueStatus {
                state: PlayerQueueState::Queued,
                position: Some(pos),
                total_queued: self.waiting_len(),
                min_players: self.min_players_for(player_uuid, 0),
                target_server: None,
                spectate_server,
            }
        } else {
            PlayerQueueStatus {
                state: PlayerQueueState::NotQueued,
                position: None,
                total_queued: self.waiting_len(),
                min_players: self.min_players_for(player_uuid, 0),
                target_server: None,
                spectate_server,
            }
        }
    }

    pub fn register_creating_server(
        &mut self,
        now: Instant,
        server_id: String,
        pod_name: Option<String>,
    ) {
        self.deleting.remove(&server_id);
        self.servers
            .entry(server_id.clone())
            .or_insert_with(|| ManagedServer {
                server_id,
                pod_name,
                address: None,
                phase: ServerPhase::Starting,
                player_count: 0,
                max_players: self.policy.max_players_per_server,
                tcp_ready: false,
                has_plugin_heartbeat: false,
                created_at: now,
                last_heartbeat: None,
                last_non_empty_at: now,
                phase_since: now,
                empty_since: None,
                outdated: false,
                version: None,
                registered_on_proxy: false,
                seen_players: HashSet::new(),
                last_players: Vec::new(),
                arrived: HashMap::new(),
                expected_players: None,
            });
    }

    pub fn upsert_infra_server(
        &mut self,
        now: Instant,
        server_id: String,
        pod_name: Option<String>,
        address: Option<String>,
        tcp_ready: bool,
        failed_or_terminating: bool,
    ) -> Vec<ProxyEvent> {
        if failed_or_terminating {
            self.requeue_lost_players(&server_id);
        }
        let kind = self.policy.game.server_kind();
        let mut events = Vec::new();
        let entry = self
            .servers
            .entry(server_id.clone())
            .or_insert_with(|| ManagedServer {
                server_id: server_id.clone(),
                pod_name: pod_name.clone(),
                address: address.clone(),
                phase: ServerPhase::Starting,
                player_count: 0,
                max_players: self.policy.max_players_per_server,
                tcp_ready,
                has_plugin_heartbeat: false,
                created_at: now,
                last_heartbeat: None,
                last_non_empty_at: now,
                phase_since: now,
                empty_since: None,
                outdated: false,
                version: None,
                registered_on_proxy: false,
                seen_players: HashSet::new(),
                last_players: Vec::new(),
                arrived: HashMap::new(),
                expected_players: None,
            });

        if pod_name.is_some() {
            entry.pod_name = pod_name;
        }
        let addr_changed = address.is_some() && entry.address != address;
        if address.is_some() {
            entry.address = address;
        }
        entry.tcp_ready = tcp_ready;

        if failed_or_terminating {
            entry.set_phase(now, ServerPhase::Ending);
        } else if !entry.has_plugin_heartbeat {
            if tcp_ready && entry.phase == ServerPhase::Starting {
                entry.set_phase(now, ServerPhase::Waiting);
                entry.last_heartbeat = Some(now);
            } else if !tcp_ready && entry.phase == ServerPhase::Waiting {
                entry.set_phase(now, ServerPhase::Starting);
            }
        }

        Self::sync_proxy_registration(entry, kind, addr_changed, &mut events);
        events
    }

    pub fn retain_infra_servers(&mut self, live_ids: &HashSet<String>) -> Vec<ProxyEvent> {
        let stale: Vec<String> = self
            .servers
            .keys()
            .filter(|id| !live_ids.contains(*id))
            .cloned()
            .collect();
        let mut events = Vec::new();
        for id in stale {
            if let Some(ev) = self.remove_server(&id) {
                events.push(ev);
            }
        }
        events
    }

    pub fn remove_server(&mut self, server_id: &str) -> Option<ProxyEvent> {
        self.requeue_lost_players(server_id);
        self.deleting.remove(server_id);
        let removed = self.servers.remove(server_id)?;
        if removed.registered_on_proxy {
            Some(ProxyEvent::ServerRemove {
                name: removed.server_id,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn seen_players(&self, server_id: &str) -> HashSet<Uuid> {
        self.servers
            .get(server_id)
            .map(|server| server.seen_players.clone())
            .unwrap_or_default()
    }

    pub fn mark_outdated(&mut self, server_id: &str) {
        if let Some(server) = self.servers.get_mut(server_id) {
            server.outdated = true;
        }
    }

    pub fn mark_deleting(&mut self, now: Instant, server_id: &str) -> Option<ProxyEvent> {
        self.requeue_lost_players(server_id);
        self.deleting.insert(server_id.to_owned(), now);
        let server = self.servers.get_mut(server_id)?;
        server.set_phase(now, ServerPhase::Ending);
        if server.registered_on_proxy {
            server.registered_on_proxy = false;
            Some(ProxyEvent::ServerRemove {
                name: server.server_id.clone(),
            })
        } else {
            None
        }
    }

    /// Servers still known after a delete request older than `retry_after`.
    /// Each returned server gets a fresh request time, so it is retried once per interval.
    pub fn deletions_to_retry(&mut self, now: Instant, retry_after: Duration) -> Vec<String> {
        let due: Vec<String> = self
            .deleting
            .iter()
            .filter(|(id, requested_at)| {
                self.servers.contains_key(*id)
                    && now.saturating_duration_since(**requested_at) >= retry_after
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &due {
            self.deleting.insert(id.clone(), now);
        }
        due
    }

    pub fn apply_heartbeat(&mut self, now: Instant, req: HeartbeatRequest) -> Vec<ProxyEvent> {
        if req.kind == ServerKind::Lobby {
            return self.apply_lobby_heartbeat(now, req);
        }
        let kind = self.policy.game.server_kind();
        let mut events = Vec::new();
        let max_players = req.max_players.max(1);
        let entry = self
            .servers
            .entry(req.server_id.clone())
            .or_insert_with(|| ManagedServer {
                server_id: req.server_id.clone(),
                pod_name: None,
                address: req.address.clone(),
                phase: req.phase,
                player_count: req.player_count,
                max_players,
                tcp_ready: true,
                has_plugin_heartbeat: true,
                created_at: now,
                last_heartbeat: Some(now),
                last_non_empty_at: now,
                phase_since: now,
                empty_since: None,
                outdated: false,
                version: None,
                registered_on_proxy: false,
                seen_players: HashSet::new(),
                last_players: Vec::new(),
                arrived: HashMap::new(),
                expected_players: None,
            });

        let addr_changed = req.address.is_some() && entry.address != req.address;
        if let Some(addr) = req.address {
            entry.address = Some(addr);
        }
        let phase_changed = entry.set_phase(now, req.phase);
        entry.player_count = req.player_count;
        if req.version.is_some() {
            entry.version = req.version;
        }
        if req.player_count == 0 {
            entry.empty_since.get_or_insert(now);
        } else {
            entry.empty_since = None;
        }
        entry.max_players = max_players;
        entry.tcp_ready = true;
        entry.has_plugin_heartbeat = true;
        entry.last_heartbeat = Some(now);
        if req.player_count > 0 || req.phase != ServerPhase::Waiting {
            entry.last_non_empty_at = now;
        }

        entry.seen_players.extend(req.player_uuids.iter().copied());
        let mut arrivals = Vec::new();
        for uuid in &req.player_uuids {
            if let Some(player) = self.remove_entry_by_uuid(*uuid) {
                arrivals.push((*uuid, player));
            }
            if let Some(tid) = self.player_transfers.get(uuid).copied()
                && self
                    .transfers
                    .get(&tid)
                    .is_some_and(|t| t.command.target_server == req.server_id)
            {
                self.player_transfers.remove(uuid);
                self.transfers.remove(&tid);
            }
        }

        let pending = self
            .pending_transfer_counts()
            .get(&req.server_id)
            .copied()
            .unwrap_or(0);
        if let Some(server) = self.servers.get_mut(&req.server_id) {
            if req.phase == ServerPhase::Waiting && req.player_count == 0 && pending == 0 {
                server.expected_players = None;
            }
            for (uuid, (seq, player)) in arrivals {
                server.arrived.insert(uuid, (seq, player.name));
            }
            server.last_players = req.player_uuids;
            Self::sync_proxy_registration(server, kind, addr_changed || phase_changed, &mut events);
        }
        events
    }

    fn apply_lobby_heartbeat(&mut self, now: Instant, req: HeartbeatRequest) -> Vec<ProxyEvent> {
        let max_players = req.max_players.max(1);
        let lobby = self
            .lobbies
            .entry(req.server_id.clone())
            .or_insert_with(|| LobbyInstance {
                id: req.server_id.clone(),
                address: None,
                state: LobbyState::Starting,
                player_count: 0,
                max_players,
                player_uuids: Vec::new(),
                version: None,
                created_at: now,
                last_heartbeat: now,
                registered_on_proxy: false,
            });
        let mut changed = false;
        if let Some(address) = req.address
            && lobby.address.as_deref() != Some(address.as_str())
        {
            lobby.address = Some(address);
            changed = true;
        }
        if req.phase == ServerPhase::Ending && lobby.state != LobbyState::Draining {
            lobby.state = LobbyState::Draining;
            changed = true;
        }
        if lobby.state == LobbyState::Starting && lobby.address.is_some() {
            lobby.state = LobbyState::Ready;
            changed = true;
        }
        lobby.player_count = req.player_count;
        lobby.max_players = max_players;
        if req.version.is_some() {
            lobby.version = req.version;
        }
        lobby.last_heartbeat = now;

        let mut events = Vec::new();
        match (&lobby.address, lobby.state) {
            (Some(address), LobbyState::Ready) if !lobby.registered_on_proxy || changed => {
                lobby.registered_on_proxy = true;
                events.push(ProxyEvent::ServerUpsert {
                    name: lobby.id.clone(),
                    address: address.clone(),
                    phase: ServerPhase::Waiting,
                    kind: ServerKind::Lobby,
                });
            }
            (_, LobbyState::Ready) => {}
            _ if lobby.registered_on_proxy => {
                lobby.registered_on_proxy = false;
                events.push(ProxyEvent::ServerRemove {
                    name: lobby.id.clone(),
                });
            }
            _ => {}
        }

        for uuid in &req.player_uuids {
            if let Some(tid) = self.player_transfers.get(uuid).copied()
                && self
                    .transfers
                    .get(&tid)
                    .is_some_and(|t| t.is_lobby_return && t.command.target_server == req.server_id)
            {
                self.player_transfers.remove(uuid);
                self.transfers.remove(&tid);
            }
        }
        if let Some(lobby) = self.lobbies.get_mut(&req.server_id) {
            lobby.player_uuids = req.player_uuids;
        }
        events
    }

    pub fn pick_lobby(&self) -> Option<&LobbyInstance> {
        self.least_loaded_lobby(None, &HashMap::new())
    }

    fn least_loaded_lobby(
        &self,
        exclude: Option<&str>,
        extra: &HashMap<String, u32>,
    ) -> Option<&LobbyInstance> {
        let mut pending: HashMap<&str, u32> = HashMap::new();
        for transfer in self.transfers.values().filter(|t| t.is_lobby_return) {
            *pending
                .entry(transfer.command.target_server.as_str())
                .or_insert(0) += 1;
        }
        self.lobbies
            .values()
            .filter(|l| l.state == LobbyState::Ready && Some(l.id.as_str()) != exclude)
            .min_by_key(|l| {
                let load = l.player_count
                    + pending.get(l.id.as_str()).copied().unwrap_or(0)
                    + extra.get(l.id.as_str()).copied().unwrap_or(0);
                (load, l.id.as_str())
            })
    }

    fn lobby_target(&self) -> String {
        self.pick_lobby().map_or_else(
            || self.policy.lobby_server_name.clone(),
            |lobby| lobby.id.clone(),
        )
    }

    pub fn drain_lobby(&mut self, now: Instant, id: &str) -> Result<DrainResult, DrainError> {
        let Some(lobby) = self.lobbies.get(id) else {
            return Err(if self.servers.contains_key(id) {
                DrainError::NotALobby
            } else {
                DrainError::UnknownServer
            });
        };
        let remaining_players = lobby
            .player_count
            .max(u32::try_from(lobby.player_uuids.len()).unwrap_or(u32::MAX));
        let players = lobby.player_uuids.clone();
        if remaining_players > 0 && self.least_loaded_lobby(Some(id), &HashMap::new()).is_none() {
            return Err(DrainError::NoOtherLobby);
        }

        let mut events = Vec::new();
        if let Some(lobby) = self.lobbies.get_mut(id) {
            lobby.state = LobbyState::Draining;
            if lobby.registered_on_proxy {
                lobby.registered_on_proxy = false;
                events.push(ProxyEvent::ServerRemove {
                    name: lobby.id.clone(),
                });
            }
        }

        let mut assigned: HashMap<String, u32> = HashMap::new();
        for player_uuid in players {
            if self.player_transfers.contains_key(&player_uuid) {
                continue;
            }
            let Some(target) = self
                .least_loaded_lobby(Some(id), &assigned)
                .map(|l| l.id.clone())
            else {
                break;
            };
            *assigned.entry(target.clone()).or_insert(0) += 1;
            events.push(self.new_lobby_transfer(
                now,
                player_uuid,
                target,
                "mise à jour du lobby".to_owned(),
            ));
        }
        Ok(DrainResult {
            remaining_players,
            events,
        })
    }

    fn new_lobby_transfer(
        &mut self,
        now: Instant,
        player_uuid: Uuid,
        target_server: String,
        reason: String,
    ) -> ProxyEvent {
        let transfer_id = Uuid::new_v4();
        let command = TransferCommand {
            transfer_id,
            player_uuid,
            target_server: target_server.clone(),
            reason: reason.clone(),
        };
        self.transfers.insert(
            transfer_id,
            ActiveTransfer {
                command,
                created_at: now,
                is_lobby_return: true,
                acked: false,
            },
        );
        self.player_transfers.insert(player_uuid, transfer_id);
        ProxyEvent::TransferPlayer {
            transfer_id,
            player_uuid,
            target_server,
            reason,
        }
    }

    fn requeue_lost_players(&mut self, server_id: &str) {
        let Some(server) = self.servers.get_mut(server_id) else {
            return;
        };
        if !matches!(server.phase, ServerPhase::Waiting | ServerPhase::Countdown) {
            return;
        }
        let joined_at = server.last_heartbeat.unwrap_or(server.created_at);
        let match_size = normalize_match_size(
            self.policy.game,
            server
                .expected_players
                .take()
                .map_or(0, |size| u8::try_from(size).unwrap_or(0)),
        );
        let mut players: Vec<(u64, Uuid, String)> = std::mem::take(&mut server.last_players)
            .into_iter()
            .map(|uuid| match server.arrived.get(&uuid) {
                Some((seq, name)) => (*seq, uuid, name.clone()),
                None => (u64::MAX, uuid, uuid.to_string()),
            })
            .collect();
        players.retain(|(_, uuid, _)| self.entry_seq(*uuid).is_none());
        if players.is_empty() {
            return;
        }
        players.sort_by_key(|(seq, _, _)| *seq);
        let lane_origin = FIRST_SEQ - seq_lane_offset(self.policy.game);
        let base = self
            .queue
            .keys()
            .next()
            .copied()
            .map_or(lane_origin, |head| head.min(lane_origin))
            .saturating_sub(players.len() as u64);
        for (offset, (_, uuid, name)) in players.into_iter().enumerate() {
            let seq = base + offset as u64;
            self.mutations.push(QueueMutation::Insert {
                seq,
                player_uuid: uuid,
                name: name.clone(),
                game: self.policy.game,
                match_size,
            });
            self.queue.insert(
                seq,
                QueuedPlayer {
                    uuid,
                    name,
                    joined_at,
                    state: QueueEntryState::Waiting,
                    match_size,
                },
            );
        }
    }

    pub fn update_server_phase(
        &mut self,
        now: Instant,
        server_id: &str,
        phase: ServerPhase,
    ) -> Result<Vec<ProxyEvent>, EngineError> {
        let kind = self.policy.game.server_kind();
        let Some(server) = self.servers.get_mut(server_id) else {
            return Err(EngineError::UnknownServer(server_id.to_owned()));
        };
        let mut events = Vec::new();
        let changed = server.set_phase(now, phase);
        server.has_plugin_heartbeat = true;
        server.last_heartbeat = Some(now);
        Self::sync_proxy_registration(server, kind, changed, &mut events);
        Ok(events)
    }

    pub fn queue_lobby_transfer(
        &mut self,
        now: Instant,
        player_uuid: Uuid,
        reason: Option<String>,
    ) -> ProxyEvent {
        self.release_player(player_uuid);

        let target_server = self.lobby_target();
        let reason = reason.unwrap_or_else(|| "retour au hub".to_owned());
        self.new_lobby_transfer(now, player_uuid, target_server, reason)
    }

    pub fn queue_spectate_transfer(
        &mut self,
        now: Instant,
        server_id: &str,
        player_uuid: Uuid,
    ) -> Result<ProxyEvent, EngineError> {
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| EngineError::UnknownServer(server_id.to_owned()))?;
        if self.deleting.contains_key(server_id) || !server.ready_for_proxy() {
            return Err(EngineError::NotJoinable(server_id.to_owned()));
        }
        self.release_player(player_uuid);
        self.spectating
            .insert(player_uuid, (server_id.to_owned(), now + SPECTATE_TTL));
        let transfer_id = Uuid::new_v4();
        let reason = "spectate".to_owned();
        let command = TransferCommand {
            transfer_id,
            player_uuid,
            target_server: server_id.to_owned(),
            reason: reason.clone(),
        };
        self.transfers.insert(
            transfer_id,
            ActiveTransfer {
                command,
                created_at: now,
                is_lobby_return: false,
                acked: false,
            },
        );
        self.player_transfers.insert(player_uuid, transfer_id);
        Ok(ProxyEvent::TransferPlayer {
            transfer_id,
            player_uuid,
            target_server: server_id.to_owned(),
            reason,
        })
    }

    pub fn ack_transfer(&mut self, now: Instant, transfer_id: Uuid, success: bool) {
        let Some(active) = self.transfers.get_mut(&transfer_id) else {
            return;
        };
        if success && !active.is_lobby_return {
            active.created_at = now;
            active.acked = true;
            return;
        }
        let is_lobby_return = active.is_lobby_return;
        let player_uuid = active.command.player_uuid;
        self.transfers.remove(&transfer_id);
        if self.player_transfers.get(&player_uuid) == Some(&transfer_id) {
            self.player_transfers.remove(&player_uuid);
        }
        if !is_lobby_return {
            self.requeue_dispatched(player_uuid, transfer_id);
        }
    }

    fn requeue_dispatched(&mut self, player_uuid: Uuid, transfer_id: Uuid) {
        let Some(entry) = self.queue.values_mut().find(|entry| {
            entry.uuid == player_uuid
                && matches!(
                    &entry.state,
                    QueueEntryState::Dispatched { transfer_id: tid, .. } if *tid == transfer_id
                )
        }) else {
            return;
        };
        entry.state = QueueEntryState::Waiting;
        self.mutations.push(QueueMutation::SetState {
            player_uuid,
            state: QueueEntryState::Waiting,
        });
    }

    pub fn tick(&mut self, now: Instant) -> TickActions {
        let mut actions = TickActions {
            proxy_events: self.dispatch(now),
            ..TickActions::default()
        };

        let (to_create, to_delete) = self.plan_fleet_actions(now);
        actions.servers_to_create = to_create;
        for deletion in to_delete {
            if let Some(ev) = self.mark_deleting(now, &deletion.server_id) {
                actions.proxy_events.push(ev);
            }
            actions.servers_to_delete.push(deletion);
        }

        actions
    }

    /// Expires transfers and dispatches queued players without planning the fleet.
    /// Only the reconciler executes deletions, so only it may call [`Self::tick`].
    pub fn dispatch(&mut self, now: Instant) -> Vec<ProxyEvent> {
        let mut events = Vec::new();

        // 1. Expire timed-out transfers so stuck entries never block players indefinitely.
        let expired: Vec<(Uuid, Uuid)> = self
            .transfers
            .iter()
            .filter(|(_, t)| {
                now.saturating_duration_since(t.created_at) >= self.policy.transfer_timeout
            })
            .map(|(tid, t)| (*tid, t.command.player_uuid))
            .collect();
        for (tid, uuid) in expired {
            self.transfers.remove(&tid);
            if self.player_transfers.get(&uuid) == Some(&tid) {
                self.player_transfers.remove(&uuid);
            }
            self.requeue_dispatched(uuid, tid);
        }

        self.spectating.retain(|_, (_, until)| now < *until);

        let lost: Vec<String> = self
            .lobbies
            .values()
            .filter(|l| {
                now.saturating_duration_since(l.last_heartbeat) >= self.policy.heartbeat_timeout
            })
            .map(|l| l.id.clone())
            .collect();
        for id in lost {
            if let Some(lobby) = self.lobbies.remove(&id)
                && lobby.registered_on_proxy
            {
                events.push(ProxyEvent::ServerRemove { name: lobby.id });
            }
        }

        events.extend(self.dispatch_queued_players(now));

        events
    }

    fn dispatch_queued_players(&mut self, now: Instant) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        if self.waiting_len() == 0 {
            return events;
        }

        if self.policy.game == GameKind::TowerWars {
            return self.dispatch_match_groups(now);
        }

        loop {
            if self.waiting_len() == 0 {
                break;
            }

            let pending_by_server = self.pending_transfer_counts();

            // Pick a joinable server: prefer already-populated Waiting/Countdown servers first
            // to fill matches quickly without splitting players across empty servers.
            let mut candidates: Vec<(bool, bool, u32, u32, String)> = self
                .servers
                .values()
                .filter(|s| {
                    !self.deleting.contains_key(&s.server_id)
                        && s.ready_for_proxy()
                        && s.phase.accepts_players()
                })
                .filter_map(|s| {
                    let pending = pending_by_server.get(&s.server_id).copied().unwrap_or(0);
                    let occupied = s.player_count.saturating_add(pending);
                    let free = s.max_players.saturating_sub(occupied);
                    if free == 0 {
                        None
                    } else {
                        Some((
                            !s.outdated,
                            occupied > 0,
                            occupied,
                            free,
                            s.server_id.clone(),
                        ))
                    }
                })
                .collect();

            if candidates.is_empty() {
                break;
            }

            candidates.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| a.4.cmp(&b.4))
            });

            let (_, has_players, _, free_slots, target_server) = candidates.remove(0);

            // Only open an empty server once queue meets queue_min_players.
            // If a server already has players waiting inside, backfill immediately!
            if !has_players && self.waiting_len() < self.policy.queue_min_players {
                break;
            }

            let dispatch_count = self.waiting_len().min(free_slots as usize);
            if dispatch_count == 0 {
                break;
            }

            let seqs: Vec<u64> = self
                .queue
                .iter()
                .filter(|(_, entry)| entry.is_waiting())
                .take(dispatch_count)
                .map(|(seq, _)| *seq)
                .collect();
            for seq in seqs {
                self.dispatch_entry(now, seq, &target_server, &mut events);
            }
        }

        events
    }

    fn dispatch_entry(
        &mut self,
        now: Instant,
        seq: u64,
        target_server: &str,
        events: &mut Vec<ProxyEvent>,
    ) {
        let Some(player) = self.queue.get_mut(&seq) else {
            return;
        };
        let player_uuid = player.uuid;
        let transfer_id = Uuid::new_v4();
        player.state = QueueEntryState::Dispatched {
            server_id: target_server.to_owned(),
            transfer_id,
        };
        self.mutations.push(QueueMutation::SetState {
            player_uuid,
            state: QueueEntryState::Dispatched {
                server_id: target_server.to_owned(),
                transfer_id,
            },
        });
        if let Some(old_tid) = self.player_transfers.remove(&player_uuid) {
            self.transfers.remove(&old_tid);
        }
        let reason = self.matchmaking_reason();
        let command = TransferCommand {
            transfer_id,
            player_uuid,
            target_server: target_server.to_owned(),
            reason: reason.clone(),
        };
        self.transfers.insert(
            transfer_id,
            ActiveTransfer {
                command,
                created_at: now,
                is_lobby_return: false,
                acked: false,
            },
        );
        self.player_transfers.insert(player_uuid, transfer_id);
        events.push(ProxyEvent::TransferPlayer {
            transfer_id,
            player_uuid,
            target_server: target_server.to_owned(),
            reason,
        });
    }

    fn waiting_by_match_size(&self) -> BTreeMap<u8, Vec<u64>> {
        let mut groups: BTreeMap<u8, Vec<u64>> = BTreeMap::new();
        for (seq, entry) in &self.queue {
            if entry.is_waiting() {
                groups
                    .entry(entry.match_size.max(1))
                    .or_default()
                    .push(*seq);
            }
        }
        groups
    }

    fn dispatch_match_groups(&mut self, now: Instant) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        while let Some((size, seqs)) = self
            .waiting_by_match_size()
            .into_iter()
            .filter(|(size, seqs)| seqs.len() >= usize::from(*size))
            .min_by_key(|(_, seqs)| seqs[0])
        {
            let pending_by_server = self.pending_transfer_counts();
            let Some(target_server) = self
                .servers
                .values()
                .filter(|s| {
                    !self.deleting.contains_key(&s.server_id)
                        && s.ready_for_proxy()
                        && s.phase.accepts_players()
                        && s.player_count == 0
                        && s.expected_players.is_none()
                        && pending_by_server.get(&s.server_id).copied().unwrap_or(0) == 0
                })
                .min_by(|a, b| {
                    a.outdated
                        .cmp(&b.outdated)
                        .then_with(|| a.server_id.cmp(&b.server_id))
                })
                .map(|s| s.server_id.clone())
            else {
                break;
            };

            for seq in seqs.into_iter().take(usize::from(size)) {
                self.dispatch_entry(now, seq, &target_server, &mut events);
            }
            if let Some(server) = self.servers.get_mut(&target_server) {
                server.expected_players = Some(u32::from(size));
            }
        }
        events
    }

    fn reap_reason(&self, now: Instant, s: &ManagedServer, pending: u32) -> Option<DeletionReason> {
        let in_phase = now.saturating_duration_since(s.phase_since);
        let empty_for = s
            .empty_since
            .map(|since| now.saturating_duration_since(since));
        match s.phase {
            ServerPhase::Ending if s.player_count == 0 && pending == 0 => {
                Some(DeletionReason::RoundEnded)
            }
            ServerPhase::Ending if in_phase >= self.policy.ending_timeout => {
                Some(DeletionReason::EndingTimeout)
            }
            ServerPhase::Starting
                if now.saturating_duration_since(s.created_at) >= self.policy.starting_timeout =>
            {
                Some(DeletionReason::StartingTimeout)
            }
            _ if s.heartbeat_lost(now, self.policy.heartbeat_timeout) => {
                Some(DeletionReason::HeartbeatLost)
            }
            ServerPhase::InGame
                if pending == 0
                    && empty_for.is_some_and(|empty| empty >= self.policy.empty_grace) =>
            {
                Some(DeletionReason::EmptyInGame)
            }
            _ => None,
        }
    }

    fn plan_fleet_actions(&self, now: Instant) -> (usize, Vec<ServerDeletion>) {
        let mut to_delete = Vec::new();
        let pending_by_server = self.pending_transfer_counts();

        let replacement_ready = self.servers.values().any(|s| {
            !s.outdated
                && !self.deleting.contains_key(&s.server_id)
                && s.phase == ServerPhase::Waiting
                && s.ready_for_proxy()
        });
        let mut retiring = HashSet::new();

        for s in self.servers.values() {
            if self.deleting.contains_key(&s.server_id) {
                continue;
            }
            let pending = pending_by_server.get(&s.server_id).copied().unwrap_or(0);
            if let Some(reason) = self.reap_reason(now, s, pending) {
                to_delete.push(ServerDeletion {
                    server_id: s.server_id.clone(),
                    reason,
                });
            } else if s.outdated && s.is_idle_empty() && pending == 0 {
                if replacement_ready {
                    to_delete.push(ServerDeletion {
                        server_id: s.server_id.clone(),
                        reason: DeletionReason::Outdated,
                    });
                } else {
                    retiring.insert(s.server_id.as_str());
                }
            }
        }

        let delete_set: HashSet<&str> = to_delete.iter().map(|d| d.server_id.as_str()).collect();

        let active_total = self
            .servers
            .values()
            .filter(|s| {
                !self.deleting.contains_key(&s.server_id)
                    && !delete_set.contains(s.server_id.as_str())
                    && !retiring.contains(s.server_id.as_str())
                    && s.phase.is_active()
            })
            .count();

        let available_standby = self
            .servers
            .values()
            .filter(|s| {
                !self.deleting.contains_key(&s.server_id)
                    && !delete_set.contains(s.server_id.as_str())
                    && !s.outdated
                    && s.phase.is_standby_capacity()
            })
            .count();

        let needed_for_queue = if self.waiting_len() == 0 {
            0
        } else if self.policy.game == GameKind::TowerWars {
            self.waiting_by_match_size()
                .iter()
                .map(|(size, seqs)| seqs.len().div_ceil(usize::from(*size)))
                .sum::<usize>()
                .max(1)
        } else {
            self.waiting_len()
                .div_ceil(self.policy.max_players_per_server as usize)
                .max(1)
        };

        let target_available = if self.waiting_len() == 0 {
            // Maintain at least min_replicas total servers when idle; keep standby if min_replicas > 0.
            self.policy
                .min_replicas
                .saturating_sub(active_total.saturating_sub(available_standby))
        } else {
            self.policy.standby_servers.max(needed_for_queue).max(
                self.policy
                    .min_replicas
                    .saturating_sub(active_total.saturating_sub(available_standby)),
            )
        };

        let headroom = self.policy.max_replicas.saturating_sub(active_total);
        let to_create = target_available
            .saturating_sub(available_standby)
            .min(headroom);

        // Scale down excess idle empty Waiting servers above both min_replicas and target_available.
        if to_create == 0 && active_total > self.policy.min_replicas {
            let mut excess_candidates: Vec<&ManagedServer> = self
                .servers
                .values()
                .filter(|s| {
                    !self.deleting.contains_key(&s.server_id)
                        && !delete_set.contains(s.server_id.as_str())
                        && !s.outdated
                        && s.phase == ServerPhase::Waiting
                        && s.player_count == 0
                        && pending_by_server.get(&s.server_id).copied().unwrap_or(0) == 0
                        && now.saturating_duration_since(s.last_non_empty_at)
                            >= self.policy.idle_ttl
                })
                .collect();

            // Keep oldest warm servers, scale down newest excess ones.
            excess_candidates.sort_by_key(|a| std::cmp::Reverse(a.created_at));

            let mut remaining_active = active_total;
            let mut remaining_available = available_standby;
            for cand in excess_candidates {
                if remaining_active <= self.policy.min_replicas
                    || remaining_available <= target_available
                {
                    break;
                }
                to_delete.push(ServerDeletion {
                    server_id: cand.server_id.clone(),
                    reason: DeletionReason::IdleExcess,
                });
                remaining_active -= 1;
                remaining_available -= 1;
            }
        }

        (to_create, to_delete)
    }

    pub fn snapshot(&self, now: Instant) -> FleetStatusSnapshot {
        let mut ready_servers = 0usize;
        let mut in_game_servers = 0usize;
        let mut starting_servers = 0usize;
        let mut servers = Vec::with_capacity(self.servers.len());

        for s in self.servers.values() {
            if self.deleting.contains_key(&s.server_id) {
                continue;
            }
            match s.phase {
                ServerPhase::Starting => starting_servers += 1,
                ServerPhase::Waiting | ServerPhase::Countdown => {
                    if s.ready_for_proxy() {
                        ready_servers += 1;
                    } else {
                        starting_servers += 1;
                    }
                }
                ServerPhase::InGame => in_game_servers += 1,
                ServerPhase::Ending => {}
            }
            servers.push(s.summary(now, self.policy.game.server_kind()));
        }

        let queued_players = self
            .queue
            .values()
            .filter(|q| q.is_waiting())
            .enumerate()
            .map(|(idx, q)| QueuedPlayerSummary {
                player_uuid: q.uuid,
                player_name: q.name.clone(),
                position: idx + 1,
                queued_for_secs: now.saturating_duration_since(q.joined_at).as_secs(),
            })
            .collect();

        FleetStatusSnapshot {
            queue_len: self.waiting_len(),
            min_players: self.policy.queue_min_players,
            max_players_per_server: self.policy.max_players_per_server,
            min_replicas: self.policy.min_replicas,
            max_replicas: self.policy.max_replicas,
            standby_servers: self.policy.standby_servers,
            ready_servers,
            in_game_servers,
            starting_servers,
            queued_players,
            servers,
            orchestrator_version: None,
            lobbies: self.lobbies.values().map(|l| l.summary(now)).collect(),
            proxy_version: None,
            games: Vec::new(),
        }
    }

    pub fn proxy_snapshot(&self) -> ProxyStateSnapshot {
        let servers = self
            .servers
            .values()
            .filter(|s| !self.deleting.contains_key(&s.server_id) && s.ready_for_proxy())
            .filter_map(|s| {
                s.address.as_ref().map(|addr| ProxyServerEntry {
                    name: s.server_id.clone(),
                    address: addr.clone(),
                    phase: s.phase,
                    kind: self.policy.game.server_kind(),
                })
            })
            .chain(
                self.lobbies
                    .values()
                    .filter(|l| l.state == LobbyState::Ready)
                    .filter_map(|l| {
                        l.address.as_ref().map(|addr| ProxyServerEntry {
                            name: l.id.clone(),
                            address: addr.clone(),
                            phase: ServerPhase::Waiting,
                            kind: ServerKind::Lobby,
                        })
                    }),
            )
            .collect();

        let pending_transfers = self
            .transfers
            .values()
            .filter(|t| !t.acked)
            .map(|t| t.command.clone())
            .collect();

        ProxyStateSnapshot {
            lobby_server_name: self.policy.lobby_server_name.clone(),
            servers,
            pending_transfers,
        }
    }

    fn sync_proxy_registration(
        server: &mut ManagedServer,
        kind: ServerKind,
        force_upsert: bool,
        events: &mut Vec<ProxyEvent>,
    ) {
        let should_register = server.ready_for_proxy();
        if should_register && (!server.registered_on_proxy || force_upsert) {
            if let Some(addr) = &server.address {
                server.registered_on_proxy = true;
                events.push(ProxyEvent::ServerUpsert {
                    name: server.server_id.clone(),
                    address: addr.clone(),
                    phase: server.phase,
                    kind,
                });
            }
        } else if !should_register && server.registered_on_proxy {
            server.registered_on_proxy = false;
            events.push(ProxyEvent::ServerRemove {
                name: server.server_id.clone(),
            });
        }
    }

    fn queue_position(&self, uuid: Uuid) -> Option<usize> {
        self.queue
            .values()
            .filter(|entry| entry.is_waiting())
            .position(|entry| entry.uuid == uuid)
            .map(|idx| idx + 1)
    }

    fn waiting_len(&self) -> usize {
        self.queue
            .values()
            .filter(|entry| entry.is_waiting())
            .count()
    }

    fn min_players_for(&self, uuid: Uuid, raw_match_size: u8) -> usize {
        match self.policy.game {
            GameKind::TowerWars => {
                let size = self
                    .queue
                    .values()
                    .find(|entry| entry.uuid == uuid)
                    .map_or_else(
                        || normalize_match_size(GameKind::TowerWars, raw_match_size),
                        |entry| entry.match_size,
                    );
                usize::from(size)
            }
            GameKind::SheepWars => self.policy.queue_min_players,
        }
    }

    pub fn expected_players(&self, server_id: &str) -> Option<u32> {
        self.servers
            .get(server_id)
            .and_then(|server| server.expected_players)
    }

    fn next_seq(&self) -> u64 {
        self.queue
            .keys()
            .next_back()
            .map_or(FIRST_SEQ, |seq| seq + 1)
    }

    fn entry_seq(&self, uuid: Uuid) -> Option<u64> {
        self.queue
            .iter()
            .find(|(_, entry)| entry.uuid == uuid)
            .map(|(seq, _)| *seq)
    }

    fn has_matchmaking_transfer(&self, uuid: Uuid) -> bool {
        self.player_transfers
            .get(&uuid)
            .and_then(|tid| self.transfers.get(tid))
            .is_some_and(|active| !active.is_lobby_return)
    }

    fn remove_entry_by_uuid(&mut self, uuid: Uuid) -> Option<(u64, QueuedPlayer)> {
        let seq = self.entry_seq(uuid)?;
        let player = self.queue.remove(&seq)?;
        self.mutations
            .push(QueueMutation::Remove { player_uuid: uuid });
        Some((seq, player))
    }

    fn remove_entry(&mut self, uuid: Uuid) -> Option<QueuedPlayer> {
        self.remove_entry_by_uuid(uuid).map(|(_, player)| player)
    }

    fn pending_transfer_counts(&self) -> HashMap<String, u32> {
        let mut counts = HashMap::new();
        for t in self.transfers.values() {
            if !t.is_lobby_return {
                *counts.entry(t.command.target_server.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    fn build_join_response(
        &self,
        status: QueueJoinStatus,
        position: usize,
        target_server: Option<String>,
        min_players: usize,
    ) -> JoinQueueResponse {
        let ready_servers = self
            .servers
            .values()
            .filter(|s| {
                !self.deleting.contains_key(&s.server_id)
                    && s.ready_for_proxy()
                    && s.phase.accepts_players()
            })
            .count();
        let active_servers = self
            .servers
            .values()
            .filter(|s| !self.deleting.contains_key(&s.server_id) && s.phase.is_active())
            .count();

        JoinQueueResponse {
            status,
            position,
            total_queued: self.waiting_len(),
            min_players,
            target_server,
            ready_servers,
            active_servers,
            max_servers: self.policy.max_replicas,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy() -> FleetPolicy {
        FleetPolicy {
            game: GameKind::SheepWars,
            lobby_server_name: "lobby".to_owned(),
            queue_min_players: 2,
            max_players_per_server: 4,
            min_replicas: 1,
            max_replicas: 3,
            standby_servers: 1,
            idle_ttl: Duration::from_secs(60),
            starting_timeout: Duration::from_secs(120),
            transfer_timeout: Duration::from_secs(15),
            empty_grace: Duration::from_secs(30),
            ending_timeout: Duration::from_secs(60),
            heartbeat_timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn queue_waits_below_min_then_dispatches_batch() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.upsert_infra_server(
            t0,
            "sheepwars-1".to_owned(),
            Some("sheepwars-1".to_owned()),
            Some("10.0.0.10:25565".to_owned()),
            true,
            false,
        );

        let p1 = Uuid::new_v4();
        let r1 = engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: p1,
                player_name: "Alice".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
        assert_eq!(r1.status, QueueJoinStatus::Queued);
        assert_eq!(r1.position, 1);

        let tick1 = engine.tick(t0 + Duration::from_secs(1));
        assert!(tick1.proxy_events.is_empty());

        let p2 = Uuid::new_v4();
        engine.join_queue(
            t0 + Duration::from_secs(2),
            JoinQueueRequest {
                player_uuid: p2,
                player_name: "Bob".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );

        let tick2 = engine.tick(t0 + Duration::from_secs(2));
        let transfers: Vec<_> = tick2
            .proxy_events
            .iter()
            .filter_map(|e| match e {
                ProxyEvent::TransferPlayer {
                    player_uuid,
                    target_server,
                    ..
                } => Some((*player_uuid, target_server.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(transfers.len(), 2);
        assert_eq!(transfers[0], (p1, "sheepwars-1".to_owned()));
        assert_eq!(transfers[1], (p2, "sheepwars-1".to_owned()));
        assert_eq!(engine.snapshot(t0).queue_len, 0);
    }

    #[test]
    fn lobby_quit_keeps_transferring_player_but_proxy_disconnect_clears() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.upsert_infra_server(
            t0,
            "sheepwars-1".to_owned(),
            None,
            Some("10.0.0.10:25565".to_owned()),
            true,
            false,
        );
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: p1,
                player_name: "Alice".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
        engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: p2,
                player_name: "Bob".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
        engine.tick(t0);

        assert_eq!(
            engine.player_status(t0, p1).state,
            PlayerQueueState::Transferring
        );
        engine.leave_queue(p1, LeaveReason::LobbyQuit);
        assert_eq!(
            engine.player_status(t0, p1).state,
            PlayerQueueState::Transferring
        );

        engine.leave_queue(p1, LeaveReason::ProxyDisconnect);
        assert_eq!(
            engine.player_status(t0, p1).state,
            PlayerQueueState::NotQueued
        );
    }

    #[test]
    fn scales_new_server_when_in_game_and_respects_k_limit() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.upsert_infra_server(
            t0,
            "sheepwars-1".to_owned(),
            None,
            Some("10.0.0.10:25565".to_owned()),
            true,
            false,
        );
        engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "sheepwars-1".to_owned(),
                address: None,
                phase: ServerPhase::InGame,
                player_count: 4,
                max_players: 4,
                player_uuids: Vec::new(),
                version: None,
                kind: Default::default(),
            },
        );

        let tick1 = engine.tick(t0);
        // min_replicas=1 is satisfied by sheepwars-1 when queue is empty -> 0 created
        assert_eq!(tick1.servers_to_create, 0);

        // When a player joins queue while sheepwars-1 is InGame, spawn a new server (up to k=3)
        let p1 = Uuid::new_v4();
        engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: p1,
                player_name: "Alice".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
        let tick2 = engine.tick(t0);
        assert_eq!(tick2.servers_to_create, 1);

        // Put 2 more servers into InGame (total 3 = k max), and verify it never exceeds k=3
        engine.upsert_infra_server(
            t0,
            "sheepwars-2".to_owned(),
            None,
            Some("10.0.0.11:25565".to_owned()),
            true,
            false,
        );
        engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "sheepwars-2".to_owned(),
                address: None,
                phase: ServerPhase::InGame,
                player_count: 4,
                max_players: 4,
                player_uuids: Vec::new(),
                version: None,
                kind: Default::default(),
            },
        );
        engine.upsert_infra_server(
            t0,
            "sheepwars-3".to_owned(),
            None,
            Some("10.0.0.12:25565".to_owned()),
            true,
            false,
        );
        engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "sheepwars-3".to_owned(),
                address: None,
                phase: ServerPhase::InGame,
                player_count: 4,
                max_players: 4,
                player_uuids: Vec::new(),
                version: None,
                kind: Default::default(),
            },
        );

        let tick3 = engine.tick(t0);
        assert_eq!(tick3.servers_to_create, 0);
    }

    #[test]
    fn ending_empty_server_is_reaped_and_excess_idle_scales_down() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.upsert_infra_server(
            t0,
            "sheepwars-1".to_owned(),
            Some("sheepwars-1".to_owned()),
            Some("10.0.0.10:25565".to_owned()),
            true,
            false,
        );
        engine.upsert_infra_server(
            t0,
            "sheepwars-2".to_owned(),
            Some("sheepwars-2".to_owned()),
            Some("10.0.0.11:25565".to_owned()),
            true,
            false,
        );
        let hb_events = engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "sheepwars-1".to_owned(),
                address: None,
                phase: ServerPhase::Ending,
                player_count: 0,
                max_players: 4,
                player_uuids: Vec::new(),
                version: None,
                kind: Default::default(),
            },
        );
        assert!(hb_events.iter().any(|e| matches!(
            e,
            ProxyEvent::ServerRemove { name } if name == "sheepwars-1"
        )));

        let tick1 = engine.tick(t0 + Duration::from_secs(1));
        assert_eq!(
            tick1.servers_to_delete,
            vec![ServerDeletion {
                server_id: "sheepwars-1".to_owned(),
                reason: DeletionReason::RoundEnded,
            }]
        );

        // Re-add a 3rd idle Waiting server and expire idle TTL -> excess above min_replicas=1 is scaled down
        engine.remove_server("sheepwars-1");
        engine.upsert_infra_server(
            t0,
            "sheepwars-3".to_owned(),
            Some("sheepwars-3".to_owned()),
            Some("10.0.0.12:25565".to_owned()),
            true,
            false,
        );
        let tick2 = engine.tick(t0 + Duration::from_secs(70));
        assert_eq!(tick2.servers_to_delete.len(), 1);
    }

    fn infra(engine: &mut FleetEngine, now: Instant, id: &str) {
        engine.upsert_infra_server(
            now,
            id.to_owned(),
            Some(id.to_owned()),
            Some("10.0.0.10:25565".to_owned()),
            true,
            false,
        );
    }

    fn heartbeat(
        engine: &mut FleetEngine,
        now: Instant,
        id: &str,
        phase: ServerPhase,
        players: u32,
    ) {
        let player_uuids = (0..players)
            .map(|n| Uuid::from_u128(u128::from(n) + 1))
            .collect();
        engine.apply_heartbeat(
            now,
            HeartbeatRequest {
                server_id: id.to_owned(),
                address: None,
                phase,
                player_count: players,
                max_players: 4,
                player_uuids,
                version: None,
                kind: Default::default(),
            },
        );
    }

    fn deleted(actions: &TickActions) -> Vec<(&str, DeletionReason)> {
        actions
            .servers_to_delete
            .iter()
            .map(|d| (d.server_id.as_str(), d.reason))
            .collect()
    }

    #[test]
    fn seen_players_accumulate_while_the_server_is_known() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 1);
        let seen = engine.seen_players("sheepwars-1");
        assert_eq!(seen.len(), 2);
        assert!(seen.contains(&Uuid::from_u128(1)));
        assert!(seen.contains(&Uuid::from_u128(2)));
        assert!(engine.seen_players("unknown").is_empty());
        engine.remove_server("sheepwars-1");
        assert!(engine.seen_players("sheepwars-1").is_empty());
    }

    #[test]
    fn ending_server_with_stale_players_is_reaped_after_ending_timeout() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Ending, 2);

        for secs in [20, 40, 59] {
            let now = t0 + Duration::from_secs(secs);
            heartbeat(&mut engine, now, "sheepwars-1", ServerPhase::Ending, 2);
            assert!(deleted(&engine.tick(now)).is_empty(), "reaped at {secs}s");
        }
        assert_eq!(
            deleted(&engine.tick(t0 + Duration::from_secs(60))),
            [("sheepwars-1", DeletionReason::EndingTimeout)]
        );
    }

    #[test]
    fn in_game_server_left_empty_is_reaped_after_grace() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);
        heartbeat(
            &mut engine,
            t0 + Duration::from_secs(5),
            "sheepwars-1",
            ServerPhase::InGame,
            0,
        );

        let t_grace = t0 + Duration::from_secs(34);
        heartbeat(&mut engine, t_grace, "sheepwars-1", ServerPhase::InGame, 0);
        assert!(deleted(&engine.tick(t_grace)).is_empty());

        let t_after = t0 + Duration::from_secs(35);
        heartbeat(&mut engine, t_after, "sheepwars-1", ServerPhase::InGame, 0);
        assert_eq!(
            deleted(&engine.tick(t_after)),
            [("sheepwars-1", DeletionReason::EmptyInGame)]
        );
    }

    #[test]
    fn a_player_coming_back_resets_the_empty_grace() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 0);
        heartbeat(
            &mut engine,
            t0 + Duration::from_secs(20),
            "sheepwars-1",
            ServerPhase::InGame,
            1,
        );
        let later = t0 + Duration::from_secs(40);
        heartbeat(&mut engine, later, "sheepwars-1", ServerPhase::InGame, 0);
        assert!(deleted(&engine.tick(later)).is_empty());
    }

    #[test]
    fn server_whose_plugin_goes_silent_is_reaped() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);

        assert!(deleted(&engine.tick(t0 + Duration::from_secs(29))).is_empty());
        assert_eq!(
            deleted(&engine.tick(t0 + Duration::from_secs(30))),
            [("sheepwars-1", DeletionReason::HeartbeatLost)]
        );
    }

    #[test]
    fn server_without_plugin_heartbeat_is_not_reaped_for_silence() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        assert!(deleted(&engine.tick(t0 + Duration::from_secs(300))).is_empty());
    }

    #[test]
    fn reaped_server_is_reported_once() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Ending, 0);
        assert_eq!(deleted(&engine.tick(t0)).len(), 1);
        assert!(deleted(&engine.tick(t0 + Duration::from_secs(1))).is_empty());
    }

    #[test]
    fn dispatch_between_ticks_leaves_reaping_to_the_reconciler() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.register_creating_server(t0, "sheepwars-stuck".to_owned(), None);
        let timed_out = t0 + Duration::from_secs(120);

        engine.dispatch(timed_out);

        assert_eq!(
            deleted(&engine.tick(timed_out)),
            vec![("sheepwars-stuck", DeletionReason::StartingTimeout)]
        );
    }

    #[test]
    fn deletion_is_retried_while_the_server_is_still_listed() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Ending, 0);
        engine.tick(t0);
        let retry_after = Duration::from_secs(30);

        assert!(
            engine
                .deletions_to_retry(t0 + Duration::from_secs(29), retry_after)
                .is_empty()
        );
        assert_eq!(
            engine.deletions_to_retry(t0 + Duration::from_secs(30), retry_after),
            vec!["sheepwars-1".to_owned()]
        );
        assert!(
            engine
                .deletions_to_retry(t0 + Duration::from_secs(31), retry_after)
                .is_empty()
        );
    }

    #[test]
    fn deletion_is_not_retried_once_the_server_is_gone() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Ending, 0);
        engine.tick(t0);
        engine.retain_infra_servers(&HashSet::new());

        assert!(
            engine
                .deletions_to_retry(t0 + Duration::from_secs(60), Duration::from_secs(30))
                .is_empty()
        );
    }

    fn waiting(engine: &mut FleetEngine, now: Instant, id: &str) {
        infra(engine, now, id);
        heartbeat(engine, now, id, ServerPhase::Waiting, 0);
    }

    #[test]
    fn outdated_idle_server_gets_a_replacement_before_it_is_deleted() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "old");
        engine.mark_outdated("old");

        let first = engine.tick(t0);
        assert_eq!(first.servers_to_create, 1);
        assert!(deleted(&first).is_empty());

        waiting(&mut engine, t0, "new");
        let second = engine.tick(t0 + Duration::from_secs(1));
        assert_eq!(deleted(&second), [("old", DeletionReason::Outdated)]);
        assert_eq!(second.servers_to_create, 0);
    }

    #[test]
    fn outdated_server_in_game_is_kept_until_its_round_ends() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "old");
        heartbeat(&mut engine, t0, "old", ServerPhase::InGame, 2);
        engine.mark_outdated("old");
        waiting(&mut engine, t0, "new");

        assert!(deleted(&engine.tick(t0)).is_empty());

        heartbeat(&mut engine, t0, "old", ServerPhase::Ending, 0);
        assert_eq!(
            deleted(&engine.tick(t0 + Duration::from_secs(1))),
            [("old", DeletionReason::RoundEnded)]
        );
    }

    #[test]
    fn queue_prefers_an_up_to_date_server() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "old");
        heartbeat(&mut engine, t0, "old", ServerPhase::Waiting, 1);
        engine.mark_outdated("old");
        waiting(&mut engine, t0, "new");

        for n in 10..12u128 {
            engine.join_queue(
                t0,
                JoinQueueRequest {
                    player_uuid: Uuid::from_u128(n),
                    player_name: format!("p{n}"),
                    game: GameKind::SheepWars,
                    match_size: 0,
                },
            );
        }
        let actions = engine.tick(t0);
        let targets: Vec<&str> = actions
            .proxy_events
            .iter()
            .filter_map(|event| match event {
                ProxyEvent::TransferPlayer { target_server, .. } => Some(target_server.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(targets, ["new", "new"]);
    }

    #[test]
    fn outdated_server_with_a_waiting_player_still_takes_players_when_alone() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "old");
        heartbeat(&mut engine, t0, "old", ServerPhase::Waiting, 1);
        engine.mark_outdated("old");
        engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: Uuid::from_u128(10),
                player_name: "p10".to_owned(),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
        let actions = engine.tick(t0);
        assert!(actions.proxy_events.iter().any(|event| matches!(
            event,
            ProxyEvent::TransferPlayer { target_server, .. } if target_server == "old"
        )));
        assert!(deleted(&actions).is_empty());
    }

    fn transfer_targets(actions: &TickActions) -> Vec<(Uuid, &str)> {
        actions
            .proxy_events
            .iter()
            .filter_map(|event| match event {
                ProxyEvent::TransferPlayer {
                    player_uuid,
                    target_server,
                    ..
                } => Some((*player_uuid, target_server.as_str())),
                _ => None,
            })
            .collect()
    }

    fn queue(engine: &mut FleetEngine, now: Instant, n: u128) {
        engine.join_queue(
            now,
            JoinQueueRequest {
                player_uuid: Uuid::from_u128(n),
                player_name: format!("p{n}"),
                game: GameKind::SheepWars,
                match_size: 0,
            },
        );
    }

    #[test]
    fn with_a_minimum_of_one_a_lone_player_goes_straight_to_a_waiting_server() {
        let mut engine = FleetEngine::new(FleetPolicy {
            queue_min_players: 1,
            ..test_policy()
        });
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 10);
        assert_eq!(
            transfer_targets(&engine.tick(t0)),
            [(Uuid::from_u128(10), "sheepwars-1")]
        );
    }

    #[test]
    fn later_players_join_the_server_that_already_has_someone_waiting() {
        let mut engine = FleetEngine::new(FleetPolicy {
            queue_min_players: 1,
            ..test_policy()
        });
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        waiting(&mut engine, t0, "sheepwars-2");
        heartbeat(&mut engine, t0, "sheepwars-2", ServerPhase::Waiting, 1);
        queue(&mut engine, t0, 10);
        queue(&mut engine, t0, 11);
        let actions = engine.tick(t0);
        let targets: Vec<&str> = transfer_targets(&actions)
            .into_iter()
            .map(|(_, server)| server)
            .collect();
        assert_eq!(targets, ["sheepwars-2", "sheepwars-2"]);
    }

    #[test]
    fn summary_reports_version_age_and_outdated_flag() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "sheepwars-1".to_owned(),
                address: None,
                phase: ServerPhase::Waiting,
                player_count: 0,
                max_players: 4,
                player_uuids: Vec::new(),
                version: Some("abc1234".to_owned()),
                kind: Default::default(),
            },
        );
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Waiting, 0);
        engine.mark_outdated("sheepwars-1");
        let snap = engine.snapshot(t0 + Duration::from_secs(42));
        let summary = &snap.servers[0];
        assert_eq!(summary.version.as_deref(), Some("abc1234"));
        assert_eq!(summary.age_secs, 42);
        assert!(summary.outdated);
    }

    #[test]
    fn spectate_transfers_the_player_and_takes_them_out_of_the_queue() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);
        queue(&mut engine, t0, 10);
        let event = engine
            .queue_spectate_transfer(t0, "sheepwars-1", Uuid::from_u128(10))
            .unwrap();
        assert!(matches!(
            event,
            ProxyEvent::TransferPlayer { ref target_server, ref reason, .. }
                if target_server == "sheepwars-1" && reason == "spectate"
        ));
        assert!(engine.snapshot(t0).queued_players.is_empty());
    }

    #[test]
    fn spectate_rejects_unknown_and_ending_servers() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        assert!(matches!(
            engine.queue_spectate_transfer(t0, "nope", Uuid::from_u128(1)),
            Err(EngineError::UnknownServer(_))
        ));
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Ending, 1);
        assert!(matches!(
            engine.queue_spectate_transfer(t0, "sheepwars-1", Uuid::from_u128(1)),
            Err(EngineError::NotJoinable(_))
        ));
    }

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn lobby_heartbeat(
        engine: &mut FleetEngine,
        now: Instant,
        id: &str,
        address: Option<&str>,
        players: &[u128],
    ) -> Vec<ProxyEvent> {
        engine.apply_heartbeat(
            now,
            HeartbeatRequest {
                server_id: id.to_owned(),
                address: address.map(str::to_owned),
                phase: ServerPhase::Waiting,
                player_count: players.len() as u32,
                max_players: 50,
                player_uuids: players.iter().map(|n| uid(*n)).collect(),
                version: None,
                kind: ServerKind::Lobby,
            },
        )
    }

    fn game_heartbeat(
        engine: &mut FleetEngine,
        now: Instant,
        id: &str,
        phase: ServerPhase,
        players: &[u128],
    ) {
        engine.apply_heartbeat(
            now,
            HeartbeatRequest {
                server_id: id.to_owned(),
                address: None,
                phase,
                player_count: players.len() as u32,
                max_players: 4,
                player_uuids: players.iter().map(|n| uid(*n)).collect(),
                version: None,
                kind: ServerKind::SheepWars,
            },
        );
    }

    fn transfer_ids(actions: &TickActions) -> Vec<(Uuid, Uuid)> {
        actions
            .proxy_events
            .iter()
            .filter_map(|event| match event {
                ProxyEvent::TransferPlayer {
                    transfer_id,
                    player_uuid,
                    ..
                } => Some((*player_uuid, *transfer_id)),
                _ => None,
            })
            .collect()
    }

    fn queued(engine: &FleetEngine, now: Instant) -> Vec<Uuid> {
        engine
            .snapshot(now)
            .queued_players
            .iter()
            .map(|p| p.player_uuid)
            .collect()
    }

    fn lobby_transfer_targets(events: &[ProxyEvent]) -> Vec<(Uuid, &str)> {
        events
            .iter()
            .filter_map(|event| match event {
                ProxyEvent::TransferPlayer {
                    player_uuid,
                    target_server,
                    ..
                } => Some((*player_uuid, target_server.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lobby_heartbeat_registers_a_lobby_instance_and_not_a_sheepwars_server() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        let events = lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        assert_eq!(
            events,
            [ProxyEvent::ServerUpsert {
                name: "lobby-a".to_owned(),
                address: "10.0.0.3:25565".to_owned(),
                phase: ServerPhase::Waiting,
                kind: ServerKind::Lobby,
            }]
        );
        let snap = engine.snapshot(t0);
        assert!(snap.servers.is_empty());
        assert_eq!(snap.lobbies.len(), 1);
        assert_eq!(snap.lobbies[0].kind, ServerKind::Lobby);
        assert!(snap.lobbies[0].ready);
        assert!(!snap.lobbies[0].draining);
        assert_eq!(
            engine.pick_lobby().map(|l| l.state),
            Some(LobbyState::Ready)
        );
        let proxy = engine.proxy_snapshot();
        assert_eq!(proxy.servers.len(), 1);
        assert_eq!(proxy.servers[0].kind, ServerKind::Lobby);
        assert!(
            lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]).is_empty()
        );
    }

    #[test]
    fn lobby_without_address_stays_starting_until_it_has_one() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        assert!(lobby_heartbeat(&mut engine, t0, "lobby-a", None, &[]).is_empty());
        assert!(engine.pick_lobby().is_none());
        assert!(!engine.snapshot(t0).lobbies[0].ready);
        let events = lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        assert_eq!(events.len(), 1);
        assert!(engine.pick_lobby().is_some());
    }

    #[test]
    fn lobby_is_removed_after_the_heartbeat_timeout() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        assert!(
            engine
                .tick(t0 + Duration::from_secs(29))
                .proxy_events
                .is_empty()
        );
        let actions = engine.tick(t0 + Duration::from_secs(30));
        assert_eq!(
            actions.proxy_events,
            [ProxyEvent::ServerRemove {
                name: "lobby-a".to_owned()
            }]
        );
        assert!(engine.pick_lobby().is_none());
        assert!(engine.snapshot(t0).lobbies.is_empty());
    }

    #[test]
    fn lobbies_are_ignored_by_fleet_planning_and_infra_reaping() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[1, 2]);
        let actions = engine.tick(t0);
        assert_eq!(actions.servers_to_create, 1);
        assert!(actions.servers_to_delete.is_empty());
        assert!(engine.retain_infra_servers(&HashSet::new()).is_empty());
        assert_eq!(engine.snapshot(t0).lobbies.len(), 1);
        let late = engine.tick(t0 + Duration::from_secs(25));
        assert!(late.servers_to_delete.is_empty());
    }

    #[test]
    fn pick_lobby_returns_the_ready_instance_with_the_fewest_players() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        assert!(engine.pick_lobby().is_none());
        lobby_heartbeat(
            &mut engine,
            t0,
            "lobby-a",
            Some("10.0.0.3:25565"),
            &[1, 2, 3],
        );
        lobby_heartbeat(&mut engine, t0, "lobby-b", Some("10.0.0.4:25565"), &[4]);
        lobby_heartbeat(&mut engine, t0, "lobby-c", None, &[]);
        assert_eq!(engine.pick_lobby().map(|l| l.id.as_str()), Some("lobby-b"));
    }

    #[test]
    fn plc_4_a_draining_lobby_is_never_picked() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        lobby_heartbeat(
            &mut engine,
            t0,
            "lobby-b",
            Some("10.0.0.4:25565"),
            &[1, 2, 3],
        );
        assert_eq!(engine.pick_lobby().map(|l| l.id.as_str()), Some("lobby-a"));
        engine.drain_lobby(t0, "lobby-a").unwrap();
        assert_eq!(engine.pick_lobby().map(|l| l.id.as_str()), Some("lobby-b"));
        let events = lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        assert!(events.is_empty());
        assert_eq!(engine.pick_lobby().map(|l| l.id.as_str()), Some("lobby-b"));
        assert!(engine.snapshot(t0).lobbies.iter().any(|l| l.draining));
        let proxy = engine.proxy_snapshot();
        assert_eq!(proxy.servers.len(), 1);
        assert_eq!(proxy.servers[0].name, "lobby-b");
    }

    #[test]
    fn lobby_returns_target_the_picked_instance_or_fall_back_to_the_service() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        let fallback = engine.queue_lobby_transfer(t0, uid(1), None);
        assert!(matches!(
            fallback,
            ProxyEvent::TransferPlayer { ref target_server, .. } if target_server == "lobby"
        ));
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[1, 2]);
        lobby_heartbeat(&mut engine, t0, "lobby-b", Some("10.0.0.4:25565"), &[3]);
        let picked = engine.queue_lobby_transfer(t0, uid(9), Some("fin de partie".to_owned()));
        assert!(matches!(
            picked,
            ProxyEvent::TransferPlayer { ref target_server, .. } if target_server == "lobby-b"
        ));
    }

    #[test]
    fn drain_spreads_players_over_the_least_loaded_other_lobbies_and_removes_the_instance() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(
            &mut engine,
            t0,
            "lobby-a",
            Some("10.0.0.3:25565"),
            &[1, 2, 3],
        );
        lobby_heartbeat(&mut engine, t0, "lobby-b", Some("10.0.0.4:25565"), &[]);
        lobby_heartbeat(&mut engine, t0, "lobby-c", Some("10.0.0.5:25565"), &[]);
        let result = engine.drain_lobby(t0, "lobby-a").unwrap();
        assert_eq!(result.remaining_players, 3);
        assert_eq!(
            result.events[0],
            ProxyEvent::ServerRemove {
                name: "lobby-a".to_owned()
            }
        );
        assert_eq!(
            lobby_transfer_targets(&result.events),
            [
                (uid(1), "lobby-b"),
                (uid(2), "lobby-c"),
                (uid(3), "lobby-b")
            ]
        );
        assert_eq!(
            engine
                .snapshot(t0)
                .lobbies
                .iter()
                .filter(|l| l.draining)
                .count(),
            1
        );
    }

    #[test]
    fn drain_can_be_polled_without_duplicating_transfers() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[1, 2]);
        lobby_heartbeat(&mut engine, t0, "lobby-b", Some("10.0.0.4:25565"), &[]);
        assert_eq!(engine.drain_lobby(t0, "lobby-a").unwrap().events.len(), 3);
        let again = engine.drain_lobby(t0, "lobby-a").unwrap();
        assert_eq!(again.remaining_players, 2);
        assert!(again.events.is_empty());
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[]);
        let done = engine.drain_lobby(t0, "lobby-a").unwrap();
        assert_eq!(done.remaining_players, 0);
        assert!(done.events.is_empty());
    }

    #[test]
    fn plc_5_drain_without_another_ready_lobby_is_refused() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.3:25565"), &[1]);
        assert_eq!(
            engine.drain_lobby(t0, "lobby-a").unwrap_err(),
            DrainError::NoOtherLobby
        );
        lobby_heartbeat(&mut engine, t0, "lobby-b", None, &[]);
        assert_eq!(
            engine.drain_lobby(t0, "lobby-a").unwrap_err(),
            DrainError::NoOtherLobby
        );
        assert!(engine.pick_lobby().is_some());
    }

    #[test]
    fn drain_rejects_unknown_ids_and_sheepwars_servers() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        assert_eq!(
            engine.drain_lobby(t0, "nope").unwrap_err(),
            DrainError::UnknownServer
        );
        assert_eq!(
            engine.drain_lobby(t0, "sheepwars-1").unwrap_err(),
            DrainError::NotALobby
        );
    }

    #[test]
    fn join_and_dispatch_are_reported_as_queue_mutations() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 10);
        queue(&mut engine, t0, 11);
        engine.tick(t0);
        let mutations = engine.take_mutations();
        assert!(
            matches!(mutations[0], QueueMutation::Insert { player_uuid, .. } if player_uuid == uid(10))
        );
        assert!(
            matches!(mutations[1], QueueMutation::Insert { player_uuid, .. } if player_uuid == uid(11))
        );
        assert!(matches!(
            &mutations[2],
            QueueMutation::SetState { player_uuid, state: QueueEntryState::Dispatched { server_id, .. } }
                if *player_uuid == uid(10) && server_id == "sheepwars-1"
        ));
        assert_eq!(mutations.len(), 4);
        assert!(engine.take_mutations().is_empty());
        engine.restore_mutations(mutations);
        assert_eq!(engine.take_mutations().len(), 4);
    }

    #[test]
    fn dispatched_entries_leave_the_queue_only_when_their_uuid_is_in_a_heartbeat() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 10);
        queue(&mut engine, t0, 11);
        let actions = engine.tick(t0);
        let (_, first) = transfer_ids(&actions)[0];
        engine.take_mutations();

        engine.ack_transfer(t0, first, true);
        assert!(engine.take_mutations().is_empty());
        assert_eq!(
            engine.player_status(t0, uid(10)).state,
            PlayerQueueState::Transferring
        );

        game_heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::Waiting, &[10]);
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::Remove {
                player_uuid: uid(10)
            }]
        );
        assert_eq!(
            engine.player_status(t0, uid(10)).state,
            PlayerQueueState::NotQueued
        );
        assert_eq!(
            engine.player_status(t0, uid(11)).state,
            PlayerQueueState::Transferring
        );
    }

    #[test]
    fn leaving_voluntarily_removes_a_waiting_entry_and_proxy_disconnect_removes_a_dispatched_one() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        queue(&mut engine, t0, 10);
        engine.take_mutations();
        assert!(engine.leave_queue(uid(10), LeaveReason::UserToggle).removed);
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::Remove {
                player_uuid: uid(10)
            }]
        );

        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 11);
        queue(&mut engine, t0, 12);
        engine.tick(t0);
        engine.take_mutations();
        assert!(!engine.leave_queue(uid(11), LeaveReason::LobbyQuit).removed);
        assert!(engine.take_mutations().is_empty());
        assert!(
            engine
                .leave_queue(uid(11), LeaveReason::ProxyDisconnect)
                .removed
        );
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::Remove {
                player_uuid: uid(11)
            }]
        );
    }

    #[test]
    fn plc_14_a_failed_transfer_puts_the_player_back_at_their_place() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 10);
        queue(&mut engine, t0, 11);
        let actions = engine.tick(t0);
        let ids = transfer_ids(&actions);
        game_heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, &[]);
        queue(&mut engine, t0, 12);
        engine.take_mutations();

        engine.ack_transfer(t0, ids[0].1, false);
        assert_eq!(queued(&engine, t0), [uid(10), uid(12)]);
        assert_eq!(engine.player_status(t0, uid(10)).position, Some(1));
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::SetState {
                player_uuid: uid(10),
                state: QueueEntryState::Waiting
            }]
        );
    }

    #[test]
    fn plc_14_an_expired_transfer_puts_the_player_back_at_their_place() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        waiting(&mut engine, t0, "sheepwars-1");
        queue(&mut engine, t0, 10);
        queue(&mut engine, t0, 11);
        engine.tick(t0);
        game_heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, &[]);
        queue(&mut engine, t0, 12);
        engine.take_mutations();

        engine.tick(t0 + Duration::from_secs(14));
        assert_eq!(queued(&engine, t0), [uid(12)]);

        game_heartbeat(
            &mut engine,
            t0 + Duration::from_secs(14),
            "sheepwars-1",
            ServerPhase::InGame,
            &[],
        );
        engine.tick(t0 + Duration::from_secs(15));
        assert_eq!(queued(&engine, t0), [uid(10), uid(11), uid(12)]);
        let mutations = engine.take_mutations();
        assert_eq!(
            mutations
                .iter()
                .filter(|m| matches!(
                    m,
                    QueueMutation::SetState {
                        state: QueueEntryState::Waiting,
                        ..
                    }
                ))
                .count(),
            2
        );
    }

    fn dispatched_record(seq: u64, n: u128, server: &str) -> QueueRecord {
        QueueRecord {
            seq,
            player_uuid: uid(n),
            name: format!("p{n}"),
            joined_at: chrono::Utc::now(),
            state: QueueEntryState::Dispatched {
                server_id: server.to_owned(),
                transfer_id: Uuid::from_u128(n + 1000),
            },
            game: GameKind::SheepWars,
            match_size: 0,
        }
    }

    #[test]
    fn restore_queue_keeps_the_seq_order() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        let records = [30, 10, 20]
            .iter()
            .map(|seq| QueueRecord {
                seq: *seq,
                player_uuid: uid(u128::from(*seq)),
                name: format!("p{seq}"),
                joined_at: chrono::Utc::now(),
                state: QueueEntryState::Waiting,
                game: GameKind::SheepWars,
                match_size: 0,
            })
            .collect();
        engine.restore_queue(t0, records);
        assert_eq!(queued(&engine, t0), [uid(10), uid(20), uid(30)]);
        assert_eq!(engine.player_status(t0, uid(20)).position, Some(2));
        assert!(engine.take_mutations().is_empty());
    }

    #[test]
    fn plc_15_a_dispatched_entry_survives_a_restart_until_a_heartbeat_shows_the_player() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.restore_queue(t0, vec![dispatched_record(5, 10, "sheepwars-1")]);
        let status = engine.player_status(t0, uid(10));
        assert_eq!(status.state, PlayerQueueState::Transferring);
        assert_eq!(status.target_server.as_deref(), Some("sheepwars-1"));
        engine.tick(t0 + Duration::from_secs(10));
        assert_eq!(
            engine.player_status(t0, uid(10)).state,
            PlayerQueueState::Transferring
        );

        waiting(&mut engine, t0, "sheepwars-1");
        game_heartbeat(
            &mut engine,
            t0 + Duration::from_secs(11),
            "sheepwars-1",
            ServerPhase::Waiting,
            &[10],
        );
        assert_eq!(
            engine.player_status(t0, uid(10)).state,
            PlayerQueueState::NotQueued
        );
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::Remove {
                player_uuid: uid(10)
            }]
        );
    }

    #[test]
    fn plc_15_a_dispatched_entry_not_seen_before_the_timeout_is_waiting_again() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        engine.restore_queue(
            t0,
            vec![
                dispatched_record(5, 10, "sheepwars-1"),
                QueueRecord {
                    seq: 6,
                    player_uuid: uid(11),
                    name: "p11".to_owned(),
                    joined_at: chrono::Utc::now(),
                    state: QueueEntryState::Waiting,
                    game: GameKind::SheepWars,
                    match_size: 0,
                },
            ],
        );
        assert_eq!(queued(&engine, t0), [uid(11)]);
        engine.tick(t0 + Duration::from_secs(15));
        assert_eq!(queued(&engine, t0), [uid(10), uid(11)]);
        assert_eq!(engine.player_status(t0, uid(10)).position, Some(1));
        assert_eq!(
            engine.take_mutations(),
            [QueueMutation::SetState {
                player_uuid: uid(10),
                state: QueueEntryState::Waiting
            }]
        );
    }

    fn game_with_arrived_players(engine: &mut FleetEngine, t0: Instant) {
        waiting(engine, t0, "sheepwars-1");
        queue(engine, t0, 10);
        queue(engine, t0, 11);
        engine.tick(t0);
        game_heartbeat(engine, t0, "sheepwars-1", ServerPhase::Waiting, &[11, 10]);
        queue(engine, t0, 12);
        engine.take_mutations();
    }

    #[test]
    fn requeues_the_players_of_a_waiting_server_that_disappears_ahead_of_later_arrivals() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        game_with_arrived_players(&mut engine, t0);
        assert_eq!(queued(&engine, t0), [uid(12)]);

        engine.remove_server("sheepwars-1");
        assert_eq!(queued(&engine, t0), [uid(10), uid(11), uid(12)]);
        let mutations = engine.take_mutations();
        let seqs: Vec<(Uuid, u64)> = mutations
            .iter()
            .filter_map(|m| match m {
                QueueMutation::Insert {
                    seq, player_uuid, ..
                } => Some((*player_uuid, *seq)),
                _ => None,
            })
            .collect();
        assert_eq!(seqs.len(), 2);
        assert_eq!(seqs[0].0, uid(10));
        assert_eq!(seqs[1].0, uid(11));
        assert!(seqs[0].1 < seqs[1].1);
        assert!(seqs[1].1 < FIRST_SEQ + 3);
        assert_eq!(engine.player_status(t0, uid(10)).position, Some(1));
        engine.remove_server("sheepwars-1");
        assert!(engine.take_mutations().is_empty());
    }

    #[test]
    fn requeues_when_the_heartbeat_of_a_waiting_server_is_lost() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        game_with_arrived_players(&mut engine, t0);
        game_heartbeat(
            &mut engine,
            t0,
            "sheepwars-1",
            ServerPhase::Countdown,
            &[10, 20, 11, 21],
        );
        let actions = engine.tick(t0 + Duration::from_secs(30));
        assert_eq!(
            deleted(&actions),
            [("sheepwars-1", DeletionReason::HeartbeatLost)]
        );
        assert_eq!(
            queued(&engine, t0),
            [uid(10), uid(11), uid(20), uid(21), uid(12)]
        );
    }

    #[test]
    fn requeues_when_the_pod_is_terminating() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        game_with_arrived_players(&mut engine, t0);
        engine.upsert_infra_server(t0, "sheepwars-1".to_owned(), None, None, true, true);
        assert_eq!(queued(&engine, t0), [uid(10), uid(11), uid(12)]);
    }

    #[test]
    fn does_not_requeue_the_players_of_a_game_in_progress() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        game_with_arrived_players(&mut engine, t0);
        game_heartbeat(
            &mut engine,
            t0,
            "sheepwars-1",
            ServerPhase::InGame,
            &[10, 11],
        );
        engine.remove_server("sheepwars-1");
        assert_eq!(queued(&engine, t0), [uid(12)]);
    }

    #[test]
    fn spectate_server_is_reported_for_sixty_seconds() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        infra(&mut engine, t0, "sheepwars-1");
        heartbeat(&mut engine, t0, "sheepwars-1", ServerPhase::InGame, 2);
        assert_eq!(engine.player_status(t0, uid(7)).spectate_server, None);
        engine
            .queue_spectate_transfer(t0, "sheepwars-1", uid(7))
            .unwrap();
        let status = engine.player_status(t0 + Duration::from_secs(59), uid(7));
        assert_eq!(status.spectate_server.as_deref(), Some("sheepwars-1"));
        assert_eq!(
            engine
                .player_status(t0 + Duration::from_secs(60), uid(7))
                .spectate_server,
            None
        );
    }

    #[test]
    fn a_lobby_shutting_down_is_never_picked_again() {
        let mut engine = FleetEngine::new(test_policy());
        let t0 = Instant::now();
        lobby_heartbeat(&mut engine, t0, "lobby-a", Some("10.0.0.1:25565"), &[]);
        let events = engine.apply_heartbeat(
            t0,
            HeartbeatRequest {
                server_id: "lobby-a".to_owned(),
                address: Some("10.0.0.1:25565".to_owned()),
                phase: ServerPhase::Ending,
                player_count: 0,
                max_players: 50,
                player_uuids: Vec::new(),
                version: None,
                kind: ServerKind::Lobby,
            },
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ProxyEvent::ServerRemove { name } if name == "lobby-a"
        )));
        assert!(engine.pick_lobby().is_none());
    }

    fn tower_engine() -> FleetEngine {
        FleetEngine::new(FleetPolicy {
            game: GameKind::TowerWars,
            queue_min_players: 4,
            max_players_per_server: 12,
            max_replicas: 5,
            ..test_policy()
        })
    }

    fn tower_heartbeat(
        engine: &mut FleetEngine,
        now: Instant,
        id: &str,
        phase: ServerPhase,
        players: &[u128],
    ) {
        engine.apply_heartbeat(
            now,
            HeartbeatRequest {
                server_id: id.to_owned(),
                address: Some("10.0.0.10:25565".to_owned()),
                phase,
                player_count: players.len() as u32,
                max_players: 12,
                player_uuids: players.iter().map(|n| uid(*n)).collect(),
                version: None,
                kind: ServerKind::TowerWars,
            },
        );
    }

    fn tower_join(engine: &mut FleetEngine, now: Instant, n: u128, match_size: u8) {
        engine.join_queue(
            now,
            JoinQueueRequest {
                player_uuid: uid(n),
                player_name: format!("p{n}"),
                game: GameKind::TowerWars,
                match_size,
            },
        );
    }

    #[test]
    fn match_size_is_normalized_for_towerwars_and_ignored_for_sheepwars() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        for (n, raw) in [(1, 0), (2, 9), (3, 3)] {
            tower_join(&mut engine, t0, n, raw);
        }
        let sizes: Vec<u8> = engine
            .take_mutations()
            .into_iter()
            .map(|m| match m {
                QueueMutation::Insert { match_size, .. } => match_size,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(sizes, [1, 1, 3]);

        let mut sheep = FleetEngine::new(test_policy());
        queue(&mut sheep, t0, 1);
        assert!(matches!(
            sheep.take_mutations()[0],
            QueueMutation::Insert { match_size: 0, .. }
        ));
    }

    #[test]
    fn a_solo_player_starts_alone_immediately_and_the_server_expects_one() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 1);
        let actions = engine.tick(t0);
        assert_eq!(transfer_targets(&actions), [(uid(10), "towerwars-1")]);
        assert_eq!(engine.expected_players("towerwars-1"), Some(1));
    }

    #[test]
    fn a_duo_waits_for_two_players_then_leaves_together() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 2);
        assert!(transfer_targets(&engine.tick(t0)).is_empty());
        assert_eq!(engine.expected_players("towerwars-1"), None);
        assert_eq!(engine.player_status(t0, uid(10)).min_players, 2);

        tower_join(&mut engine, t0, 11, 2);
        let actions = engine.tick(t0);
        assert_eq!(
            transfer_targets(&actions),
            [(uid(10), "towerwars-1"), (uid(11), "towerwars-1")]
        );
        assert_eq!(engine.expected_players("towerwars-1"), Some(2));
    }

    #[test]
    fn different_sizes_never_mix_and_groups_leave_in_fifo_order() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_heartbeat(&mut engine, t0, "towerwars-2", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 3);
        tower_join(&mut engine, t0, 11, 2);
        tower_join(&mut engine, t0, 12, 2);
        tower_join(&mut engine, t0, 13, 3);
        let actions = engine.tick(t0);
        assert_eq!(
            transfer_targets(&actions),
            [(uid(11), "towerwars-1"), (uid(12), "towerwars-1")]
        );
        assert_eq!(engine.expected_players("towerwars-2"), None);
        assert_eq!(queued(&engine, t0), [uid(10), uid(13)]);

        tower_join(&mut engine, t0, 14, 3);
        let actions = engine.tick(t0);
        assert_eq!(
            transfer_targets(&actions),
            [
                (uid(10), "towerwars-2"),
                (uid(13), "towerwars-2"),
                (uid(14), "towerwars-2")
            ]
        );
        assert_eq!(engine.expected_players("towerwars-2"), Some(3));
    }

    #[test]
    fn the_s_oldest_of_a_group_leave_and_the_rest_keep_waiting() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        for n in 10..13 {
            tower_join(&mut engine, t0, n, 2);
        }
        let actions = engine.tick(t0);
        assert_eq!(
            transfer_targets(&actions),
            [(uid(10), "towerwars-1"), (uid(11), "towerwars-1")]
        );
        assert_eq!(queued(&engine, t0), [uid(12)]);
    }

    #[test]
    fn a_non_empty_or_already_assigned_server_is_never_backfilled() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 2);
        tower_join(&mut engine, t0, 11, 2);
        engine.tick(t0);
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[10]);
        tower_join(&mut engine, t0, 12, 1);
        assert!(transfer_targets(&engine.tick(t0)).is_empty());
        assert_eq!(queued(&engine, t0), [uid(12)]);

        tower_heartbeat(&mut engine, t0, "towerwars-2", ServerPhase::Waiting, &[]);
        assert_eq!(
            transfer_targets(&engine.tick(t0)),
            [(uid(12), "towerwars-2")]
        );
    }

    #[test]
    fn expected_players_is_cleared_once_the_server_is_empty_and_waiting_again() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 1);
        engine.tick(t0);
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        assert_eq!(engine.expected_players("towerwars-1"), Some(1));
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[10]);
        assert_eq!(engine.expected_players("towerwars-1"), Some(1));
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::InGame, &[10]);
        assert_eq!(engine.expected_players("towerwars-1"), Some(1));
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        assert_eq!(engine.expected_players("towerwars-1"), None);
        assert_eq!(engine.expected_players("unknown"), None);
    }

    #[test]
    fn expected_players_is_cleared_when_every_transfer_fails() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 1);
        let actions = engine.tick(t0);
        let (_, transfer_id) = transfer_ids(&actions)[0];
        engine.ack_transfer(t0, transfer_id, false);
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        assert_eq!(engine.expected_players("towerwars-1"), None);
        assert_eq!(
            transfer_targets(&engine.tick(t0)),
            [(uid(10), "towerwars-1")]
        );
    }

    #[test]
    fn a_server_that_goes_away_forgets_its_expected_size() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        tower_join(&mut engine, t0, 10, 1);
        engine.tick(t0);
        engine.remove_server("towerwars-1");
        assert_eq!(engine.expected_players("towerwars-1"), None);
    }

    #[test]
    fn restored_towerwars_entries_without_a_size_count_as_solo() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        engine.restore_queue(
            t0,
            vec![QueueRecord {
                seq: 5,
                player_uuid: uid(10),
                name: "p10".to_owned(),
                joined_at: chrono::Utc::now(),
                state: QueueEntryState::Waiting,
                game: GameKind::TowerWars,
                match_size: 0,
            }],
        );
        tower_heartbeat(&mut engine, t0, "towerwars-1", ServerPhase::Waiting, &[]);
        assert_eq!(
            transfer_targets(&engine.tick(t0)),
            [(uid(10), "towerwars-1")]
        );
    }

    #[test]
    fn towerwars_needs_one_server_per_pending_match() {
        let mut engine = tower_engine();
        let t0 = Instant::now();
        for n in 10..14 {
            tower_join(&mut engine, t0, n, 1);
        }
        assert_eq!(engine.tick(t0).servers_to_create, 4);
    }
}
