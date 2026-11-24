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
    HeartbeatRequest, HeartbeatResponse, LevelProgress, MatchOutcome, MatchReport, MatchReportAck,
    PhaseUpdateRequest, PlayerReward, SendToLobbyRequest, ServerPhase,
};
use ferriscraft_towerwars::domain::{GameMode, MobKind, TowerKind};
use ferriscraft_towerwars::orchestrator::{GAME_KIND, SERVER_KIND};
use ferriscraft_towerwars::{TowerWarsConfig, TowerWarsPlugin};
use rspigot_core::admin::{AdminLists, ListEntry};
use rspigot_core::config::{GameConfig, LevelData};
use rspigot_core::events::{ClientEvent, Hand, Inbound, PlayerInput};
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

    fn opened_screen(&mut self, title: &str) -> bool {
        self.drain_into_log().iter().any(|event| {
            matches!(event, ClientEvent::OpenScreen { title: shown, .. } if shown.plain_text().contains(title))
        })
    }

    fn last_teleport_y(&mut self) -> Option<f64> {
        self.drain_into_log()
            .iter()
            .rev()
            .find_map(|event| match event {
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
        admin: AdminLists {
            operators: vec![ListEntry::named("Dave").operator(4)],
            ..AdminLists::default()
        },
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
    spectate_for: Vec<Uuid>,
    queue_requests: Vec<Uuid>,
    expected_players: Option<u32>,
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
                    match guard.expected_players {
                        Some(expected_players) => Json(HeartbeatResponse {
                            expected_players: Some(expected_players),
                        })
                        .into_response(),
                        None => StatusCode::NO_CONTENT.into_response(),
                    }
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
                        status["spectate_server"] = json!("towerwars-e2e");
                    }
                    Json(status).into_response()
                },
            ),
        )
        .with_state(state)
}

const LOBBY_Y: f64 = 101.0;
const ANCHOR_Y: f64 = 80.0;
const NAMES: [&str; 4] = ["Alice", "Bob", "Carol", "Dave"];

struct Env {
    runtime: tokio::runtime::Runtime,
    orch: SharedOrchestrator,
    core: Core,
    handle: ferriscraft_towerwars::ShutdownHandle,
    task: tokio::task::JoinHandle<Result<(), rspigot_plugin::Error>>,
    _stop: watch::Sender<bool>,
}

#[derive(Clone, Copy)]
struct Setup {
    mode: GameMode,
    ephemeral: bool,
    countdown_ticks: u32,
    end_delay_ticks: u32,
    expected_players: Option<u32>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            mode: GameMode::Solo,
            ephemeral: true,
            countdown_ticks: 8,
            end_delay_ticks: 6,
            expected_players: Some(4),
        }
    }
}

fn start(spectators: &[&str]) -> Env {
    start_with(spectators, Setup::default())
}

fn start_with(spectators: &[&str], setup: Setup) -> Env {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let host_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let host_addr = host_listener.local_addr().unwrap();
    let (inbound_tx, inbox_rx) = crossbeam_channel::unbounded::<Inbound<Link>>();
    let (stop_tx, stop_rx) = watch::channel(false);
    runtime.spawn(rspigot_plugins::serve(
        host_listener,
        None,
        inbound_tx,
        stop_rx,
    ));

    let orch: SharedOrchestrator = Arc::new(Mutex::new(MockOrchestrator {
        expected_players: setup.expected_players,
        ..MockOrchestrator::default()
    }));
    for name in spectators {
        orch.lock()
            .unwrap()
            .spectate_for
            .push(PlayerId::offline(name).0);
    }
    let orch_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let orch_addr = orch_listener.local_addr().unwrap();
    let orch_router = mock_orchestrator_router(Arc::clone(&orch));
    runtime.spawn(async move {
        let _ = axum::serve(orch_listener, orch_router).await;
    });

    let config = TowerWarsConfig {
        server_id: "towerwars-e2e".to_owned(),
        orchestrator_url: Some(format!("http://{orch_addr}")),
        orchestrator_token: Some(EXPECTED_TOKEN.to_owned()),
        heartbeat_interval: Duration::from_millis(500),
        ephemeral_pod: setup.ephemeral,
        mode: setup.mode,
        countdown_ticks: setup.countdown_ticks,
        end_delay_ticks: setup.end_delay_ticks,
        seed: Some(5),
        decor: false,
        ..TowerWarsConfig::default()
    };
    let plugin = TowerWarsPlugin::new(config);
    let handle = plugin.shutdown_handle();
    let task = runtime.spawn(rspigot_plugin::run_with(
        plugin,
        format!("http://{host_addr}"),
        None,
    ));
    Env {
        runtime,
        orch,
        core: Core::new(inbox_rx),
        handle,
        task,
        _stop: stop_tx,
    }
}

fn pump(core: &mut Core, players: &mut [TestPlayer]) {
    for player in players.iter_mut() {
        confirm_teleports(&mut core.server, player);
    }
}

fn command(core: &mut Core, player: &TestPlayer, text: &str) {
    core.server.handle(Inbound::Input {
        player: player.id,
        input: PlayerInput::Command(text.to_owned()),
    });
    core.server.deliver();
}

fn at(player: &mut TestPlayer, y: f64) -> bool {
    player
        .last_teleport_y()
        .is_some_and(|found| (found - y).abs() < 1.0)
}

fn domain_id(player: &TestPlayer) -> ferriscraft_towerwars::domain::PlayerId {
    ferriscraft_towerwars::domain::PlayerId(player.id.0)
}

#[test]
fn towerwars_full_lifecycle_orchestrator_countdown_play_forfeit_and_report() {
    let mut env = start(&[]);
    let orch = Arc::clone(&env.orch);
    env.core
        .run_until("towerwars plugin connects and enters Waiting", |c| {
            if c.plugin.is_none() {
                return false;
            }
            let guard = orch.lock().unwrap();
            guard
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Starting)
                && guard.heartbeats.iter().any(|hb| {
                    hb.phase == ServerPhase::Waiting
                        && hb.max_players == 6
                        && hb.kind == SERVER_KIND
                })
        });

    let mut players: Vec<TestPlayer> = NAMES
        .iter()
        .map(|name| join(&mut env.core.server, name))
        .collect();
    let uuids: Vec<Uuid> = players.iter().map(|player| player.id.0).collect();

    env.core.run_until(
        "four players reach the lobby and start the countdown",
        |c| {
            pump(c, &mut players);
            let countdown = orch.lock().unwrap().heartbeats.iter().any(|hb| {
                hb.phase == ServerPhase::Countdown
                    && hb.player_count == 4
                    && uuids.iter().all(|uuid| hb.player_uuids.contains(uuid))
            });
            countdown && players.iter_mut().all(|player| at(player, LOBBY_Y))
        },
    );

    env.core.run_until(
        "the match starts and the players hover over their lane",
        |c| {
            pump(c, &mut players);
            let in_game = orch
                .lock()
                .unwrap()
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::InGame);
            in_game && players.iter_mut().all(|player| at(player, ANCHOR_Y))
        },
    );

    command(&mut env.core, &players[0], "towerwars join");
    command(&mut env.core, &players[0], "towerwars version");
    command(&mut env.core, &players[1], "towerwars forcestart");
    command(&mut env.core, &players[2], "towerwars nonsense");
    env.core.run_until("the commands are answered", |c| {
        pump(c, &mut players);
        players[0].said("déjà dans la partie")
            && players[0].said("TowerWars")
            && players[1].said("Vous ne pouvez pas faire ça")
            && players[2].said("Action inconnue")
    });

    let game = env.handle.game().expect("game is ready");
    let alice = domain_id(&players[0]);
    let gold = |game: &ferriscraft_towerwars::game::Game, id| {
        game.with_match(|m| m.gold(id).map(|gold| gold.get()))
            .flatten()
    };
    assert_eq!(gold(&game, alice), Some(100));
    assert_eq!(
        game.place_tower(alice, TowerKind::Archer, (200, 200)),
        Err("§cOn ne peut pas construire ici.")
    );
    game.place_tower(alice, TowerKind::Archer, (2, 4))
        .expect("tower");
    assert_eq!(gold(&game, alice), Some(50));
    assert_eq!(
        game.place_tower(alice, TowerKind::Archer, (2, 4)),
        Err("§cCette case est déjà occupée.")
    );
    game.send_mob(alice, MobKind::Silverfish, 1).expect("send");
    assert_eq!(gold(&game, alice), Some(35));
    assert_eq!(
        game.send_mob(alice, MobKind::Silverfish, 11),
        Err("§cPlus de charge pour ce mob, patientez.")
    );
    assert_eq!(
        game.send_mob(alice, MobKind::Spider, 1),
        Err("§cCe tier de mobs n'est pas encore débloqué.")
    );

    env.core
        .run_until("the tower and the three mobs are rendered", |c| {
            pump(c, &mut players);
            game.rendered() == (3, 1)
        });
    assert_eq!(
        game.with_match(|m| m
            .lane(ferriscraft_towerwars::domain::LaneId::ALL[0])
            .towers()
            .len()),
        Some(1)
    );

    env.core.server.handle(Inbound::Input {
        player: players[0].id,
        input: PlayerInput::HeldSlot(1),
    });
    env.core.server.handle(Inbound::Input {
        player: players[0].id,
        input: PlayerInput::UseItem {
            hand: Hand::Main,
            sequence: 1,
            rotation: rspigot_domain::Rotation::default(),
        },
    });
    env.core.server.deliver();
    env.core
        .run_until("right-clicking the send item opens the send menu", |c| {
            pump(c, &mut players);
            players[0].opened_screen("Envoyer des mobs")
        });

    for player in &players[1..] {
        env.core.server.handle(Inbound::Leave {
            player: player.id,
            session: 0,
        });
    }
    env.core.server.deliver();

    env.core.run_until(
        "the last lane standing wins and the report is posted",
        |c| {
            pump(c, &mut players[..1]);
            !orch.lock().unwrap().report_attempts.is_empty()
        },
    );
    let grace = Instant::now() + Duration::from_millis(2500);
    env.core
        .run_until("the end delay elapses while the ack is withheld", |c| {
            pump(c, &mut players[..1]);
            let guard = orch.lock().unwrap();
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
    orch.lock().unwrap().ack_released = true;

    env.core.run_until("alice sees her rewards", |c| {
        pump(c, &mut players[..1]);
        players[0].said("+150 XP TowerWars")
    });
    env.core
        .run_until("alice is sent back to the hub and the pod ends", |_| {
            let guard = orch.lock().unwrap();
            let returned = guard.send_lobby_calls.iter().any(|(uuid, reason)| {
                *uuid == uuids[0] && reason.as_deref() == Some("fin de partie")
            });
            let ending = guard
                .phase_updates
                .iter()
                .any(|(id, phase)| id == "towerwars-e2e" && *phase == ServerPhase::Ending);
            let hb_ending = guard
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Ending);
            returned && ending && hb_ending
        });

    {
        let guard = orch.lock().unwrap();
        assert!(guard.report_attempts.len() >= 2, "the 503 was retried");
        let first = &guard.report_attempts[0];
        assert!(
            guard
                .report_attempts
                .iter()
                .all(|attempt| attempt.match_id == first.match_id)
        );
        assert_eq!(first.game, GAME_KIND);
        assert_eq!(first.server_id, "towerwars-e2e");
        assert!(!first.forced_start);
        assert_eq!(first.participants.len(), 4);
        let outcome_of = |name: &str| {
            first
                .participants
                .iter()
                .find(|p| p.name == name)
                .map(|p| (p.outcome, p.completed))
        };
        assert_eq!(outcome_of("Alice"), Some((MatchOutcome::Win, true)));
        for name in &NAMES[1..] {
            assert_eq!(
                outcome_of(name),
                Some((MatchOutcome::Loss, false)),
                "{name}"
            );
        }
        assert_eq!(guard.unauthorized_requests, 0);
    }
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn a_player_sent_to_spectate_is_not_seated() {
    let mut env = start(&["Watcher"]);
    let orch = Arc::clone(&env.orch);
    env.core
        .run_until("towerwars plugin connects and enters Waiting", |c| {
            c.plugin.is_some()
                && orch
                    .lock()
                    .unwrap()
                    .heartbeats
                    .iter()
                    .any(|hb| hb.phase == ServerPhase::Waiting)
        });
    let mut players = vec![
        join(&mut env.core.server, "Watcher"),
        join(&mut env.core.server, "Bob"),
        join(&mut env.core.server, "Carol"),
        join(&mut env.core.server, "Dave"),
    ];
    env.core
        .run_until("the watcher is told they are spectating", |c| {
            pump(c, &mut players);
            players[0].said("spectateur") && players[0].said("/hub")
        });
    let settle = Instant::now() + Duration::from_secs(2);
    env.core
        .run_until("the spectator never fills the roster", |c| {
            pump(c, &mut players);
            let guard = orch.lock().unwrap();
            assert!(
                guard
                    .heartbeats
                    .iter()
                    .all(|hb| hb.phase != ServerPhase::Countdown),
                "a countdown started although only three players were seated"
            );
            Instant::now() >= settle
        });
    assert!(
        orch.lock()
            .unwrap()
            .queue_requests
            .contains(&PlayerId::offline("Watcher").0)
    );
    assert_eq!(orch.lock().unwrap().unauthorized_requests, 0);
    env.task.abort();
    drop(env.runtime);
}

const ROOMY: u32 = 6_000;

fn wait_for_waiting(env: &mut Env) {
    let orch = Arc::clone(&env.orch);
    env.core
        .run_until("towerwars plugin connects and enters Waiting", |c| {
            c.plugin.is_some()
                && orch
                    .lock()
                    .unwrap()
                    .heartbeats
                    .iter()
                    .any(|hb| hb.phase == ServerPhase::Waiting)
        });
}

fn seat_four(env: &mut Env) -> Vec<TestPlayer> {
    wait_for_waiting(env);
    let orch = Arc::clone(&env.orch);
    let mut players: Vec<TestPlayer> = NAMES
        .iter()
        .map(|name| join(&mut env.core.server, name))
        .collect();
    env.core
        .run_until("the four players start the countdown", |c| {
            pump(c, &mut players);
            let countdown = orch
                .lock()
                .unwrap()
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Countdown && hb.player_count == 4);
            countdown && players.iter_mut().all(|player| at(player, LOBBY_Y))
        });
    players
}

fn forced_match(env: &mut Env) -> Vec<TestPlayer> {
    let mut players = seat_four(env);
    let orch = Arc::clone(&env.orch);
    command(&mut env.core, &players[3], "towerwars forcestart");
    env.core.run_until("the forced match starts", |c| {
        pump(c, &mut players);
        let in_game = orch
            .lock()
            .unwrap()
            .heartbeats
            .iter()
            .any(|hb| hb.phase == ServerPhase::InGame);
        in_game && players.iter_mut().all(|player| at(player, ANCHOR_Y))
    });
    players
}

fn has_left(game: &ferriscraft_towerwars::game::Game, player: &TestPlayer) -> Option<bool> {
    let id = domain_id(player);
    game.with_match(|m| m.world().player(id).map(|state| state.left))
        .flatten()
}

fn last_heartbeat(orch: &SharedOrchestrator) -> Option<HeartbeatRequest> {
    orch.lock().unwrap().heartbeats.last().cloned()
}

const LEFT: &str = "§cVous avez quitté la partie.";

#[test]
fn leaving_forfeits_the_lane_and_the_player_can_no_longer_act() {
    let mut env = start_with(
        &[],
        Setup {
            countdown_ticks: ROOMY,
            end_delay_ticks: ROOMY,
            ..Setup::default()
        },
    );
    let mut players = forced_match(&mut env);
    let game = env.handle.game().expect("game is ready");
    let alice = domain_id(&players[0]);
    let bob = domain_id(&players[1]);

    command(&mut env.core, &players[1], "towerwars leave");
    env.core
        .run_until("bob leaves and is back in the lobby", |c| {
            pump(c, &mut players);
            players[1].said("Vous avez quitté la partie") && at(&mut players[1], LOBBY_Y)
        });

    assert_eq!(has_left(&game, &players[1]), Some(true));
    assert_eq!(has_left(&game, &players[0]), Some(false));
    let gold = |id| {
        game.with_match(|m| m.gold(id).map(|gold| gold.get()))
            .flatten()
    };
    let before = gold(bob);
    assert_eq!(game.place_tower(bob, TowerKind::Archer, (2, 4)), Err(LEFT));
    assert_eq!(game.send_mob(bob, MobKind::Silverfish, 1), Err(LEFT));
    assert_eq!(gold(bob), before);
    game.place_tower(alice, TowerKind::Archer, (2, 4))
        .expect("alice still plays");
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn the_hub_command_sends_the_player_to_the_lobby_and_removes_them_from_the_match() {
    let mut env = start_with(
        &[],
        Setup {
            countdown_ticks: ROOMY,
            end_delay_ticks: ROOMY,
            ..Setup::default()
        },
    );
    let orch = Arc::clone(&env.orch);
    let mut players = forced_match(&mut env);
    let game = env.handle.game().expect("game is ready");
    let carol_uuid = players[2].id.0;

    command(&mut env.core, &players[2], "towerwars hub");
    env.core
        .run_until("the orchestrator is asked to send carol to the hub", |c| {
            pump(c, &mut players);
            orch.lock()
                .unwrap()
                .send_lobby_calls
                .iter()
                .any(|(uuid, _)| *uuid == carol_uuid)
        });
    env.core.run_until("carol is forfeited", |c| {
        pump(c, &mut players);
        has_left(&game, &players[2]) == Some(true)
    });
    assert_eq!(has_left(&game, &players[0]), Some(false));
    assert_eq!(
        game.place_tower(domain_id(&players[2]), TowerKind::Archer, (2, 4)),
        Err(LEFT)
    );

    let carol = players.remove(2);
    env.core.server.handle(Inbound::Leave {
        player: carol.id,
        session: 0,
    });
    env.core.server.deliver();
    env.core
        .run_until("the heartbeat count drops to three", |c| {
            pump(c, &mut players);
            last_heartbeat(&orch)
                .is_some_and(|hb| hb.player_count == 3 && !hb.player_uuids.contains(&carol_uuid))
        });
    assert_eq!(orch.lock().unwrap().unauthorized_requests, 0);
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn an_admin_stop_ends_the_match_and_removes_every_display_entity() {
    let mut env = start_with(
        &[],
        Setup {
            countdown_ticks: ROOMY,
            end_delay_ticks: ROOMY,
            ..Setup::default()
        },
    );
    let orch = Arc::clone(&env.orch);
    let mut players = forced_match(&mut env);
    let game = env.handle.game().expect("game is ready");
    let alice = domain_id(&players[0]);
    game.place_tower(alice, TowerKind::Archer, (2, 4))
        .expect("tower");
    game.send_mob(alice, MobKind::Silverfish, 1).expect("send");
    env.core.run_until("a tower and mobs are rendered", |c| {
        pump(c, &mut players);
        let (mobs, towers) = game.rendered();
        mobs > 0 && towers == 1
    });

    command(&mut env.core, &players[1], "towerwars stop");
    command(&mut env.core, &players[3], "towerwars stop");
    env.core
        .run_until("the match is stopped and nothing is left rendered", |c| {
            pump(c, &mut players);
            game.rendered() == (0, 0)
                && game.with_match(|_| ()).is_none()
                && last_heartbeat(&orch).is_some_and(|hb| hb.phase != ServerPhase::InGame)
        });
    env.core
        .run_until("the non-admin is told they cannot stop", |c| {
            pump(c, &mut players);
            players[1].said("Vous ne pouvez pas faire ça")
        });
    let settle = Instant::now() + Duration::from_secs(1);
    env.core.run_until("nothing reappears after the stop", |c| {
        pump(c, &mut players);
        assert_eq!(game.rendered(), (0, 0));
        Instant::now() >= settle
    });
    assert_eq!(
        last_heartbeat(&orch).map(|hb| hb.phase),
        Some(ServerPhase::Countdown)
    );
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn in_teams_the_teammate_keeps_playing_when_a_partner_leaves() {
    let mut env = start_with(
        &[],
        Setup {
            mode: GameMode::Teams,
            countdown_ticks: ROOMY,
            end_delay_ticks: ROOMY,
            ..Setup::default()
        },
    );
    let orch = Arc::clone(&env.orch);
    let mut players = forced_match(&mut env);
    let game = env.handle.game().expect("game is ready");
    let lane_of = |player: &TestPlayer| {
        let id = domain_id(player);
        game.with_match(|m| m.world().player(id).map(|state| state.lane))
            .flatten()
    };
    let leaver = 1;
    let mate = (0..4)
        .find(|index| *index != leaver && lane_of(&players[*index]) == lane_of(&players[leaver]))
        .expect("a teammate");
    let rival = (0..4)
        .find(|index| lane_of(&players[*index]) != lane_of(&players[leaver]))
        .expect("a rival");

    command(&mut env.core, &players[leaver], "towerwars leave");
    env.core.run_until("the partner leaves", |c| {
        pump(c, &mut players);
        has_left(&game, &players[leaver]) == Some(true)
    });
    let settle = Instant::now() + Duration::from_secs(1);
    env.core.run_until("the match carries on", |c| {
        pump(c, &mut players);
        assert!(
            orch.lock().unwrap().report_attempts.is_empty(),
            "the match ended although a teammate remained"
        );
        Instant::now() >= settle
    });
    assert_eq!(has_left(&game, &players[mate]), Some(false));
    assert_eq!(has_left(&game, &players[rival]), Some(false));
    assert_eq!(
        last_heartbeat(&orch).map(|hb| hb.phase),
        Some(ServerPhase::InGame)
    );
    game.place_tower(domain_id(&players[mate]), TowerKind::Archer, (2, 4))
        .expect("the teammate still builds");
    assert_eq!(
        game.place_tower(domain_id(&players[leaver]), TowerKind::Archer, (3, 1)),
        Err(LEFT)
    );
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn a_solo_expectation_starts_the_match_at_once_with_three_bots() {
    let mut env = start_with(
        &[],
        Setup {
            countdown_ticks: ROOMY,
            expected_players: Some(1),
            ..Setup::default()
        },
    );
    let orch = Arc::clone(&env.orch);
    env.core.run_until("the plugin reports waiting", |c| {
        c.plugin.is_some()
            && orch
                .lock()
                .unwrap()
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::Waiting)
    });
    env.core
        .run_until("the expectation reaches the server", |_| {
            orch.lock().unwrap().heartbeats.len() >= 3
        });
    let mut players = vec![join(&mut env.core.server, "Alice")];
    env.core
        .run_until("the match starts without a countdown", |c| {
            pump(c, &mut players);
            let guard = orch.lock().unwrap();
            assert!(
                guard
                    .heartbeats
                    .iter()
                    .all(|hb| hb.phase != ServerPhase::Countdown)
            );
            guard
                .heartbeats
                .iter()
                .any(|hb| hb.phase == ServerPhase::InGame && hb.player_count == 1)
        });
    let game = env.handle.game().expect("game is ready");
    assert_eq!(game.with_match(|m| m.world().players.len()), Some(4));
    env.task.abort();
    drop(env.runtime);
}

#[test]
fn an_unknown_expectation_never_starts_the_match() {
    let mut env = start_with(
        &[],
        Setup {
            countdown_ticks: 2,
            expected_players: None,
            ..Setup::default()
        },
    );
    let orch = Arc::clone(&env.orch);
    let mut players: Vec<TestPlayer> = NAMES
        .iter()
        .map(|name| join(&mut env.core.server, name))
        .collect();
    let settle = Instant::now() + Duration::from_secs(2);
    env.core.run_until("nothing starts", |c| {
        pump(c, &mut players);
        let guard = orch.lock().unwrap();
        assert!(
            guard.heartbeats.iter().all(|hb| {
                hb.phase != ServerPhase::Countdown && hb.phase != ServerPhase::InGame
            })
        );
        Instant::now() >= settle
    });
    env.task.abort();
    drop(env.runtime);
}
