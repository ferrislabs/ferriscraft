use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use ferriscraft_protocol::{
    GameKind, HeartbeatRequest, LevelProgress, MatchOutcome, MatchReport, MatchReportAck,
    PhaseUpdateRequest, PlayerReward, SendToLobbyRequest, ServerPhase,
};
use ferriscraft_sheepwars::domain::{RespawnRule, RoundRules};
use ferriscraft_sheepwars::{SheepWarsConfig, SheepWarsPlugin};
use rspigot_core::config::{GameConfig, LevelData};
use rspigot_core::events::{ClientEvent, Inbound, PlayerInput};
use rspigot_core::ports::{ChunkGenerator, ChunkStore, ClientLink, EntityStore, StoreError};
use rspigot_core::server::Server;
use rspigot_core::status::StatusBoard;
use rspigot_core::worlds::WorldSpec;
use rspigot_domain::chunk::ChunkColumn;
use rspigot_domain::player::{ClientSettings, GameProfile, PlayerId};
use rspigot_domain::text::Component;
use rspigot_domain::world::Dimension;
use rspigot_domain::{BiomeId, BlockPos, BlockStateId, ChunkPos, Identifier, Vec3};
use rspigot_nbt::Compound;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::watch;
use uuid::Uuid;

const TICK: Duration = Duration::from_millis(50);
const PATIENCE: Duration = Duration::from_secs(20);
const DEFAULT_SPAWN: Vec3 = Vec3::new(0.5, 65.0, 0.5);
const EXPECTED_TOKEN: &str = "e2e-orch-secret";

#[derive(Clone, Default)]
struct Link {
    events: Arc<Mutex<Vec<ClientEvent>>>,
    closed: Arc<AtomicBool>,
}

impl Link {
    fn take(&self) -> Vec<ClientEvent> {
        self.events
            .lock()
            .map(|mut e| std::mem::take(&mut *e))
            .unwrap_or_default()
    }
}

impl ClientLink for Link {
    fn send(&self, event: ClientEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    fn disconnect(&self, _: Component) {
        self.closed.store(true, Ordering::Release);
    }

    fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
}

struct NoStore;

impl ChunkStore for NoStore {
    fn load(&self, _: &Identifier, _: ChunkPos) -> Result<Option<ChunkColumn>, StoreError> {
        Ok(None)
    }
    fn save(&self, _: &Identifier, _: &ChunkColumn) -> Result<(), StoreError> {
        Ok(())
    }
    fn flush(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

impl EntityStore for NoStore {
    fn load_entities(
        &self,
        _: &Identifier,
        _: ChunkPos,
    ) -> Result<Option<Vec<Compound>>, StoreError> {
        Ok(None)
    }
    fn save_entities(&self, _: &Identifier, _: ChunkPos, _: &[Compound]) -> Result<(), StoreError> {
        Ok(())
    }
}

struct VoidPlatform;

impl ChunkGenerator for VoidPlatform {
    fn generate(&self, dimension: &Dimension, pos: ChunkPos) -> ChunkColumn {
        let mut chunk = ChunkColumn::empty(pos, dimension.height, BiomeId(0));
        if pos == ChunkPos::new(0, 0) {
            for x in 0..4 {
                for z in 0..4 {
                    chunk.set_block(BlockPos::new(x, 64, z), BlockStateId(1));
                }
            }
        }
        chunk
    }

    fn spawn_point(&self, _: &Dimension) -> Vec3 {
        DEFAULT_SPAWN
    }
}

struct TestPlayer {
    id: PlayerId,
    link: Link,
    log: Vec<ClientEvent>,
    confirmed_cursor: usize,
    pos: Vec3,
}

impl TestPlayer {
    fn drain_into_log(&mut self) -> &[ClientEvent] {
        self.log.extend(self.link.take());
        &self.log
    }

    fn received_chunk(&mut self, pos: ChunkPos) -> bool {
        self.drain_into_log()
            .iter()
            .any(|event| matches!(event, ClientEvent::Chunk { chunk, .. } if chunk.pos() == pos))
    }

    fn said(&mut self, text: &str) -> bool {
        self.drain_into_log().iter().any(|event| {
            matches!(event, ClientEvent::SystemChat { message, .. } if message.plain_text().contains(text))
        })
    }

    fn event_debug_has(&mut self, needle: &str) -> bool {
        self.drain_into_log()
            .iter()
            .any(|event| format!("{event:?}").contains(needle))
    }

    fn last_teleport_y(&mut self) -> Option<f64> {
        self.drain_into_log().iter().rev().find_map(|event| match event {
            ClientEvent::Teleport { position, .. } => Some(position.y),
            _ => None,
        })
    }
}

fn world_factory(server: &mut Server<Link>, spec: &WorldSpec) -> Result<(), String> {
    server.load_world(spec, Arc::new(VoidPlatform), Arc::new(NoStore));
    Ok(())
}

fn build_server() -> Server<Link> {
    let config = GameConfig {
        view_distance: 4,
        spawn_chunk_radius: 2,
        chunk_workers: 2,
        spawn_protection: 0,
        ..GameConfig::default()
    };
    let mut server = Server::new(
        config,
        LevelData::default(),
        Arc::new(NoStore),
        Arc::new(VoidPlatform),
        StatusBoard::new(20),
    )
    .with_world_factory(world_factory);
    server.wait_for_spawn(Duration::from_secs(10));
    server
}

fn confirm_teleports(server: &mut Server<Link>, player: &mut TestPlayer) {
    player.drain_into_log();
    let mut updates = Vec::new();
    for event in &player.log[player.confirmed_cursor..] {
        if let ClientEvent::Teleport { id, position, .. } = event {
            updates.push((*id, *position));
        }
    }
    player.confirmed_cursor = player.log.len();
    for (id, pos) in updates {
        player.pos = pos;
        server.handle(Inbound::Input {
            player: player.id,
            input: PlayerInput::TeleportConfirm(id),
        });
    }
    server.deliver();
}

fn join(server: &mut Server<Link>, name: &str) -> TestPlayer {
    let link = Link::default();
    let id = PlayerId::offline(name);
    server.handle(Inbound::Join {
        profile: GameProfile {
            id,
            name: name.to_owned(),
            properties: Vec::new(),
        },
        settings: ClientSettings::default(),
        link: link.clone(),
        address: std::net::Ipv4Addr::LOCALHOST.into(),
        session: 0,
    });
    server.deliver();
    server.tick();
    let mut player = TestPlayer {
        id,
        link,
        log: Vec::new(),
        confirmed_cursor: 0,
        pos: DEFAULT_SPAWN,
    };
    assert!(player.received_chunk(DEFAULT_SPAWN.chunk()));
    confirm_teleports(server, &mut player);
    server.handle(Inbound::Input {
        player: id,
        input: PlayerInput::Loaded,
    });
    player
}

struct Core {
    server: Server<Link>,
    inbox: Receiver<Inbound<Link>>,
    plugin: Option<u64>,
    next_tick: Instant,
}

impl Core {
    fn new(inbox: Receiver<Inbound<Link>>) -> Self {
        Self {
            server: build_server(),
            inbox,
            plugin: None,
            next_tick: Instant::now(),
        }
    }

    fn run_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for: {what}");
            if now >= self.next_tick {
                let waiting = self.inbox.len();
                let inbox = self.inbox.clone();
                for msg in inbox.try_iter().take(waiting) {
                    self.handle(msg);
                }
                self.server.tick();
                self.next_tick += TICK;
                continue;
            }
            match self.inbox.recv_timeout(self.next_tick - now) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("plugin host stopped"),
            }
        }
    }

    fn handle(&mut self, msg: Inbound<Link>) {
        match &msg {
            Inbound::PluginConnected { connection, .. } => self.plugin = Some(*connection),
            Inbound::PluginDisconnected { connection } if self.plugin == Some(*connection) => {
                self.plugin = None;
            }
            _ => {}
        }
        self.server.handle(msg);
        self.server.deliver();
    }
}

#[derive(Default)]
struct MockOrchestrator {
    heartbeats: Vec<HeartbeatRequest>,
    phase_updates: Vec<(String, ServerPhase)>,
    send_lobby_calls: Vec<(Uuid, Option<String>)>,
    unauthorized_requests: usize,
    report_attempts: Vec<MatchReport>,
    ack_released: bool,
    profile_requests: Vec<Uuid>,
    spectate_for: Vec<Uuid>,
    queue_requests: Vec<Uuid>,
}

type SharedOrchestrator = Arc<Mutex<MockOrchestrator>>;

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        == Some(EXPECTED_TOKEN)
}

fn progress(level: u32) -> LevelProgress {
    LevelProgress {
        level,
        xp_into_level: 10,
        xp_for_next: 100,
        total_xp: 10,
    }
}

fn ack_for(report: &MatchReport) -> MatchReportAck {
    MatchReportAck {
        match_id: report.match_id,
        counted: true,
        duplicate: false,
        rewards: report
            .participants
            .iter()
            .map(|participant| PlayerReward {
                uuid: participant.uuid,
                xp: if participant.outcome == MatchOutcome::Win {
                    150
                } else {
                    50
                },
                coins: 30,
                game: progress(12),
                game_level_up: participant.outcome == MatchOutcome::Win,
                network: progress(3),
                network_level_up: false,
            })
            .collect(),
    }
}

fn mock_orchestrator_router(state: SharedOrchestrator) -> Router {
    Router::new()
        .route(
            "/api/v1/servers/heartbeat",
            post(
                |State(orch): State<SharedOrchestrator>,
                 headers: HeaderMap,
                 Json(body): Json<HeartbeatRequest>| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.heartbeats.push(body);
                    Json(json!({ "ok": true })).into_response()
                },
            ),
        )
        .route(
            "/api/v1/servers/{server_id}/phase",
            post(
                |State(orch): State<SharedOrchestrator>,
                 AxumPath(server_id): AxumPath<String>,
                 headers: HeaderMap,
                 Json(body): Json<PhaseUpdateRequest>| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.phase_updates.push((server_id, body.phase));
                    Json(json!({ "ok": true })).into_response()
                },
            ),
        )
        .route(
            "/api/v1/players/{uuid}/send-lobby",
            post(
                |State(orch): State<SharedOrchestrator>,
                 AxumPath(uuid): AxumPath<Uuid>,
                 headers: HeaderMap,
                 Json(body): Json<SendToLobbyRequest>| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.send_lobby_calls.push((uuid, body.reason));
                    Json(json!({ "ok": true })).into_response()
                },
            ),
        )
        .route(
            "/api/v1/matches",
            post(
                |State(orch): State<SharedOrchestrator>,
                 headers: HeaderMap,
                 Json(body): Json<MatchReport>| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.report_attempts.push(body.clone());
                    if !guard.ack_released {
                        return (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({ "error": "progression_unavailable" })),
                        )
                            .into_response();
                    }
                    Json(ack_for(&body)).into_response()
                },
            ),
        )
        .route(
            "/api/v1/players/{uuid}/profile",
            get(
                |State(orch): State<SharedOrchestrator>,
                 AxumPath(uuid): AxumPath<Uuid>,
                 headers: HeaderMap| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.profile_requests.push(uuid);
                    let level = json!({
                        "level": 12, "xp_into_level": 0, "xp_for_next": 100, "total_xp": 0
                    });
                    Json(json!({
                        "uuid": uuid,
                        "name": "Player",
                        "rank": "member",
                        "coins": 0,
                        "network": level,
                        "games": [{
                            "game": "sheep_wars", "progress": level,
                            "played": 0, "wins": 0, "losses": 0, "draws": 0
                        }],
                        "unlocked": ["sheep_trail_hearts"],
                        "equipped": { "sheep_trail": "sheep_trail_hearts" }
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/queue/player/{uuid}",
            get(
                |State(orch): State<SharedOrchestrator>,
                 AxumPath(uuid): AxumPath<Uuid>,
                 headers: HeaderMap| async move {
                    let mut guard = orch.lock().unwrap();
                    if !authorized(&headers) {
                        guard.unauthorized_requests += 1;
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    guard.queue_requests.push(uuid);
                    let mut status = json!({
                        "state": "not_queued", "total_queued": 0, "min_players": 2
                    });
                    if guard.spectate_for.contains(&uuid) {
                        status["spectate_server"] = json!("sheepwars-e2e");
                    }
                    Json(status).into_response()
                },
            ),
        )
        .with_state(state)
}

#[test]
fn sheepwars_full_lifecycle_orchestrator_countdown_combat_and_ephemeral_end() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_test_writer()
        .try_init();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let host_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let host_addr = host_listener.local_addr().unwrap();
    let (inbound_tx, inbox_rx) = crossbeam_channel::unbounded::<Inbound<Link>>();
    let (_stop_tx, stop_rx) = watch::channel(false);
    runtime.spawn(rspigot_plugins::serve(
        host_listener,
        None,
        inbound_tx,
        stop_rx,
    ));

    let orch_state: SharedOrchestrator = Arc::new(Mutex::new(MockOrchestrator::default()));
    let orch_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let orch_addr = orch_listener.local_addr().unwrap();
    let orch_router = mock_orchestrator_router(Arc::clone(&orch_state));
    runtime.spawn(async move {
        let _ = axum::serve(orch_listener, orch_router).await;
    });

    let config = SheepWarsConfig {
        server_id: "sheepwars-e2e".to_owned(),
        orchestrator_url: Some(format!("http://{orch_addr}")),
        orchestrator_token: Some(EXPECTED_TOKEN.to_owned()),
        heartbeat_interval: Duration::from_millis(500),
        ephemeral_pod: true,
        arena_path: PathBuf::from("/nonexistent-e2e-arena.json"),
        rules: RoundRules {
            min_players: 2,
            max_players: 4,
            countdown_ticks: 8,
            sheep_interval_ticks: 6,
            respawn: RespawnRule::Lives(1),
        },
        end_delay_ticks: 6,
        version: None,
    };

    let plugin_task = runtime.spawn(rspigot_plugin::run_with(
        SheepWarsPlugin::new(config),
        format!("http://{host_addr}"),
        None,
    ));

    let mut core = Core::new(inbox_rx);
    core.run_until("sheepwars plugin connects and enters Waiting phase", |c| {
        if c.plugin.is_none() {
            return false;
        }
        let guard = orch_state.lock().unwrap();
        guard
            .heartbeats
            .iter()
            .any(|hb| hb.phase == ServerPhase::Starting)
            && guard
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Waiting && hb.max_players == 4)
    });

    // Two players join -> auto-enrolled, teleported to lobby (y=100), countdown starts
    let mut alice = join(&mut core.server, "Alice");
    let mut bob = join(&mut core.server, "Bob");

    core.run_until("both players reach lobby and trigger Countdown", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        let orch_ok = orch_state.lock().unwrap().heartbeats.iter().any(|hb| {
            hb.phase == ServerPhase::Countdown
                && hb.player_count == 2
                && hb.player_uuids.contains(&alice.id.0)
                && hb.player_uuids.contains(&bob.id.0)
        });
        orch_ok
            && alice.last_teleport_y().is_some_and(|y| (y - 100.0).abs() < 1.0)
            && bob.last_teleport_y().is_some_and(|y| (y - 100.0).abs() < 1.0)
    });

    // Wait for countdown to finish -> InGame, teleported down to arena (y=65)
    core.run_until("match starts and transitions orchestrator to InGame", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        let in_game_hb = orch_state
            .lock()
            .unwrap()
            .heartbeats
            .iter()
            .any(|hb| hb.phase == ServerPhase::InGame);
        in_game_hb
            && alice.last_teleport_y().is_some_and(|y| (y - 65.0).abs() < 1.0)
            && bob.last_teleport_y().is_some_and(|y| (y - 65.0).abs() < 1.0)
    });

    // Advance slightly so at least one sheep wool item is distributed
    core.run_until("sheep item distributed during InGame", |_| {
        alice.event_debug_has("Wool") || alice.event_debug_has("Mouton")
    });

    // Teleport Bob below void_fall_y (20.0) -> killed -> 1 life -> eliminated -> Alice wins.
    // A single 55-block move packet is refused by rspigot movement checks since 0.1.0.
    core.server.handle(Inbound::Console(format!(
        "tp Bob {} 10 {}",
        bob.pos.x, bob.pos.z
    )));
    core.run_until("Bob is teleported below the arena", |c| {
        confirm_teleports(&mut c.server, &mut bob);
        bob.pos.y < 20.0
    });
    core.server.handle(Inbound::Input {
        player: bob.id,
        input: PlayerInput::Move {
            position: Some(Vec3::new(bob.pos.x, bob.pos.y - 0.5, bob.pos.z)),
            rotation: None,
            on_ground: false,
        },
    });
    core.server.deliver();

    core.run_until("Bob falls into void and Alice wins", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        alice.said("Victoire de l'équipe Rouge") && alice.event_debug_has("TitleText")
    });

    core.run_until("the round report reaches the orchestrator", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        !orch_state.lock().unwrap().report_attempts.is_empty()
    });
    let grace = Instant::now() + Duration::from_millis(2500);
    core.run_until("the end delay elapses while the ack is withheld", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        let guard = orch_state.lock().unwrap();
        assert!(
            guard.phase_updates.is_empty() && guard.send_lobby_calls.is_empty(),
            "server went Ending before the report was acknowledged"
        );
        assert!(
            guard
                .heartbeats
                .iter()
                .all(|hb| hb.phase != ServerPhase::Ending)
        );
        Instant::now() >= grace
    });
    orch_state.lock().unwrap().ack_released = true;

    core.run_until("alice sees her rewards", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        alice.said("+150 XP SheepWars") && bob.said("+50 XP SheepWars")
    });

    // Celebration ends -> ReturnAllToLobby + Ending phase update
    core.run_until("end delay triggers send-lobby for both players and Ending phase", |_| {
        let guard = orch_state.lock().unwrap();
        let alice_returned = guard
            .send_lobby_calls
            .iter()
            .any(|(uuid, reason)| *uuid == alice.id.0 && reason.as_deref() == Some("fin de partie"));
        let bob_returned = guard
            .send_lobby_calls
            .iter()
            .any(|(uuid, reason)| *uuid == bob.id.0 && reason.as_deref() == Some("fin de partie"));
        let phase_ending = guard
            .phase_updates
            .iter()
            .any(|(id, phase)| id == "sheepwars-e2e" && *phase == ServerPhase::Ending);
        let hb_ending = guard
            .heartbeats
            .iter()
            .any(|hb| hb.phase == ServerPhase::Ending);
        alice_returned && bob_returned && phase_ending && hb_ending
    });

    {
        let guard = orch_state.lock().unwrap();
        assert!(guard.report_attempts.len() >= 2, "the 503 was retried");
        let first = &guard.report_attempts[0];
        assert!(
            guard
                .report_attempts
                .iter()
                .all(|attempt| attempt.match_id == first.match_id)
        );
        assert_eq!(first.game, GameKind::SheepWars);
        assert_eq!(first.server_id, "sheepwars-e2e");
        assert!(!first.forced_start);
        assert_eq!(first.participants.len(), 2);
        let outcome_of = |name: &str| {
            first
                .participants
                .iter()
                .find(|p| p.name == name)
                .map(|p| (p.outcome, p.completed))
        };
        assert_eq!(outcome_of("Alice"), Some((MatchOutcome::Win, true)));
        assert_eq!(outcome_of("Bob"), Some((MatchOutcome::Loss, true)));
        assert!(guard.profile_requests.contains(&alice.id.0));
        assert!(guard.profile_requests.contains(&bob.id.0));
    }
    assert!(alice.said("Level 12 SheepWars") || alice.event_debug_has("Level 12 SheepWars"));
    assert_eq!(orch_state.lock().unwrap().unauthorized_requests, 0);
    plugin_task.abort();
}

#[test]
fn a_player_sent_to_spectate_is_not_enrolled_in_a_team() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_test_writer()
        .try_init();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let host_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let host_addr = host_listener.local_addr().unwrap();
    let (inbound_tx, inbox_rx) = crossbeam_channel::unbounded::<Inbound<Link>>();
    let (_stop_tx, stop_rx) = watch::channel(false);
    runtime.spawn(rspigot_plugins::serve(
        host_listener,
        None,
        inbound_tx,
        stop_rx,
    ));

    let orch_state: SharedOrchestrator = Arc::new(Mutex::new(MockOrchestrator::default()));
    orch_state
        .lock()
        .unwrap()
        .spectate_for
        .push(PlayerId::offline("Watcher").0);
    let orch_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let orch_addr = orch_listener.local_addr().unwrap();
    let orch_router = mock_orchestrator_router(Arc::clone(&orch_state));
    runtime.spawn(async move {
        let _ = axum::serve(orch_listener, orch_router).await;
    });

    let config = SheepWarsConfig {
        server_id: "sheepwars-e2e".to_owned(),
        orchestrator_url: Some(format!("http://{orch_addr}")),
        orchestrator_token: Some(EXPECTED_TOKEN.to_owned()),
        heartbeat_interval: Duration::from_millis(500),
        ephemeral_pod: true,
        arena_path: PathBuf::from("/nonexistent-e2e-arena.json"),
        rules: RoundRules {
            min_players: 2,
            max_players: 4,
            countdown_ticks: 8,
            sheep_interval_ticks: 6,
            respawn: RespawnRule::Lives(1),
        },
        end_delay_ticks: 6,
        version: None,
    };
    let plugin_task = runtime.spawn(rspigot_plugin::run_with(
        SheepWarsPlugin::new(config),
        format!("http://{host_addr}"),
        None,
    ));

    let mut core = Core::new(inbox_rx);
    core.run_until("sheepwars plugin connects and enters Waiting phase", |c| {
        c.plugin.is_some()
            && orch_state
                .lock()
                .unwrap()
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Waiting)
    });

    let mut watcher = join(&mut core.server, "Watcher");
    let mut bob = join(&mut core.server, "Bob");
    core.run_until("the watcher is told they are spectating", |c| {
        confirm_teleports(&mut c.server, &mut watcher);
        confirm_teleports(&mut c.server, &mut bob);
        watcher.said("spectateur") && watcher.said("/hub")
    });
    let settle = Instant::now() + Duration::from_secs(2);
    core.run_until("the spectator never fills the queue of the round", |c| {
        confirm_teleports(&mut c.server, &mut watcher);
        confirm_teleports(&mut c.server, &mut bob);
        let guard = orch_state.lock().unwrap();
        assert!(
            guard
                .heartbeats
                .iter()
                .all(|hb| hb.phase != ServerPhase::Countdown),
            "a countdown started although only one player joined a team"
        );
        Instant::now() >= settle
    });
    assert!(
        orch_state
            .lock()
            .unwrap()
            .queue_requests
            .contains(&watcher.id.0)
    );
    assert_eq!(orch_state.lock().unwrap().unauthorized_requests, 0);
    plugin_task.abort();
}
