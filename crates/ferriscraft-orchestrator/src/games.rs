use std::collections::{BTreeMap, HashSet};
use std::iter::once;
use std::time::Instant;

use ferriscraft_protocol::{
    FleetStatusSnapshot, GameFleetStatus, GameKind, HeartbeatRequest, JoinQueueRequest,
    JoinQueueResponse, LeaveQueueResponse, LeaveReason, PlayerQueueStatus, ProxyEvent,
    ProxyStateSnapshot, QueueJoinStatus, ServerPhase,
};
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{EngineError, FleetEngine};
use crate::lobby::{DrainError, DrainResult};
use crate::queue::QueueMutation;

#[derive(Debug, Error)]
pub enum GameError {
    #[error("jeu inactif : {}", .0.slug())]
    Inactive(GameKind),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

pub struct Games {
    host: FleetEngine,
    others: BTreeMap<GameKind, FleetEngine>,
}

impl Games {
    pub fn new(host: FleetEngine) -> Self {
        Self {
            host,
            others: BTreeMap::new(),
        }
    }

    pub fn insert_engine(&mut self, engine: FleetEngine) {
        let game = engine.policy().game;
        if game == self.host.policy().game {
            self.host = engine;
        } else {
            self.others.insert(game, engine);
        }
    }

    pub fn lobby_host(&self) -> &FleetEngine {
        &self.host
    }

    pub fn lobby_host_mut(&mut self) -> &mut FleetEngine {
        &mut self.host
    }

    pub fn engine(&self, game: GameKind) -> Result<&FleetEngine, GameError> {
        self.iter()
            .find(|engine| engine.policy().game == game)
            .ok_or(GameError::Inactive(game))
    }

    pub fn engine_mut(&mut self, game: GameKind) -> Result<&mut FleetEngine, GameError> {
        if self.host.policy().game == game {
            return Ok(&mut self.host);
        }
        self.others.get_mut(&game).ok_or(GameError::Inactive(game))
    }

    pub fn iter(&self) -> impl Iterator<Item = &FleetEngine> {
        once(&self.host).chain(self.others.values())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut FleetEngine> {
        once(&mut self.host).chain(self.others.values_mut())
    }

    fn holder(&self, player_uuid: Uuid) -> Option<GameKind> {
        self.iter()
            .find(|engine| engine.holds_player(player_uuid))
            .map(|engine| engine.policy().game)
    }

    pub fn take_mutations(&mut self) -> Vec<QueueMutation> {
        self.iter_mut()
            .flat_map(FleetEngine::take_mutations)
            .collect()
    }

    pub fn restore_mutations(&mut self, unpersisted: Vec<QueueMutation>) {
        self.host.restore_mutations(unpersisted);
    }

    pub fn needs_enqueue(&self, game: GameKind, player_uuid: Uuid) -> Result<bool, GameError> {
        self.engine(game)?;
        Ok(self.holder(player_uuid).is_none())
    }

    pub fn join(
        &mut self,
        now: Instant,
        req: JoinQueueRequest,
        seq: Option<u64>,
    ) -> Result<(JoinQueueResponse, Vec<ProxyEvent>), GameError> {
        self.engine(req.game)?;
        let player_uuid = req.player_uuid;
        let game = self.holder(player_uuid).unwrap_or(req.game);
        let engine = self.engine_mut(game)?;
        let resp = match seq {
            Some(seq) => engine.join_queue_with_seq(now, req, seq),
            None => engine.join_queue(now, req),
        };
        let events = engine.dispatch(now);
        let snap = engine.snapshot(now);
        let post_status = engine.player_status(now, player_uuid);
        let mut updated = resp;
        updated.total_queued = post_status.total_queued;
        updated.ready_servers = snap.ready_servers;
        updated.active_servers = snap.ready_servers + snap.in_game_servers + snap.starting_servers;
        if let Some(target) = post_status.target_server {
            updated.status = QueueJoinStatus::Transferring;
            updated.position = 0;
            updated.target_server = Some(target);
        } else if let Some(pos) = post_status.position {
            updated.position = pos;
        }
        Ok((updated, events))
    }

    pub fn leave_queue(
        &mut self,
        game: GameKind,
        player_uuid: Uuid,
        reason: LeaveReason,
    ) -> Result<LeaveQueueResponse, GameError> {
        Ok(self.engine_mut(game)?.leave_queue(player_uuid, reason))
    }

    pub fn disconnect(&mut self, player_uuid: Uuid) -> LeaveQueueResponse {
        let mut resp = self
            .host
            .leave_queue(player_uuid, LeaveReason::ProxyDisconnect);
        for engine in self.others.values_mut() {
            let other = engine.leave_queue(player_uuid, LeaveReason::ProxyDisconnect);
            resp.removed |= other.removed;
        }
        resp
    }

    pub fn player_status(
        &self,
        now: Instant,
        player_uuid: Uuid,
        game: GameKind,
    ) -> Result<PlayerQueueStatus, GameError> {
        self.engine(game)?;
        let spectate_server = self
            .iter()
            .find_map(|engine| engine.player_status(now, player_uuid).spectate_server);
        let game = self.holder(player_uuid).unwrap_or(game);
        let mut status = self.engine(game)?.player_status(now, player_uuid);
        status.spectate_server = spectate_server;
        Ok(status)
    }

    pub fn apply_heartbeat(
        &mut self,
        now: Instant,
        req: HeartbeatRequest,
    ) -> Result<Vec<ProxyEvent>, GameError> {
        let game = req.kind.game().unwrap_or(self.host.policy().game);
        let engine = self.engine_mut(game)?;
        let mut events = engine.apply_heartbeat(now, req);
        events.extend(engine.dispatch(now));
        Ok(events)
    }

    pub fn expected_players(&self, server_id: &str) -> Option<u32> {
        self.iter()
            .find_map(|engine| engine.expected_players(server_id))
    }

    pub fn update_server_phase(
        &mut self,
        now: Instant,
        server_id: &str,
        phase: ServerPhase,
    ) -> Result<Vec<ProxyEvent>, EngineError> {
        let engine = self
            .iter_mut()
            .find(|engine| engine.has_server(server_id))
            .ok_or_else(|| EngineError::UnknownServer(server_id.to_owned()))?;
        let mut events = engine.update_server_phase(now, server_id, phase)?;
        events.extend(engine.dispatch(now));
        Ok(events)
    }

    pub fn drain_lobby(&mut self, now: Instant, id: &str) -> Result<DrainResult, DrainError> {
        match self.host.drain_lobby(now, id) {
            Err(DrainError::UnknownServer)
                if self.others.values().any(|engine| engine.has_server(id)) =>
            {
                Err(DrainError::NotALobby)
            }
            result => result,
        }
    }

    pub fn queue_spectate_transfer(
        &mut self,
        now: Instant,
        server_id: &str,
        player_uuid: Uuid,
    ) -> Result<ProxyEvent, EngineError> {
        let owner = self
            .iter()
            .find(|engine| engine.has_server(server_id))
            .map(|engine| engine.policy().game)
            .ok_or_else(|| EngineError::UnknownServer(server_id.to_owned()))?;
        let event = self
            .iter_mut()
            .find(|engine| engine.policy().game == owner)
            .ok_or_else(|| EngineError::UnknownServer(server_id.to_owned()))?
            .queue_spectate_transfer(now, server_id, player_uuid)?;
        for engine in self
            .iter_mut()
            .filter(|engine| engine.policy().game != owner)
        {
            engine.release_player(player_uuid);
        }
        Ok(event)
    }

    pub fn queue_lobby_transfer(
        &mut self,
        now: Instant,
        player_uuid: Uuid,
        reason: Option<String>,
    ) -> ProxyEvent {
        for engine in self.others.values_mut() {
            engine.release_player(player_uuid);
        }
        self.host.queue_lobby_transfer(now, player_uuid, reason)
    }

    pub fn ack_transfer(&mut self, now: Instant, transfer_id: Uuid, success: bool) {
        for engine in self.iter_mut() {
            engine.ack_transfer(now, transfer_id, success);
        }
    }

    pub fn seen_players(&self, server_id: &str) -> HashSet<Uuid> {
        self.iter()
            .flat_map(|engine| engine.seen_players(server_id))
            .collect()
    }

    pub fn snapshot(&self, now: Instant) -> FleetStatusSnapshot {
        let mut merged = self.host.snapshot(now);
        let mut games = vec![game_status(self.host.policy().game, &merged)];
        for engine in self.others.values() {
            let snap = engine.snapshot(now);
            games.push(game_status(engine.policy().game, &snap));
            merged.servers.extend(snap.servers);
        }
        merged.games = games;
        merged
    }

    pub fn proxy_snapshot(&self) -> ProxyStateSnapshot {
        let mut merged = self.host.proxy_snapshot();
        for engine in self.others.values() {
            let snap = engine.proxy_snapshot();
            merged.servers.extend(snap.servers);
            merged.pending_transfers.extend(snap.pending_transfers);
        }
        merged
    }
}

fn game_status(game: GameKind, snap: &FleetStatusSnapshot) -> GameFleetStatus {
    GameFleetStatus {
        game,
        queue_len: snap.queue_len,
        min_players: snap.min_players,
        max_players_per_server: snap.max_players_per_server,
        min_replicas: snap.min_replicas,
        max_replicas: snap.max_replicas,
        standby_servers: snap.standby_servers,
        ready_servers: snap.ready_servers,
        in_game_servers: snap.in_game_servers,
        starting_servers: snap.starting_servers,
        queued_players: snap.queued_players.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ferriscraft_protocol::{PlayerQueueState, ServerKind};

    use super::*;
    use crate::engine::FleetPolicy;

    fn policy(game: GameKind, min: usize, max: usize, standby: usize) -> FleetPolicy {
        FleetPolicy {
            game,
            lobby_server_name: "lobby".to_owned(),
            queue_min_players: 2,
            max_players_per_server: 4,
            min_replicas: min,
            max_replicas: max,
            standby_servers: standby,
            idle_ttl: Duration::from_secs(60),
            starting_timeout: Duration::from_secs(120),
            transfer_timeout: Duration::from_secs(15),
            empty_grace: Duration::from_secs(30),
            ending_timeout: Duration::from_secs(60),
            heartbeat_timeout: Duration::from_secs(30),
        }
    }

    fn both() -> Games {
        let mut games = Games::new(FleetEngine::new(policy(GameKind::SheepWars, 1, 2, 1)));
        games.insert_engine(FleetEngine::new(policy(GameKind::TowerWars, 0, 3, 2)));
        games
    }

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn join_request(n: u128, game: GameKind) -> JoinQueueRequest {
        JoinQueueRequest {
            player_uuid: uid(n),
            player_name: format!("p{n}"),
            game,
            match_size: 0,
        }
    }

    fn duo_request(n: u128) -> JoinQueueRequest {
        JoinQueueRequest {
            match_size: 2,
            ..join_request(n, GameKind::TowerWars)
        }
    }

    fn beat(kind: ServerKind, id: &str, players: &[u128]) -> HeartbeatRequest {
        HeartbeatRequest {
            server_id: id.to_owned(),
            address: Some("10.0.0.1:25565".to_owned()),
            kind,
            phase: ServerPhase::Waiting,
            player_count: players.len() as u32,
            max_players: 4,
            player_uuids: players.iter().map(|n| uid(*n)).collect(),
            version: None,
        }
    }

    fn transfer_target(events: &[ProxyEvent], player: Uuid) -> Option<&str> {
        events.iter().find_map(|event| match event {
            ProxyEvent::TransferPlayer {
                player_uuid,
                target_server,
                ..
            } if *player_uuid == player => Some(target_server.as_str()),
            _ => None,
        })
    }

    #[test]
    fn spec_tw_26_a_towerwars_entry_goes_to_a_towerwars_server_only() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::SheepWars, "sheepwars-1", &[]))
            .unwrap();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();

        let (first, _) = games.join(now, duo_request(1), None).unwrap();
        assert_eq!(first.status, QueueJoinStatus::Queued);
        let (second, events) = games.join(now, duo_request(2), None).unwrap();
        assert_eq!(second.status, QueueJoinStatus::Transferring);
        assert_eq!(second.target_server.as_deref(), Some("towerwars-1"));
        assert_eq!(transfer_target(&events, uid(2)), Some("towerwars-1"));
        let sheep = games.engine(GameKind::SheepWars).unwrap();
        assert!(!sheep.holds_player(uid(1)));
        assert!(!sheep.holds_player(uid(2)));
    }

    #[test]
    fn spec_tw_26_a_sheepwars_entry_goes_to_a_sheepwars_server_only() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();
        games
            .apply_heartbeat(now, beat(ServerKind::SheepWars, "sheepwars-1", &[]))
            .unwrap();

        games
            .join(now, join_request(1, GameKind::SheepWars), None)
            .unwrap();
        let (resp, events) = games
            .join(now, join_request(2, GameKind::SheepWars), None)
            .unwrap();
        assert_eq!(resp.target_server.as_deref(), Some("sheepwars-1"));
        assert_eq!(transfer_target(&events, uid(1)), Some("sheepwars-1"));
        assert!(
            !games
                .engine(GameKind::TowerWars)
                .unwrap()
                .holds_player(uid(1))
        );
    }

    #[test]
    fn spec_tw_26_a_full_towerwars_fleet_does_not_spill_into_sheepwars() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::SheepWars, "sheepwars-1", &[]))
            .unwrap();
        for n in 1..=3 {
            let (resp, events) = games
                .join(now, join_request(n, GameKind::TowerWars), None)
                .unwrap();
            assert_eq!(resp.status, QueueJoinStatus::Queued, "player {n}");
            assert!(events.is_empty());
        }
        let status = games
            .player_status(now, uid(1), GameKind::TowerWars)
            .unwrap();
        assert_eq!(status.state, PlayerQueueState::Queued);
        assert_eq!(status.total_queued, 3);
    }

    #[test]
    fn spec_tw_26_a_player_cannot_be_in_both_queues() {
        let mut games = both();
        let now = Instant::now();
        games
            .join(now, join_request(1, GameKind::SheepWars), None)
            .unwrap();
        assert!(!games.needs_enqueue(GameKind::TowerWars, uid(1)).unwrap());
        let (resp, _) = games
            .join(now, join_request(1, GameKind::TowerWars), None)
            .unwrap();
        assert_eq!(resp.status, QueueJoinStatus::AlreadyQueued);
        assert_eq!(resp.position, 1);
        assert!(
            !games
                .engine(GameKind::TowerWars)
                .unwrap()
                .holds_player(uid(1))
        );

        let reverse = both_with_tower_entry();
        assert!(!reverse.needs_enqueue(GameKind::SheepWars, uid(1)).unwrap());
    }

    fn both_with_tower_entry() -> Games {
        let mut games = both();
        games
            .join(Instant::now(), join_request(1, GameKind::TowerWars), None)
            .unwrap();
        games
    }

    #[test]
    fn a_player_queued_for_towerwars_joining_sheepwars_is_already_queued() {
        let mut games = both_with_tower_entry();
        let (resp, _) = games
            .join(Instant::now(), join_request(1, GameKind::SheepWars), None)
            .unwrap();
        assert_eq!(resp.status, QueueJoinStatus::AlreadyQueued);
        assert!(
            !games
                .engine(GameKind::SheepWars)
                .unwrap()
                .holds_player(uid(1))
        );
    }

    #[test]
    fn leaving_one_queue_frees_the_player_for_the_other() {
        let mut games = both_with_tower_entry();
        let left = games
            .leave_queue(GameKind::TowerWars, uid(1), LeaveReason::UserToggle)
            .unwrap();
        assert!(left.removed);
        assert!(games.needs_enqueue(GameKind::SheepWars, uid(1)).unwrap());
    }

    #[test]
    fn disconnect_removes_the_player_from_every_queue() {
        let mut games = both_with_tower_entry();
        assert!(games.disconnect(uid(1)).removed);
        assert!(games.needs_enqueue(GameKind::TowerWars, uid(1)).unwrap());
    }

    #[test]
    fn spec_tw_26_heartbeats_are_routed_by_kind() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::SheepWars, "sheepwars-1", &[]))
            .unwrap();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();
        games
            .apply_heartbeat(now, beat(ServerKind::Lobby, "lobby-1", &[]))
            .unwrap();

        let sheep = games.engine(GameKind::SheepWars).unwrap();
        let tower = games.engine(GameKind::TowerWars).unwrap();
        assert!(sheep.has_server("sheepwars-1"));
        assert!(!sheep.has_server("towerwars-1"));
        assert!(tower.has_server("towerwars-1"));
        assert!(!tower.has_server("sheepwars-1"));
        assert_eq!(sheep.snapshot(now).lobbies.len(), 1);
        assert!(tower.snapshot(now).lobbies.is_empty());
    }

    #[test]
    fn a_heartbeat_for_an_inactive_game_is_rejected() {
        let mut games = Games::new(FleetEngine::new(policy(GameKind::SheepWars, 1, 2, 1)));
        let err = games
            .apply_heartbeat(
                Instant::now(),
                beat(ServerKind::TowerWars, "towerwars-1", &[]),
            )
            .unwrap_err();
        assert!(matches!(err, GameError::Inactive(GameKind::TowerWars)));
        assert!(
            games
                .join(Instant::now(), join_request(1, GameKind::TowerWars), None)
                .is_err()
        );
    }

    #[test]
    fn spec_tw_26_fleet_planning_is_independent_per_game() {
        let mut games = both();
        let now = Instant::now();
        let idle_sheep = games.engine_mut(GameKind::SheepWars).unwrap().tick(now);
        let idle_tower = games.engine_mut(GameKind::TowerWars).unwrap().tick(now);
        assert_eq!(idle_sheep.servers_to_create, 1);
        assert_eq!(idle_tower.servers_to_create, 0);

        games
            .join(now, join_request(1, GameKind::TowerWars), None)
            .unwrap();
        let busy_tower = games.engine_mut(GameKind::TowerWars).unwrap().tick(now);
        let busy_sheep = games.engine_mut(GameKind::SheepWars).unwrap().tick(now);
        assert_eq!(busy_tower.servers_to_create, 2);
        assert_eq!(busy_sheep.servers_to_create, 1);

        let tower = games.engine_mut(GameKind::TowerWars).unwrap();
        for id in ["towerwars-a", "towerwars-b", "towerwars-c"] {
            tower.register_creating_server(now, id.to_owned(), Some(id.to_owned()));
        }
        assert_eq!(tower.tick(now).servers_to_create, 0);
        let sheep = games.engine_mut(GameKind::SheepWars).unwrap();
        sheep.register_creating_server(now, "sheepwars-a".to_owned(), None);
        assert_eq!(sheep.tick(now).servers_to_create, 0);
    }

    #[test]
    fn snapshots_merge_every_game() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::SheepWars, "sheepwars-1", &[]))
            .unwrap();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();
        games.join(now, duo_request(7), None).unwrap();

        let snap = games.snapshot(now);
        let kinds: Vec<ServerKind> = snap.servers.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&ServerKind::SheepWars));
        assert!(kinds.contains(&ServerKind::TowerWars));
        assert_eq!(snap.max_replicas, 2);
        assert_eq!(snap.games.len(), 2);
        let tower = snap
            .games
            .iter()
            .find(|g| g.game == GameKind::TowerWars)
            .unwrap();
        assert_eq!((tower.max_replicas, tower.queue_len), (3, 1));

        let proxy = games.proxy_snapshot();
        let names: Vec<&str> = proxy.servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"sheepwars-1") && names.contains(&"towerwars-1"));
        let tower_entry = proxy
            .servers
            .iter()
            .find(|s| s.name == "towerwars-1")
            .unwrap();
        assert_eq!(tower_entry.kind, ServerKind::TowerWars);
    }

    #[test]
    fn phase_updates_find_the_owning_engine() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();
        games
            .update_server_phase(now, "towerwars-1", ServerPhase::InGame)
            .unwrap();
        assert!(matches!(
            games.update_server_phase(now, "nope", ServerPhase::InGame),
            Err(EngineError::UnknownServer(_))
        ));
    }

    #[test]
    fn draining_a_game_server_is_not_a_lobby_drain() {
        let mut games = both();
        let now = Instant::now();
        games
            .apply_heartbeat(now, beat(ServerKind::TowerWars, "towerwars-1", &[]))
            .unwrap();
        assert_eq!(
            games.drain_lobby(now, "towerwars-1").unwrap_err(),
            DrainError::NotALobby
        );
        assert_eq!(
            games.drain_lobby(now, "ghost").unwrap_err(),
            DrainError::UnknownServer
        );
    }

    #[test]
    fn returning_to_the_lobby_clears_a_towerwars_queue_entry() {
        let mut games = both_with_tower_entry();
        games.queue_lobby_transfer(Instant::now(), uid(1), None);
        assert!(games.needs_enqueue(GameKind::TowerWars, uid(1)).unwrap());
        assert!(
            !games
                .engine(GameKind::TowerWars)
                .unwrap()
                .holds_player(uid(1))
        );
    }

    #[test]
    fn mutations_of_every_engine_are_collected_with_their_game() {
        let mut games = both();
        let now = Instant::now();
        games
            .join(now, join_request(1, GameKind::SheepWars), None)
            .unwrap();
        games
            .join(now, join_request(2, GameKind::TowerWars), None)
            .unwrap();
        let games_seen: Vec<GameKind> = games
            .take_mutations()
            .into_iter()
            .filter_map(|m| match m {
                QueueMutation::Insert { game, .. } => Some(game),
                _ => None,
            })
            .collect();
        assert_eq!(games_seen, [GameKind::SheepWars, GameKind::TowerWars]);
        assert!(games.take_mutations().is_empty());
    }

    #[test]
    fn requeued_players_of_two_games_never_share_a_sequence_number() {
        let mut games = both();
        let now = Instant::now();
        for (engine_game, server, players) in [
            (GameKind::SheepWars, "sheepwars-1", [1u128, 2]),
            (GameKind::TowerWars, "towerwars-1", [3u128, 4]),
        ] {
            let engine = games.engine_mut(engine_game).unwrap();
            let kind = engine_game.server_kind();
            engine.apply_heartbeat(now, beat(kind, server, &players));
            engine.mark_deleting(now, server);
        }
        let seqs: Vec<u64> = games
            .take_mutations()
            .into_iter()
            .filter_map(|m| match m {
                QueueMutation::Insert { seq, .. } => Some(seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs.len(), 4);
        let unique: HashSet<u64> = seqs.iter().copied().collect();
        assert_eq!(unique.len(), 4, "{seqs:?}");
    }
}
