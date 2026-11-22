use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use ferriscraft_protocol::{
    FleetStatusSnapshot, HeartbeatRequest, JoinQueueRequest, JoinQueueResponse, LeaveQueueResponse,
    LeaveReason, PlayerQueueState, PlayerQueueStatus, ProxyEvent, ProxyServerEntry,
    ProxyStateSnapshot, QueueJoinStatus, QueuedPlayerSummary, ServerPhase, ServerSummary,
    TransferCommand,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("serveur inconnu : {0}")]
    UnknownServer(String),
    #[error("serveur non joignable : {0}")]
    NotJoinable(String),
}

#[derive(Debug, Clone)]
pub struct FleetPolicy {
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
}

#[derive(Debug, Clone)]
struct ActiveTransfer {
    command: TransferCommand,
    created_at: Instant,
    is_lobby_return: bool,
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

    fn summary(&self, now: Instant) -> ServerSummary {
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
    deleting: HashSet<String>,
    queue: VecDeque<QueuedPlayer>,
    transfers: HashMap<Uuid, ActiveTransfer>,
    player_transfers: HashMap<Uuid, Uuid>,
}

impl FleetEngine {
    pub fn new(policy: FleetPolicy) -> Self {
        Self {
            policy,
            servers: BTreeMap::new(),
            deleting: HashSet::new(),
            queue: VecDeque::new(),
            transfers: HashMap::new(),
            player_transfers: HashMap::new(),
        }
    }

    pub fn policy(&self) -> &FleetPolicy {
        &self.policy
    }

    pub fn join_queue(&mut self, now: Instant, req: JoinQueueRequest) -> JoinQueueResponse {
        if let Some(transfer_id) = self.player_transfers.get(&req.player_uuid)
            && let Some(active) = self.transfers.get(transfer_id)
            && !active.is_lobby_return
        {
            return self.build_join_response(
                QueueJoinStatus::Transferring,
                0,
                Some(active.command.target_server.clone()),
            );
        }

        if let Some(pos) = self.queue_position(req.player_uuid) {
            return self.build_join_response(QueueJoinStatus::AlreadyQueued, pos, None);
        }

        self.queue.push_back(QueuedPlayer {
            uuid: req.player_uuid,
            name: req.player_name,
            joined_at: now,
        });
        let pos = self.queue.len();
        self.build_join_response(QueueJoinStatus::Queued, pos, None)
    }

    pub fn leave_queue(&mut self, player_uuid: Uuid, reason: LeaveReason) -> LeaveQueueResponse {
        let mut removed = false;
        if let Some(idx) = self
            .queue
            .iter()
            .position(|entry| entry.uuid == player_uuid)
        {
            self.queue.remove(idx);
            removed = true;
        }

        if reason == LeaveReason::ProxyDisconnect
            && let Some(transfer_id) = self.player_transfers.remove(&player_uuid)
        {
            self.transfers.remove(&transfer_id);
        }

        LeaveQueueResponse {
            removed,
            total_queued: self.queue.len(),
        }
    }

    pub fn player_status(&self, player_uuid: Uuid) -> PlayerQueueStatus {
        if let Some(transfer_id) = self.player_transfers.get(&player_uuid)
            && let Some(active) = self.transfers.get(transfer_id)
            && !active.is_lobby_return
        {
            return PlayerQueueStatus {
                state: PlayerQueueState::Transferring,
                position: None,
                total_queued: self.queue.len(),
                min_players: self.policy.queue_min_players,
                target_server: Some(active.command.target_server.clone()),
            };
        }

        if let Some(pos) = self.queue_position(player_uuid) {
            PlayerQueueStatus {
                state: PlayerQueueState::Queued,
                position: Some(pos),
                total_queued: self.queue.len(),
                min_players: self.policy.queue_min_players,
                target_server: None,
            }
        } else {
            PlayerQueueStatus {
                state: PlayerQueueState::NotQueued,
                position: None,
                total_queued: self.queue.len(),
                min_players: self.policy.queue_min_players,
                target_server: None,
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

        Self::sync_proxy_registration(entry, addr_changed, &mut events);
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

    pub fn mark_outdated(&mut self, server_id: &str) {
        if let Some(server) = self.servers.get_mut(server_id) {
            server.outdated = true;
        }
    }

    pub fn mark_deleting(&mut self, now: Instant, server_id: &str) -> Option<ProxyEvent> {
        self.deleting.insert(server_id.to_owned());
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

    pub fn apply_heartbeat(&mut self, now: Instant, req: HeartbeatRequest) -> Vec<ProxyEvent> {
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

        // Clear matching active transfers once the game server reports the player connected.
        for uuid in &req.player_uuids {
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

        if let Some(server) = self.servers.get_mut(&req.server_id) {
            Self::sync_proxy_registration(server, addr_changed || phase_changed, &mut events);
        }
        events
    }

    pub fn update_server_phase(
        &mut self,
        now: Instant,
        server_id: &str,
        phase: ServerPhase,
    ) -> Result<Vec<ProxyEvent>, EngineError> {
        let Some(server) = self.servers.get_mut(server_id) else {
            return Err(EngineError::UnknownServer(server_id.to_owned()));
        };
        let mut events = Vec::new();
        let changed = server.set_phase(now, phase);
        server.has_plugin_heartbeat = true;
        server.last_heartbeat = Some(now);
        Self::sync_proxy_registration(server, changed, &mut events);
        Ok(events)
    }

    pub fn queue_lobby_transfer(
        &mut self,
        now: Instant,
        player_uuid: Uuid,
        reason: Option<String>,
    ) -> ProxyEvent {
        if let Some(idx) = self.queue.iter().position(|e| e.uuid == player_uuid) {
            self.queue.remove(idx);
        }
        if let Some(old_tid) = self.player_transfers.remove(&player_uuid) {
            self.transfers.remove(&old_tid);
        }

        let transfer_id = Uuid::new_v4();
        let reason = reason.unwrap_or_else(|| "retour au hub".to_owned());
        let command = TransferCommand {
            transfer_id,
            player_uuid,
            target_server: self.policy.lobby_server_name.clone(),
            reason: reason.clone(),
        };
        self.transfers.insert(
            transfer_id,
            ActiveTransfer {
                command: command.clone(),
                created_at: now,
                is_lobby_return: true,
            },
        );
        self.player_transfers.insert(player_uuid, transfer_id);

        ProxyEvent::TransferPlayer {
            transfer_id,
            player_uuid,
            target_server: command.target_server,
            reason,
        }
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
        if self.deleting.contains(server_id) || !server.ready_for_proxy() {
            return Err(EngineError::NotJoinable(server_id.to_owned()));
        }
        if let Some(idx) = self.queue.iter().position(|e| e.uuid == player_uuid) {
            self.queue.remove(idx);
        }
        if let Some(old_tid) = self.player_transfers.remove(&player_uuid) {
            self.transfers.remove(&old_tid);
        }
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

    pub fn ack_transfer(&mut self, transfer_id: Uuid, _success: bool) {
        if let Some(active) = self.transfers.remove(&transfer_id)
            && self.player_transfers.get(&active.command.player_uuid) == Some(&transfer_id)
        {
            self.player_transfers.remove(&active.command.player_uuid);
        }
    }

    pub fn tick(&mut self, now: Instant) -> TickActions {
        let mut actions = TickActions::default();

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
        }

        // 2. Dispatch queued players onto available SheepWars servers.
        actions
            .proxy_events
            .extend(self.dispatch_queued_players(now));

        // 3. Plan scaling actions (create up to k max, delete Ending/stuck/excess idle servers).
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

    fn dispatch_queued_players(&mut self, now: Instant) -> Vec<ProxyEvent> {
        let mut events = Vec::new();
        if self.queue.is_empty() {
            return events;
        }

        loop {
            if self.queue.is_empty() {
                break;
            }

            let pending_by_server = self.pending_transfer_counts();

            // Pick a joinable server: prefer already-populated Waiting/Countdown servers first
            // to fill matches quickly without splitting players across empty servers.
            let mut candidates: Vec<(bool, bool, u32, u32, String)> = self
                .servers
                .values()
                .filter(|s| {
                    !self.deleting.contains(&s.server_id)
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
            if !has_players && self.queue.len() < self.policy.queue_min_players {
                break;
            }

            let dispatch_count = self.queue.len().min(free_slots as usize);
            if dispatch_count == 0 {
                break;
            }

            for _ in 0..dispatch_count {
                let Some(player) = self.queue.pop_front() else {
                    break;
                };
                if let Some(old_tid) = self.player_transfers.remove(&player.uuid) {
                    self.transfers.remove(&old_tid);
                }
                let transfer_id = Uuid::new_v4();
                let reason = "sheepwars_matchmaking".to_owned();
                let command = TransferCommand {
                    transfer_id,
                    player_uuid: player.uuid,
                    target_server: target_server.clone(),
                    reason: reason.clone(),
                };
                self.transfers.insert(
                    transfer_id,
                    ActiveTransfer {
                        command,
                        created_at: now,
                        is_lobby_return: false,
                    },
                );
                self.player_transfers.insert(player.uuid, transfer_id);
                events.push(ProxyEvent::TransferPlayer {
                    transfer_id,
                    player_uuid: player.uuid,
                    target_server: target_server.clone(),
                    reason,
                });
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
                && !self.deleting.contains(&s.server_id)
                && s.phase == ServerPhase::Waiting
                && s.ready_for_proxy()
        });
        let mut retiring = HashSet::new();

        for s in self.servers.values() {
            if self.deleting.contains(&s.server_id) {
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
                !self.deleting.contains(&s.server_id)
                    && !delete_set.contains(s.server_id.as_str())
                    && !retiring.contains(s.server_id.as_str())
                    && s.phase.is_active()
            })
            .count();

        let available_standby = self
            .servers
            .values()
            .filter(|s| {
                !self.deleting.contains(&s.server_id)
                    && !delete_set.contains(s.server_id.as_str())
                    && !s.outdated
                    && s.phase.is_standby_capacity()
            })
            .count();

        let needed_for_queue = if self.queue.is_empty() {
            0
        } else {
            self.queue
                .len()
                .div_ceil(self.policy.max_players_per_server as usize)
                .max(1)
        };

        let target_available = if self.queue.is_empty() {
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
                    !self.deleting.contains(&s.server_id)
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
            if self.deleting.contains(&s.server_id) {
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
            servers.push(s.summary(now));
        }

        let queued_players = self
            .queue
            .iter()
            .enumerate()
            .map(|(idx, q)| QueuedPlayerSummary {
                player_uuid: q.uuid,
                player_name: q.name.clone(),
                position: idx + 1,
                queued_for_secs: now.saturating_duration_since(q.joined_at).as_secs(),
            })
            .collect();

        FleetStatusSnapshot {
            queue_len: self.queue.len(),
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
        }
    }

    pub fn proxy_snapshot(&self) -> ProxyStateSnapshot {
        let servers = self
            .servers
            .values()
            .filter(|s| !self.deleting.contains(&s.server_id) && s.ready_for_proxy())
            .filter_map(|s| {
                s.address.as_ref().map(|addr| ProxyServerEntry {
                    name: s.server_id.clone(),
                    address: addr.clone(),
                    phase: s.phase,
                })
            })
            .collect();

        let pending_transfers = self.transfers.values().map(|t| t.command.clone()).collect();

        ProxyStateSnapshot {
            lobby_server_name: self.policy.lobby_server_name.clone(),
            servers,
            pending_transfers,
        }
    }

    fn sync_proxy_registration(
        server: &mut ManagedServer,
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
            .iter()
            .position(|entry| entry.uuid == uuid)
            .map(|idx| idx + 1)
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
    ) -> JoinQueueResponse {
        let ready_servers = self
            .servers
            .values()
            .filter(|s| {
                !self.deleting.contains(&s.server_id)
                    && s.ready_for_proxy()
                    && s.phase.accepts_players()
            })
            .count();
        let active_servers = self
            .servers
            .values()
            .filter(|s| !self.deleting.contains(&s.server_id) && s.phase.is_active())
            .count();

        JoinQueueResponse {
            status,
            position,
            total_queued: self.queue.len(),
            min_players: self.policy.queue_min_players,
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
            },
        );
        engine.join_queue(
            t0,
            JoinQueueRequest {
                player_uuid: p2,
                player_name: "Bob".to_owned(),
            },
        );
        engine.tick(t0);

        assert_eq!(
            engine.player_status(p1).state,
            PlayerQueueState::Transferring
        );
        engine.leave_queue(p1, LeaveReason::LobbyQuit);
        assert_eq!(
            engine.player_status(p1).state,
            PlayerQueueState::Transferring
        );

        engine.leave_queue(p1, LeaveReason::ProxyDisconnect);
        assert_eq!(engine.player_status(p1).state, PlayerQueueState::NotQueued);
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
}
