use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ferriscraft_protocol::{HeartbeatRequest, HeartbeatResponse, MatchReportAck, ServerPhase};
use futures_util::future::join_all;
use rspigot_plugin::{
    Attribute, Billboard, BlockDisplay, BlockState, BossBar, ChunkLoad, CustomInventory,
    DisplayProps, DisplaySpec, Effect, EffectType, Entity, EntityType, Error, GameMode,
    InteractionBox, InventorySlot, ItemStack, Location, Material, Particle, ParticleBurst, Player,
    Position, Rotation, Server, SlotItem, SoundCue, TextDisplay, Title, Transformation, Uuid,
    Vector,
};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::aim::surface_cell;
use super::arena::{Arena, Point};
use super::intent::Intent;
use super::menus::{Menu, MenuAction, build_menu, lane_menu, send_count, send_menu, tower_menu};
use super::messages::message;
use super::names::{next_priority, tower_name};
use super::notices::{Audience, notices, victory_subtitle};
use super::render::{
    MIN_AIM_CHANGE, MobOp, MobSnapshot, MobTrack, RING_POINTS, Shot, TowerOp, death_shot,
    diff_mobs, leak_shot, mob_entity, mob_scale, range_color, ring_points, shots, snapshots,
    tower_ops, yaw_differs,
};
use super::report::{
    DeliveryError, MatchSummary, build_report, deliver, not_counted_notice, retry_schedule,
    reward_display,
};
use super::session::{ForceStartError, Session, Stage};
use super::sounds::{Hearing, REFUSED, cues as sound_cues};
use super::tower_model::{Part, model_height, tower_model};
use crate::config::TowerWarsConfig;
use crate::domain::{LaneId, Level, MobId, MobKind, PlayerId, TowerId, TowerKind, Vec3};
use crate::hud::{
    ACTION_BAR_REFRESH_TICKS, Hud, action_bar, bossbar_progress, bossbar_title, countdown_title,
    lane_panel, lobby_lines, sidebar_lines, waiting_title,
};
use crate::orchestrator::{SERVER_KIND, TowerWarsOrchestratorClient, spectates_here};
use crate::sim::{Event, Match};

pub const BUILD_ITEM: Material = Material::Stick;
pub const SEND_ITEM: Material = Material::Emerald;
pub const TELEPORT_ITEM: Material = Material::Compass;
pub const HUB_ITEM: Material = Material::Book;
const BUILD_SLOT: u8 = 0;
const SEND_SLOT: u8 = 1;
const TELEPORT_SLOT: u8 = 4;
const HUB_SLOT: u8 = 8;
const SLOW_TICK: Duration = Duration::from_millis(40);
const VIEW_PITCH: f32 = 60.0;
const CELEBRATION_TICKS: u32 = 100;
const CELEBRATION_PULSE_TICKS: u32 = 5;
const HEARTBEAT_FAILURE_LOG_EVERY: Duration = Duration::from_secs(30);
pub const SHUTDOWN_REPORT_GRACE: Duration = Duration::from_secs(5);
const SHUTDOWN_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Build,
    Send,
    Teleport,
    Hub,
}

pub fn tool_of(material: Material) -> Option<Tool> {
    match material {
        BUILD_ITEM => Some(Tool::Build),
        SEND_ITEM => Some(Tool::Send),
        TELEPORT_ITEM => Some(Tool::Teleport),
        HUB_ITEM => Some(Tool::Hub),
        _ => None,
    }
}

pub fn player_id(uuid: Uuid) -> PlayerId {
    PlayerId(uuid)
}

fn phase_for(shutting_down: bool, current: ServerPhase) -> ServerPhase {
    if shutting_down {
        ServerPhase::Ending
    } else {
        current
    }
}

async fn wait_until_idle(in_flight: &AtomicUsize, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    while in_flight.load(Ordering::Acquire) > 0 {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(SHUTDOWN_POLL).await;
    }
    true
}

const fn position(vec: Vec3) -> Position {
    Position {
        x: vec.x as f64,
        y: vec.y as f64,
        z: vec.z as f64,
    }
}

fn point_position(point: Point) -> Position {
    Position {
        x: point.x,
        y: point.y,
        z: point.z,
    }
}

const fn display_kind(spec: &DisplaySpec) -> EntityType {
    match spec {
        DisplaySpec::Text(_) => EntityType::TextDisplay,
        DisplaySpec::Item(_) => EntityType::ItemDisplay,
        DisplaySpec::Block(_) => EntityType::BlockDisplay,
        DisplaySpec::Interaction(_) => EntityType::Interaction,
    }
}

fn item_stack(item: &super::menus::MenuItem) -> ItemStack {
    ItemStack::new(item.material, item.count)
        .name(item.name.as_str())
        .lore(item.lore.iter().map(String::as_str))
}

fn fill(inventory: &CustomInventory, menu: &Menu) {
    inventory.clear();
    for item in &menu.items {
        inventory.set(item.slot, Some(item_stack(item)));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    Build,
    Send,
    Tower(TowerId),
    Lanes,
}

struct OpenMenu {
    player: PlayerId,
    kind: MenuKind,
    menu: Menu,
    inventory: CustomInventory,
}

const PANEL_SCALE: f32 = 2.2;
const PANEL_BACKGROUND: i32 = 0x6000_0000;

struct MobEntry {
    track: MobTrack,
    entity: Entity,
    label: Entity,
}

struct TowerEntry {
    parts: Vec<(Entity, Part)>,
    hitbox: Entity,
    kind: TowerKind,
    at: Vec3,
    yaw: f32,
}

impl TowerEntry {
    fn remove(self) {
        for (entity, _) in self.parts {
            entity.remove();
        }
        self.hitbox.remove();
    }

    fn aim(&mut self, yaw: f32) {
        self.yaw = yaw;
        for (entity, part) in self.parts.iter().filter(|(_, part)| part.turret) {
            entity.display.edit(
                BlockDisplay::default().props(
                    DisplayProps::default()
                        .transformation(part.transformation(yaw))
                        .interpolation(0, 3),
                ),
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Generation(u64);

#[derive(Debug, Default)]
struct Epoch(u64);

impl Epoch {
    const fn current(&self) -> Generation {
        Generation(self.0)
    }

    const fn advance(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }

    fn admits(&self, captured: Generation) -> bool {
        self.current() == captured
    }
}

#[derive(Debug, Default)]
struct PlayerView {
    viewing: Option<LaneId>,
    selected: Option<TowerKind>,
}

#[derive(Debug, Clone, Copy)]
struct Celebration {
    lane: Option<LaneId>,
    age: u32,
}

enum Background {
    Report {
        match_id: Uuid,
        result: Result<MatchReportAck, DeliveryError>,
    },
}

struct State {
    session: Session,
    players: HashMap<PlayerId, Player>,
    names: HashMap<PlayerId, String>,
    views: HashMap<PlayerId, PlayerView>,
    mobs: HashMap<MobId, MobEntry>,
    towers: HashMap<TowerId, TowerEntry>,
    hitboxes: HashMap<i32, TowerId>,
    menus: HashMap<u64, OpenMenu>,
    celebration: Option<Celebration>,
    hud: Hud,
    bar: BossBar,
    ticks: u64,
    epoch: Epoch,
    last_reported_phase: Option<ServerPhase>,
    panels: HashMap<LaneId, Entity>,
}

impl State {
    fn name_of(&self, id: PlayerId) -> Option<String> {
        self.names.get(&id).cloned().or_else(|| {
            let lane = self
                .session
                .lane_of(id)
                .filter(|_| self.session.is_bot(id))?;
            Some(format!("Bot {}", lane.index() + 1))
        })
    }

    fn observed_lanes(&self) -> Vec<LaneId> {
        if self.players.is_empty() {
            return Vec::new();
        }
        self.session.game().map_or_else(Vec::new, |game| {
            game.world()
                .lanes
                .iter()
                .filter(|lane| !lane.owners.is_empty() && !lane.eliminated)
                .map(|lane| lane.id)
                .collect()
        })
    }

    fn menu_for(&self, id: PlayerId, kind: MenuKind) -> Option<Menu> {
        let game = self.session.game()?;
        match kind {
            MenuKind::Build => Some(build_menu(
                game.gold(id)?.get(),
                self.views.get(&id).and_then(|view| view.selected),
            )),
            MenuKind::Send => send_menu(game, id),
            MenuKind::Tower(tower) => tower_menu(game, id, tower),
            MenuKind::Lanes => Some(lane_menu(
                game,
                id,
                &|other| self.name_of(other),
                &|other| self.session.is_bot(other),
            )),
        }
    }
}

pub struct Game {
    arena: Arena,
    server: Server,
    config: TowerWarsConfig,
    orchestrator: Option<Arc<TowerWarsOrchestratorClient>>,
    heartbeat_notify: Arc<Notify>,
    background_tx: mpsc::UnboundedSender<Background>,
    background_rx: Mutex<mpsc::UnboundedReceiver<Background>>,
    state: Mutex<State>,
    shutting_down: AtomicBool,
    reports_in_flight: Arc<AtomicUsize>,
}

impl Game {
    pub fn new(
        arena: Arena,
        server: Server,
        session: Session,
        hud: Hud,
        bar: BossBar,
        config: TowerWarsConfig,
        orchestrator: Option<Arc<TowerWarsOrchestratorClient>>,
    ) -> Self {
        let (background_tx, background_rx) = mpsc::unbounded_channel();
        Self {
            arena,
            server,
            config,
            orchestrator,
            heartbeat_notify: Arc::new(Notify::new()),
            background_tx,
            background_rx: Mutex::new(background_rx),
            shutting_down: AtomicBool::new(false),
            reports_in_flight: Arc::new(AtomicUsize::new(0)),
            state: Mutex::new(State {
                session,
                players: HashMap::new(),
                names: HashMap::new(),
                views: HashMap::new(),
                mobs: HashMap::new(),
                towers: HashMap::new(),
                hitboxes: HashMap::new(),
                menus: HashMap::new(),
                celebration: None,
                hud,
                bar,
                ticks: 0,
                epoch: Epoch::default(),
                last_reported_phase: None,
                panels: HashMap::new(),
            }),
        }
    }

    pub fn spawn_heartbeat_loop(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        let orchestrator = self.orchestrator.clone()?;
        let game = Arc::clone(self);
        let interval_dur = self.config.heartbeat_interval;
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval_dur);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut last_failure_log: Option<Instant> = None;
            loop {
                let payload = game.heartbeat_payload();
                match orchestrator.send_heartbeat(&payload).await {
                    Ok(response) => {
                        game.apply_heartbeat_response(response).await;
                        if last_failure_log.take().is_some() {
                            tracing::info!(
                                server_id = %payload.server_id,
                                "Heartbeat orchestrateur rétabli"
                            );
                        }
                    }
                    Err(err) => {
                        let now = Instant::now();
                        if last_failure_log
                            .is_none_or(|at| now.duration_since(at) >= HEARTBEAT_FAILURE_LOG_EVERY)
                        {
                            last_failure_log = Some(now);
                            tracing::warn!(
                                server_id = %payload.server_id,
                                phase = ?payload.phase,
                                players = payload.player_count,
                                "Échec envoi heartbeat orchestrateur : {err}"
                            );
                        }
                    }
                }
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = game.heartbeat_notify.notified() => {}
                }
            }
        }))
    }

    async fn apply_heartbeat_response(&self, response: Option<HeartbeatResponse>) {
        let Some(expected) = response.and_then(|response| response.expected_players) else {
            return;
        };
        let intents = self.lock().session.set_expected(Some(expected));
        self.run(intents).await;
        self.check_phase_changed();
    }

    pub fn heartbeat_payload(&self) -> HeartbeatRequest {
        let state = self.lock();
        let player_uuids: Vec<Uuid> = state.players.values().map(|p| p.uuid).collect();
        let player_count = u32::try_from(player_uuids.len()).unwrap_or(u32::MAX);
        let max_players = u32::try_from(self.config.max_players).unwrap_or(u32::MAX);
        HeartbeatRequest {
            server_id: self.config.server_id.clone(),
            address: None,
            phase: phase_for(
                self.shutting_down.load(Ordering::Acquire),
                state.session.server_phase(),
            ),
            player_count,
            max_players,
            player_uuids,
            version: self.config.version.clone(),
            kind: SERVER_KIND,
        }
    }

    pub async fn shutdown(&self, grace: Duration) {
        self.shutting_down.store(true, Ordering::Release);
        if let Some(orchestrator) = &self.orchestrator
            && let Err(error) = orchestrator.send_heartbeat(&self.heartbeat_payload()).await
        {
            tracing::warn!("dernier heartbeat non envoyé : {error}");
        }
        if !wait_until_idle(&self.reports_in_flight, grace).await {
            tracing::warn!("rapport de partie encore en cours à l'arrêt, abandon");
        }
    }

    fn check_phase_changed(&self) {
        let changed = {
            let mut state = self.lock();
            let current = state.session.server_phase();
            if state.last_reported_phase != Some(current) {
                state.last_reported_phase = Some(current);
                true
            } else {
                false
            }
        };
        if changed {
            self.heartbeat_notify.notify_one();
        }
    }

    async fn should_watch(&self, uuid: Uuid) -> bool {
        let Some(orchestrator) = &self.orchestrator else {
            return false;
        };
        match orchestrator.fetch_queue_status(uuid).await {
            Ok(status) => spectates_here(&status, &self.config.server_id),
            Err(error) => {
                tracing::debug!(%uuid, "statut de file indisponible : {error}");
                false
            }
        }
    }

    pub async fn admit(&self, player: Player) {
        let id = player_id(player.uuid);
        let watch = self.should_watch(player.uuid).await;
        let intents = {
            let mut state = self.lock();
            state.bar.add_player(&player);
            state.names.insert(id, player.name.clone());
            state.players.insert(id, player.clone());
            if watch {
                state.session.watch(id)
            } else {
                state.session.join(id)
            }
        };
        self.run(intents).await;
        self.heartbeat_notify.notify_one();
    }

    fn submit_report(&self, summary: &MatchSummary) {
        let Some(orchestrator) = self.orchestrator.clone() else {
            return;
        };
        let report = {
            let state = self.lock();
            build_report(summary, &self.config.server_id, |id| state.name_of(id))
        };
        let background = self.background_tx.clone();
        let in_flight = Arc::clone(&self.reports_in_flight);
        in_flight.fetch_add(1, Ordering::AcqRel);
        tokio::spawn(async move {
            let result = deliver(&retry_schedule(), || {
                orchestrator.post_match_report(&report)
            })
            .await;
            match &result {
                Ok(ack) => tracing::info!(
                    match_id = %report.match_id,
                    counted = ack.counted,
                    duplicate = ack.duplicate,
                    "rapport de partie accepté"
                ),
                Err(DeliveryError::Rejected(error)) => tracing::warn!(
                    match_id = %report.match_id,
                    "rapport de partie refusé définitivement : {error}"
                ),
                Err(DeliveryError::Exhausted(error)) => tracing::error!(
                    match_id = %report.match_id,
                    "rapport de partie non envoyé, budget de renvoi épuisé : {error}"
                ),
            }
            in_flight.fetch_sub(1, Ordering::AcqRel);
            let _ = background.send(Background::Report {
                match_id: report.match_id,
                result,
            });
        });
    }

    fn take_background(&self) -> Vec<Background> {
        let mut inbox = self
            .background_rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut taken = Vec::new();
        while let Ok(item) = inbox.try_recv() {
            taken.push(item);
        }
        taken
    }

    async fn drain_background(&self) {
        for item in self.take_background() {
            match item {
                Background::Report { match_id, result } => {
                    if let Ok(ack) = &result {
                        self.announce(ack);
                    }
                    let intents = self.lock().session.report_settled(match_id);
                    self.run(intents).await;
                }
            }
        }
    }

    fn announce(&self, ack: &MatchReportAck) {
        if !ack.counted {
            for player in self.everyone() {
                player.message.send(not_counted_notice());
            }
            return;
        }
        for reward in &ack.rewards {
            let Some(player) = self.player(player_id(reward.uuid)) else {
                continue;
            };
            let display = reward_display(reward);
            for line in &display.lines {
                player.message.send(line.as_str());
            }
            if let Some(title) = &display.title {
                player.title.send(Title::new(title.as_str()));
            }
        }
    }

    pub async fn restore_arena(&self, prebuilt: bool) -> usize {
        let placed = self.arena.prepare(prebuilt).await;
        tracing::info!(
            placed,
            prebuilt,
            pending_chunks = self.arena.pending_chunks(),
            "arène construite"
        );
        placed
    }

    pub fn on_chunk_load(&self, event: &ChunkLoad) {
        if event.world().id() != self.arena.world.id() {
            return;
        }
        let chunk = event.chunk();
        let placed = self.arena.place_once(chunk.x, chunk.z);
        if placed > 0 {
            tracing::debug!(
                "placé {placed} blocs dans le chunk ({}, {})",
                chunk.x,
                chunk.z
            );
        }
    }

    pub fn is_arena_world(&self, world_id: &str) -> bool {
        self.arena.world.id() == world_id
    }

    pub async fn release(&self, player: &Player) {
        let id = player_id(player.uuid);
        let (intents, stale) = {
            let mut state = self.lock();
            state.bar.remove_player(player);
            let keys: Vec<u64> = state
                .menus
                .iter()
                .filter(|(_, open)| open.player == id)
                .map(|(key, _)| *key)
                .collect();
            let stale: Vec<CustomInventory> = keys
                .into_iter()
                .filter_map(|key| state.menus.remove(&key))
                .map(|open| open.inventory)
                .collect();
            (state.session.leave(id), stale)
        };
        for inventory in stale {
            inventory.discard();
        }
        self.run(intents).await;
        {
            let mut state = self.lock();
            state.players.remove(&id);
            state.views.remove(&id);
        }
        self.heartbeat_notify.notify_one();
        self.check_phase_changed();
    }

    pub async fn join(&self, player: &Player) {
        let id = player_id(player.uuid);
        let intents = {
            let mut state = self.lock();
            state.players.insert(id, player.clone());
            state.names.insert(id, player.name.clone());
            state.session.join(id)
        };
        self.run(intents).await;
        self.check_phase_changed();
    }

    pub async fn quit(&self, player: &Player) {
        let intents = self.lock().session.quit(player_id(player.uuid));
        self.run(intents).await;
        self.check_phase_changed();
    }

    pub async fn send_player_hub(&self, player: &Player) {
        if let Some(orch) = &self.orchestrator {
            player.message.send("§eRetour au Hub FerrisCraft...");
            let intents = self.lock().session.leave(player_id(player.uuid));
            self.run(intents).await;
            self.check_phase_changed();
            if let Err(err) = orch
                .send_player_to_lobby(player.uuid, Some("retour demandé".to_owned()))
                .await
            {
                tracing::warn!(player = %player.name, "send-lobby échoué : {err}");
                player
                    .message
                    .send("§cOrchestrateur indisponible, utilisez §d/hub§c.");
            }
        } else {
            player
                .message
                .send("§eUtilisez §d/hub §epour retourner au Hub FerrisCraft.");
        }
    }

    pub async fn force_start(&self) -> Result<(), ForceStartError> {
        let intents = self.lock().session.force_start()?;
        self.run(intents).await;
        self.check_phase_changed();
        Ok(())
    }

    pub async fn stop(&self) {
        let intents = self.lock().session.stop();
        self.run(intents).await;
        self.check_phase_changed();
    }

    pub fn is_lost(&self, x: f64, y: f64, z: f64) -> bool {
        self.arena.layout.is_void_fall(y) || self.arena.layout.is_outside(x, z)
    }

    pub async fn rescue(&self, player: &Player) {
        let id = player_id(player.uuid);
        let point = {
            let state = self.lock();
            state
                .views
                .get(&id)
                .and_then(|view| view.viewing)
                .and_then(|lane| self.arena.layout.anchor(lane))
                .unwrap_or_else(|| self.arena.layout.lobby())
        };
        self.teleport(player, point).await;
    }

    pub async fn tick(&self) {
        let started = Instant::now();
        self.drain_background().await;
        let tick = self.lock().session.tick();
        self.run(tick.intents).await;
        self.apply_events(&tick.events).await;
        let simulated = started.elapsed();
        self.sync_mobs().await;
        let synced = started.elapsed();
        self.advance_previews().await;
        self.advance_celebration();
        self.refresh();
        self.sync_panels().await;
        self.check_phase_changed();
        let total = started.elapsed();
        if total >= SLOW_TICK {
            tracing::warn!(
                total_ms = total.as_millis() as u64,
                simulation_ms = simulated.as_millis() as u64,
                mobs_ms = (synced - simulated).as_millis() as u64,
                previews_ms = (total - synced).as_millis() as u64,
                "tick lent"
            );
        }
    }

    pub fn close(&self) {
        let mut state = self.lock();
        for (_, entry) in state.mobs.drain() {
            entry.entity.remove();
            entry.label.remove();
        }
        for (_, entry) in state.towers.drain() {
            entry.remove();
        }
        state.hitboxes.clear();
        for (_, panel) in state.panels.drain() {
            panel.remove();
        }
        for (_, open) in state.menus.drain() {
            open.inventory.discard();
        }
        state.celebration = None;
        state.hud.close();
        state.bar.remove();
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn player(&self, id: PlayerId) -> Option<Player> {
        self.lock().players.get(&id).cloned()
    }

    fn everyone(&self) -> Vec<Player> {
        self.lock().players.values().cloned().collect()
    }

    fn location(&self, point: Point) -> Location {
        Location {
            world: self.arena.world.id().to_owned(),
            position: point_position(point),
            rotation: Rotation {
                yaw: 0.0,
                pitch: VIEW_PITCH,
            },
        }
    }

    async fn teleport(&self, player: &Player, point: Point) {
        if let Err(error) = player.location.teleport_to(&self.location(point)).await {
            tracing::warn!("impossible de téléporter {} : {error}", player.name);
        }
    }

    async fn run(&self, intents: Vec<Intent>) {
        let mut queue: VecDeque<Intent> = intents.into();
        while let Some(intent) = queue.pop_front() {
            queue.extend(self.perform(intent).await);
        }
    }

    async fn perform(&self, intent: Intent) -> Vec<Intent> {
        match intent {
            Intent::Broadcast(text) => {
                for player in self.everyone() {
                    player.message.send(text.as_str());
                }
            }
            Intent::Tell { player, text } => {
                if let Some(player) = self.player(player) {
                    player.message.send(text);
                }
            }
            Intent::Enroll { player } => {
                if let Some(player) = self.player(player) {
                    self.enroll(&player);
                }
            }
            Intent::Unenroll { player } => {
                self.lock().views.remove(&player);
            }
            Intent::Lobby { player } => {
                if let Some(player) = self.player(player) {
                    self.teleport(&player, self.arena.layout.lobby()).await;
                }
            }
            Intent::Watch { player } => {
                if let Some(player) = self.player(player) {
                    self.watch(&player).await;
                }
            }
            Intent::Begin { seats } => {
                for seat in seats {
                    if let Some(player) = self.player(seat.player) {
                        self.begin(&player, seat.lane).await;
                    }
                }
            }
            Intent::Won { winner } => self.won(winner),
            Intent::ReturnAllToLobby { reason } => {
                let uuids: Vec<Uuid> = self.everyone().into_iter().map(|p| p.uuid).collect();
                if let Some(orch) = &self.orchestrator {
                    let futures = uuids
                        .into_iter()
                        .map(|uuid| orch.send_player_to_lobby(uuid, Some(reason.clone())));
                    for result in join_all(futures).await {
                        if let Err(err) = result {
                            tracing::warn!("Échec retour au Hub orchestrateur : {err}");
                        }
                    }
                }
            }
            Intent::SendToLobby { player, reason } => {
                if let Some(orch) = &self.orchestrator
                    && let Err(err) = orch.send_player_to_lobby(player.0, Some(reason)).await
                {
                    tracing::warn!(uuid = %player.0, "Échec retour au Hub orchestrateur : {err}");
                }
            }
            Intent::EndServer => {
                if let Some(orch) = &self.orchestrator
                    && let Err(err) = orch
                        .update_phase(&self.config.server_id, ServerPhase::Ending)
                        .await
                {
                    tracing::warn!(
                        server_id = %self.config.server_id,
                        "Échec passage en phase ending : {err}"
                    );
                }
                self.heartbeat_notify.notify_one();
            }
            Intent::SubmitReport(summary) => self.submit_report(&summary),
            Intent::Reset => return self.reset(),
        }
        Vec::new()
    }

    fn enroll(&self, player: &Player) {
        self.lock().views.remove(&player_id(player.uuid));
        player.game_mode.set(GameMode::Adventure);
        player.abilities.set_flying(false);
        player.abilities.set_allow_flight(false);
        player.inventory.clear();
        give_hub_item(player);
        player.health.set(20.0);
        player.food.set_level(20);
        player.food.set_saturation(5.0);
    }

    async fn begin(&self, player: &Player, lane: LaneId) {
        let Some(anchor) = self.arena.layout.anchor(lane) else {
            return;
        };
        self.lock().views.insert(
            player_id(player.uuid),
            PlayerView {
                viewing: Some(lane),
                selected: None,
            },
        );
        player.game_mode.set(GameMode::Adventure);
        player.inventory.clear();
        give_kit(player, true);
        player.health.set(20.0);
        player.food.set_level(20);
        self.teleport(player, anchor).await;
        player.abilities.set_allow_flight(true);
        player.abilities.set_flying(true);
    }

    async fn watch(&self, player: &Player) {
        let id = player_id(player.uuid);
        let lane = {
            let state = self.lock();
            state.session.game().and_then(|game| {
                game.world()
                    .lanes
                    .iter()
                    .find(|lane| {
                        lane.owners
                            .iter()
                            .any(|owner| !state.session.is_bot(*owner))
                    })
                    .map(|lane| lane.id)
            })
        };
        player.game_mode.set(GameMode::Adventure);
        player.inventory.clear();
        let Some(anchor) = lane.and_then(|lane| self.arena.layout.anchor(lane)) else {
            give_hub_item(player);
            self.teleport(player, self.arena.layout.lobby()).await;
            return;
        };
        self.lock().views.insert(
            id,
            PlayerView {
                viewing: lane,
                selected: None,
            },
        );
        give_kit(player, false);
        self.teleport(player, anchor).await;
        player.abilities.set_allow_flight(true);
        player.abilities.set_flying(true);
    }

    fn won(&self, winner: Option<LaneId>) {
        let subtitle = {
            let state = self.lock();
            state.session.game().map_or_else(String::new, |game| {
                victory_subtitle(game, winner, &|id| state.name_of(id))
            })
        };
        let headline = if winner.is_some() {
            "§6Victoire !"
        } else {
            "§eMatch nul"
        };
        for player in self.everyone() {
            player
                .title
                .send(Title::new(headline).subtitle(subtitle.as_str()));
        }
        self.lock().celebration = Some(Celebration {
            lane: winner,
            age: 0,
        });
    }

    fn reset(&self) -> Vec<Intent> {
        let mut state = self.lock();
        state.epoch.advance();
        for (_, entry) in state.mobs.drain() {
            entry.entity.remove();
            entry.label.remove();
        }
        for (_, entry) in state.towers.drain() {
            entry.remove();
        }
        state.hitboxes.clear();
        for (_, panel) in state.panels.drain() {
            panel.remove();
        }
        for (_, open) in state.menus.drain() {
            open.inventory.discard();
        }
        state.views.clear();
        state.celebration = None;
        let connected: Vec<PlayerId> = state.players.keys().copied().collect();
        state.names.retain(|id, _| connected.contains(id));
        let mut ids = connected;
        ids.sort();
        ids.into_iter()
            .flat_map(|id| state.session.join(id))
            .collect()
    }

    fn refresh(&self) {
        let mut state = self.lock();
        state.ticks += 1;
        let ticks = state.ticks;
        if !ticks.is_multiple_of(ACTION_BAR_REFRESH_TICKS) {
            return;
        }
        let State {
            session,
            players,
            names,
            hud,
            bar,
            ..
        } = &mut *state;
        let name_of = |id: PlayerId| names.get(&id).cloned();
        match (session.stage(), session.game()) {
            (_, Some(game)) => {
                hud.render(sidebar_lines(game, &name_of));
                bar.set_title(bossbar_title(game));
                bar.set_progress(bossbar_progress(game));
                for (id, player) in players.iter() {
                    if let Some(line) = action_bar(game, *id) {
                        player.action_bar.send(line);
                    }
                }
            }
            (Stage::Countdown { remaining }, None) => {
                hud.render(lobby_lines(
                    session.roster().len(),
                    session.threshold(),
                    Some(remaining),
                ));
                bar.set_title(countdown_title(remaining));
                bar.set_progress(
                    1.0 - f64::from(remaining) / f64::from(session.rules().countdown_ticks.max(1)),
                );
            }
            _ => {
                let (count, min) = (session.roster().len(), session.threshold());
                hud.render(lobby_lines(count, min, None));
                bar.set_title(waiting_title(count, min));
                bar.set_progress(0.0);
            }
        }
    }

    async fn apply_events(&self, events: &[Event]) {
        if events.is_empty() {
            return;
        }
        let (tower_changes, mut drawn, notified, epoch, heard) = {
            let state = self.lock();
            let Some(game) = state.session.game() else {
                return;
            };
            let heard: Vec<(Option<Player>, SoundCue)> = sound_cues(events, game)
                .into_iter()
                .flat_map(|cue| {
                    let sound = SoundCue::new(cue.sound).volume(cue.volume).pitch(cue.pitch);
                    let listeners: Vec<Option<Player>> = match cue.hearing {
                        Hearing::At(at) => {
                            return vec![(None, sound.at(position(at)))];
                        }
                        Hearing::Everyone => state.players.values().cloned().map(Some).collect(),
                        Hearing::Lane(lane) => state
                            .players
                            .iter()
                            .filter(|(id, _)| state.session.lane_of(**id) == Some(lane))
                            .map(|(_, player)| Some(player.clone()))
                            .collect(),
                        Hearing::Player(id) => vec![state.players.get(&id).cloned()],
                    };
                    listeners
                        .into_iter()
                        .flatten()
                        .map(|player| (Some(player), sound.clone()))
                        .collect()
                })
                .collect();
            let name_of = |id: PlayerId| state.name_of(id);
            let audience: Vec<(Audience, String)> = notices(events, game, &name_of)
                .into_iter()
                .map(|notice| (notice.audience, notice.text))
                .collect();
            let recipients: Vec<(Audience, Player, String)> = audience
                .into_iter()
                .flat_map(|(audience, text)| {
                    state
                        .players
                        .iter()
                        .filter(|(id, _)| match audience {
                            Audience::Everyone => true,
                            Audience::Lane(lane) => state.session.lane_of(**id) == Some(lane),
                        })
                        .map(|(_, player)| (audience, player.clone(), text.clone()))
                        .collect::<Vec<_>>()
                })
                .collect();
            (
                tower_ops(events, game),
                shots(events, game),
                recipients,
                state.epoch.current(),
                heard,
            )
        };
        for (_, player, text) in notified {
            player.message.send(text);
        }
        for (listener, sound) in heard {
            match listener {
                Some(player) => player.sound.play(sound),
                None => self.arena.world.sound.play(sound),
            }
        }
        drawn.extend(self.retire_mobs(events));
        for op in tower_changes {
            self.apply_tower_op(op, epoch).await;
        }
        for shot in drawn
            .into_iter()
            .take(super::render::MAX_PARTICLES_PER_TICK)
        {
            self.arena
                .world
                .particle
                .spawn(ParticleBurst::new(shot.particle, position(shot.at)).count(shot.count));
        }
    }

    fn retire_mobs(&self, events: &[Event]) -> Vec<Shot> {
        let mut state = self.lock();
        let mut drawn = Vec::new();
        for event in events {
            let (mob, leaked) = match event {
                Event::MobKilled { mob, .. } => (mob, false),
                Event::MobLeaked { mob, .. } => (mob, true),
                _ => continue,
            };
            if let Some(entry) = state.mobs.remove(mob) {
                drawn.push(if leaked {
                    leak_shot(entry.track.position)
                } else {
                    death_shot(entry.track.position)
                });
                entry.entity.remove();
                entry.label.remove();
            }
        }
        drawn
    }

    async fn apply_tower_op(&self, op: TowerOp, epoch: Generation) {
        match op {
            TowerOp::Spawn {
                tower,
                kind,
                level,
                position: at,
            } => {
                if !self.lock().epoch.admits(epoch) {
                    return;
                }
                if let Some(entry) = self.spawn_tower(kind, level, at, 0.0).await {
                    self.keep_tower(tower, entry, epoch);
                }
            }
            TowerOp::Upgrade { tower, level } => {
                let old = {
                    let mut state = self.lock();
                    let old = state.towers.remove(&tower);
                    if let Some(entry) = &old {
                        state.hitboxes.remove(&entry.hitbox.entity_id);
                    }
                    old
                };
                let Some(old) = old else { return };
                let (kind, at, yaw) = (old.kind, old.at, old.yaw);
                old.remove();
                if let Some(entry) = self.spawn_tower(kind, level, at, yaw).await {
                    self.keep_tower(tower, entry, epoch);
                }
            }
            TowerOp::Aim { tower, yaw } => {
                let mut state = self.lock();
                if let Some(entry) = state.towers.get_mut(&tower)
                    && yaw_differs(entry.yaw, yaw, MIN_AIM_CHANGE)
                {
                    entry.aim(yaw);
                }
            }
            TowerOp::Remove { tower } => {
                let mut state = self.lock();
                if let Some(entry) = state.towers.remove(&tower) {
                    state.hitboxes.remove(&entry.hitbox.entity_id);
                    entry.remove();
                }
            }
        }
    }

    async fn spawn_display(
        &self,
        at: Position,
        spec: impl Into<DisplaySpec>,
    ) -> Result<Entity, Error> {
        let spec = spec.into();
        let entity = self.arena.world.spawn(display_kind(&spec), at).await?;
        entity.display.edit(spec);
        Ok(entity)
    }

    fn keep_tower(&self, tower: TowerId, entry: TowerEntry, epoch: Generation) {
        let mut state = self.lock();
        if state.epoch.admits(epoch) {
            state.hitboxes.insert(entry.hitbox.entity_id, tower);
            state.towers.insert(tower, entry);
        } else {
            drop(state);
            entry.remove();
        }
    }

    async fn spawn_tower(
        &self,
        kind: TowerKind,
        level: Level,
        at: Vec3,
        yaw: f32,
    ) -> Option<TowerEntry> {
        let model = tower_model(kind, level);
        let spawned = join_all(model.iter().map(|part| {
            let display = BlockDisplay::new(BlockState {
                kind: part.block,
                properties: Vec::new(),
            })
            .props(DisplayProps::default().transformation(part.transformation(yaw)));
            self.spawn_display(position(at), display)
        }))
        .await;
        let mut parts = Vec::with_capacity(model.len());
        let mut failed = false;
        for (result, part) in spawned.into_iter().zip(model) {
            match result {
                Ok(entity) => parts.push((entity, part)),
                Err(error) => {
                    tracing::warn!(
                        "impossible d'afficher la tour {} : {error}",
                        tower_name(kind)
                    );
                    failed = true;
                }
            }
        }
        let hitbox = InteractionBox::new(0.95, model_height(kind, level) + 0.2).response(true);
        let hitbox = match (failed, self.spawn_display(position(at), hitbox).await) {
            (false, Ok(entity)) => entity,
            (_, result) => {
                if let Err(error) = &result {
                    tracing::warn!("impossible de créer la zone de clic de la tour : {error}");
                }
                for (entity, _) in parts {
                    entity.remove();
                }
                if let Ok(entity) = result {
                    entity.remove();
                }
                return None;
            }
        };
        Some(TowerEntry {
            parts,
            hitbox,
            kind,
            at,
            yaw,
        })
    }

    async fn sync_mobs(&self) {
        let (spawns, epoch) = {
            let mut state = self.lock();
            let epoch = state.epoch.current();
            let observed = state.observed_lanes();
            let Some(game) = state.session.game() else {
                return;
            };
            let now = snapshots(game, &observed);
            let ops = diff_mobs(
                state.mobs.iter().map(|(id, entry)| (*id, &entry.track)),
                &now,
                state.ticks,
            );
            let mut spawns = Vec::new();
            for op in ops {
                match op {
                    MobOp::Spawn(snapshot) => spawns.push(snapshot),
                    MobOp::Move {
                        mob,
                        position: at,
                        yaw,
                    } => {
                        if let Some(entry) = state.mobs.get_mut(&mob) {
                            entry.entity.teleport(position(at));
                            if let Some(yaw) = yaw {
                                entry.entity.set_rotation(Rotation { yaw, pitch: 0.0 });
                                entry.track.yaw = yaw;
                            }
                            entry.track.position = at;
                        }
                    }
                    MobOp::Relabel { mob, text } => {
                        if let Some(entry) = state.mobs.get_mut(&mob) {
                            entry.label.display.edit(TextDisplay::new(text.as_str()));
                            entry.track.label = text;
                        }
                    }
                    MobOp::Remove { mob } => {
                        if let Some(entry) = state.mobs.remove(&mob) {
                            entry.entity.remove();
                            entry.label.remove();
                        }
                    }
                }
            }
            (spawns, epoch)
        };
        let spawned = join_all(spawns.iter().map(|snapshot| self.spawn_mob(snapshot))).await;
        for (snapshot, entry) in spawns.iter().zip(spawned) {
            if let Some(entry) = entry {
                let mut state = self.lock();
                if state.epoch.admits(epoch) {
                    state.mobs.insert(snapshot.id, entry);
                } else {
                    drop(state);
                    entry.entity.remove();
                    entry.label.remove();
                }
            }
        }
    }

    async fn spawn_mob(&self, snapshot: &MobSnapshot) -> Option<MobEntry> {
        let at = position(snapshot.position);
        let entity = match self.arena.world.spawn(mob_entity(snapshot.kind), at).await {
            Ok(entity) => entity,
            Err(error) => {
                tracing::warn!("impossible de faire apparaître un mob : {error}");
                return None;
            }
        };
        entity.ai.set(false);
        entity.flags.set_gravity(false);
        entity.flags.set_silent(true);
        entity.flags.set_invulnerable(true);
        entity.flags.set_persistent(false);
        entity
            .attributes
            .set(Attribute::Scale, mob_scale(snapshot.kind));
        entity.set_rotation(Rotation {
            yaw: snapshot.yaw,
            pitch: 0.0,
        });
        let label = TextDisplay::new(snapshot.label.as_str()).props(
            DisplayProps::default()
                .billboard(Billboard::Center)
                .transformation(Transformation::default().translation(
                    0.0,
                    super::render::LABEL_LIFT,
                    0.0,
                )),
        );
        let label = match self.spawn_display(at, label).await {
            Ok(label) => label,
            Err(error) => {
                tracing::debug!("impossible d'afficher la vie d'un mob : {error}");
                entity.remove();
                return None;
            }
        };
        entity.passengers.add(&label);
        Some(MobEntry {
            track: MobTrack::of(snapshot),
            entity,
            label,
        })
    }

    async fn sync_panels(&self) {
        let (missing, epoch) = {
            let state = self.lock();
            if !state.ticks.is_multiple_of(ACTION_BAR_REFRESH_TICKS) {
                return;
            }
            let Some(game) = state.session.game() else {
                return;
            };
            let name_of = |id: PlayerId| state.name_of(id);
            let mut missing = Vec::new();
            for lane in game
                .world()
                .lanes
                .iter()
                .filter(|lane| !lane.owners.is_empty())
            {
                let text = lane_panel(game, lane.id, &name_of);
                match state.panels.get(&lane.id) {
                    Some(panel) => panel.display.edit(TextDisplay::new(text.as_str())),
                    None => missing.push((lane.id, text)),
                }
            }
            (missing, state.epoch.current())
        };
        for (lane, text) in missing {
            let Some(at) = self.arena.layout.panel(lane) else {
                continue;
            };
            let panel = TextDisplay::new(text.as_str())
                .line_width(260)
                .background(PANEL_BACKGROUND)
                .props(
                    DisplayProps::default()
                        .billboard(Billboard::Vertical)
                        .transformation(Transformation::default().scale(
                            PANEL_SCALE,
                            PANEL_SCALE,
                            PANEL_SCALE,
                        )),
                );
            let Ok(entity) = self.spawn_display(point_position(at), panel).await else {
                continue;
            };
            let mut state = self.lock();
            if state.epoch.admits(epoch) && !state.panels.contains_key(&lane) {
                state.panels.insert(lane, entity);
            } else {
                drop(state);
                entity.remove();
            }
        }
    }

    async fn advance_previews(&self) {
        let watchers: Vec<(Player, TowerKind)> = {
            let state = self.lock();
            if !state.ticks.is_multiple_of(ACTION_BAR_REFRESH_TICKS) || !state.session.is_playing()
            {
                return;
            }
            state
                .views
                .iter()
                .filter_map(|(id, view)| {
                    let kind = view.selected?;
                    state.players.get(id).map(|player| (player.clone(), kind))
                })
                .collect()
        };
        for (player, kind) in watchers {
            let holding = player
                .inventory
                .get_items()
                .await
                .is_ok_and(|items| items.held_slot == BUILD_SLOT);
            if !holding {
                continue;
            }
            let Some(((x, z), surface_y)) = self.aimed_cell(&player).await else {
                continue;
            };
            let center = Vec3::new(x as f32 + 0.5, surface_y + 0.1, z as f32 + 0.5);
            let radius = kind.stats(Level::T1).range;
            for point in ring_points(center, radius, RING_POINTS) {
                player.particle.spawn(ParticleBurst::new(
                    Particle::Dust {
                        color: range_color(),
                        scale: 1.0,
                    },
                    position(point),
                ));
            }
        }
    }

    fn advance_celebration(&self) {
        let pulse = {
            let mut state = self.lock();
            let Some(celebration) = state.celebration.as_mut() else {
                return;
            };
            let age = celebration.age;
            celebration.age += 1;
            let lane = celebration.lane;
            if age >= CELEBRATION_TICKS {
                state.celebration = None;
                return;
            }
            if !age.is_multiple_of(CELEBRATION_PULSE_TICKS) {
                return;
            }
            lane.and_then(|lane| self.arena.layout.anchor(lane))
        };
        if let Some(anchor) = pulse {
            self.arena.world.particle.spawn(
                ParticleBurst::new(Particle::Firework, point_position(anchor))
                    .count(40)
                    .spread(Vector {
                        x: 4.0,
                        y: 3.0,
                        z: 4.0,
                    })
                    .speed(0.1),
            );
        }
    }

    pub async fn use_tool(&self, player: &Player, tool: Tool, right_click: bool) {
        match tool {
            Tool::Hub => {
                if right_click {
                    self.send_player_hub(player).await;
                }
            }
            Tool::Send => self.open_menu(player, MenuKind::Send).await,
            Tool::Teleport => self.open_menu(player, MenuKind::Lanes).await,
            Tool::Build => {
                if right_click {
                    self.build_click(player).await;
                } else {
                    self.open_menu(player, MenuKind::Build).await;
                }
            }
        }
    }

    async fn aimed_cell(&self, player: &Player) -> Option<((i32, i32), f32)> {
        let id = player_id(player.uuid);
        let surface_y = {
            let state = self.lock();
            let lane = state.session.lane_of(id)?;
            state.session.game()?.lane(lane).surface_y()
        };
        let state = player.get_state().await.ok()?;
        let at = state.player.position;
        let rotation = state.player.rotation;
        let cell = surface_cell(
            (at.x, at.y, at.z),
            rotation.yaw,
            rotation.pitch,
            f64::from(surface_y),
        )?;
        Some((cell, surface_y))
    }

    async fn build_click(&self, player: &Player) {
        let id = player_id(player.uuid);
        let selected = self.lock().views.get(&id).and_then(|view| view.selected);
        let Some((cell, _)) = self.aimed_cell(player).await else {
            tracing::info!(player = %player.name, ?selected, "clic de construction sans case visée");
            if selected.is_none() {
                self.open_menu(player, MenuKind::Build).await;
            }
            return;
        };
        tracing::info!(player = %player.name, ?cell, ?selected, "clic de construction");
        let existing = {
            let state = self.lock();
            state
                .session
                .lane_of(id)
                .and_then(|lane| state.session.game()?.lane(lane).tower_at(cell))
        };
        if let Some(tower) = existing {
            self.open_menu(player, MenuKind::Tower(tower)).await;
            return;
        }
        let Some(kind) = selected else {
            self.open_menu(player, MenuKind::Build).await;
            return;
        };
        match self.place_tower(id, kind, cell) {
            Ok(_) => tracing::info!(player = %player.name, ?cell, ?kind, "tour posée"),
            Err(text) => {
                tracing::info!(player = %player.name, ?cell, ?kind, refus = text, "pose refusée");
                player.message.send(text);
                player.sound.play(SoundCue::new(REFUSED).volume(0.7));
            }
        }
    }

    pub async fn tower_clicked(&self, player: &Player, entity_id: i32) -> bool {
        let id = player_id(player.uuid);
        let tower = self.lock().hitboxes.get(&entity_id).copied();
        let Some(tower) = tower else {
            return false;
        };
        let owned = {
            let state = self.lock();
            state
                .session
                .lane_of(id)
                .and_then(|lane| state.session.game()?.lane(lane).tower(tower))
                .is_some()
        };
        if owned {
            self.open_menu(player, MenuKind::Tower(tower)).await;
        } else {
            player
                .message
                .send(message(&crate::domain::SimError::NotOwner));
        }
        true
    }

    pub fn send_mob(&self, player: PlayerId, kind: MobKind, count: u8) -> Result<(), &'static str> {
        self.command(|game| game.send(player, kind, count))
    }

    pub fn place_tower(
        &self,
        player: PlayerId,
        kind: TowerKind,
        cell: (i32, i32),
    ) -> Result<TowerId, &'static str> {
        self.command(|game| game.place(player, kind, cell))
    }

    pub fn with_match<R>(&self, read: impl FnOnce(&Match) -> R) -> Option<R> {
        self.lock().session.game().map(read)
    }

    pub fn rendered(&self) -> (usize, usize) {
        let state = self.lock();
        (state.mobs.len(), state.towers.len())
    }

    fn command<T>(
        &self,
        action: impl FnOnce(&mut Match) -> Result<T, crate::domain::SimError>,
    ) -> Result<T, &'static str> {
        let mut state = self.lock();
        match state.session.playing() {
            None => Err("§cLa partie n'a pas commencé."),
            Some(game) => action(game).map_err(|error| message(&error)),
        }
    }

    async fn open_menu(&self, player: &Player, kind: MenuKind) {
        let id = player_id(player.uuid);
        let menu = self.lock().menu_for(id, kind);
        let Some(menu) = menu else {
            player
                .message
                .send("§cCe menu n'est pas disponible maintenant.");
            return;
        };
        let inventory = match self
            .server
            .create_inventory(menu.rows, menu.title.as_str())
            .await
        {
            Ok(inventory) => inventory,
            Err(error) => {
                tracing::warn!("impossible de créer le menu : {error}");
                return;
            }
        };
        fill(&inventory, &menu);
        if let Err(error) = player.inventory.open(&inventory).await {
            tracing::warn!("impossible d'ouvrir le menu : {error}");
            inventory.discard();
            return;
        }
        let stale = {
            let mut state = self.lock();
            let keys: Vec<u64> = state
                .menus
                .iter()
                .filter(|(_, open)| open.player == id)
                .map(|(key, _)| *key)
                .collect();
            let stale: Vec<CustomInventory> = keys
                .into_iter()
                .filter_map(|key| state.menus.remove(&key))
                .map(|open| open.inventory)
                .collect();
            state.menus.insert(
                inventory.id,
                OpenMenu {
                    player: id,
                    kind,
                    menu,
                    inventory,
                },
            );
            stale
        };
        for old in stale {
            old.discard();
        }
    }

    pub fn menu_closed(&self, inventory_id: u64) {
        if let Some(open) = self.lock().menus.remove(&inventory_id) {
            open.inventory.discard();
        }
    }

    pub fn owns_menu(&self, inventory_id: u64) -> bool {
        self.lock().menus.contains_key(&inventory_id)
    }

    pub async fn menu_click(
        &self,
        player: &Player,
        inventory_id: u64,
        slot: i32,
        right_click: bool,
        shift: bool,
    ) {
        let id = player_id(player.uuid);
        let picked = {
            let state = self.lock();
            let open = state.menus.get(&inventory_id);
            open.filter(|open| open.player == id).and_then(|open| {
                let action = u16::try_from(slot)
                    .ok()
                    .and_then(|slot| open.menu.action_at(slot))?;
                Some((open.kind, action))
            })
        };
        let Some((kind, action)) = picked else {
            return;
        };
        let close = self
            .menu_action(player, kind, action, right_click, shift)
            .await;
        if close {
            if let Some(open) = self.lock().menus.get(&inventory_id) {
                open.inventory.close();
            }
            self.menu_closed(inventory_id);
        } else {
            self.refresh_menu(inventory_id);
        }
    }

    async fn menu_action(
        &self,
        player: &Player,
        kind: MenuKind,
        action: MenuAction,
        right_click: bool,
        shift: bool,
    ) -> bool {
        let id = player_id(player.uuid);
        match action {
            MenuAction::Build(tower) => {
                self.lock().views.entry(id).or_default().selected = Some(tower);
                player.message.send(format!(
                    "§aTour sélectionnée : §f{}§a. Clic droit sur une case de votre voie pour construire.",
                    tower_name(tower)
                ));
                player.inventory.set_held_slot(i32::from(BUILD_SLOT));
                true
            }
            MenuAction::Send(mob) => {
                let count = send_count(right_click, shift);
                if let Err(text) = self.send_mob(id, mob, count) {
                    player.message.send(text);
                    player.sound.play(SoundCue::new(REFUSED).volume(0.7));
                }
                false
            }
            MenuAction::View(lane) => {
                self.view_lane(player, lane).await;
                true
            }
            MenuAction::ViewOwn => {
                let own = self.lock().session.lane_of(id);
                if let Some(lane) = own {
                    self.view_lane(player, lane).await;
                }
                true
            }
            MenuAction::Upgrade(branch) => {
                let MenuKind::Tower(tower) = kind else {
                    return false;
                };
                if let Err(text) = self.command(|game| game.upgrade(id, tower, branch)) {
                    player.message.send(text);
                }
                false
            }
            MenuAction::CyclePriority => {
                let MenuKind::Tower(tower) = kind else {
                    return false;
                };
                let result = self.command(|game| {
                    let lane = game
                        .world()
                        .player(id)
                        .ok_or(crate::domain::SimError::UnknownPlayer)?
                        .lane;
                    let current = game
                        .lane(lane)
                        .tower(tower)
                        .ok_or(crate::domain::SimError::UnknownTower)?
                        .priority;
                    game.set_priority(id, tower, next_priority(current))
                });
                if let Err(text) = result {
                    player.message.send(text);
                }
                false
            }
            MenuAction::Sell => {
                let MenuKind::Tower(tower) = kind else {
                    return false;
                };
                match self.command(|game| game.sell(id, tower)) {
                    Ok(refund) => player
                        .message
                        .send(format!("§aTour vendue : §6+{refund} or§a.")),
                    Err(text) => player.message.send(text),
                }
                true
            }
        }
    }

    fn refresh_menu(&self, inventory_id: u64) {
        let mut state = self.lock();
        let Some((player, kind)) = state
            .menus
            .get(&inventory_id)
            .map(|open| (open.player, open.kind))
        else {
            return;
        };
        let rebuilt = state.menu_for(player, kind);
        match (rebuilt, state.menus.get_mut(&inventory_id)) {
            (Some(menu), Some(open)) => {
                fill(&open.inventory, &menu);
                open.menu = menu;
            }
            (None, Some(open)) => open.inventory.close(),
            _ => {}
        }
    }

    async fn view_lane(&self, player: &Player, lane: LaneId) {
        let Some(anchor) = self.arena.layout.anchor(lane) else {
            return;
        };
        self.lock()
            .views
            .entry(player_id(player.uuid))
            .or_default()
            .viewing = Some(lane);
        self.teleport(player, anchor).await;
        player.abilities.set_allow_flight(true);
        player.abilities.set_flying(true);
    }
}

fn give_hub_item(player: &Player) {
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HUB_SLOT),
        Some(ItemStack::new(HUB_ITEM, 1).name("§cRetour au Hub §7(clic droit)")),
    ));
}

fn give_kit(player: &Player, playing: bool) {
    if playing {
        player.inventory.set(SlotItem::new(
            InventorySlot::Hotbar(BUILD_SLOT),
            Some(
                ItemStack::new(BUILD_ITEM, 1)
                    .name("§bConstruire §7(clic gauche : menu, clic droit : poser)"),
            ),
        ));
        player.inventory.set(SlotItem::new(
            InventorySlot::Hotbar(SEND_SLOT),
            Some(ItemStack::new(SEND_ITEM, 1).name("§aEnvoyer des mobs §7(clic)")),
        ));
    }
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(TELEPORT_SLOT),
        Some(ItemStack::new(TELEPORT_ITEM, 1).name("§eObserver une voie §7(clic)")),
    ));
    give_hub_item(player);
    player.inventory.set_held_slot(i32::from(BUILD_SLOT));
    player.effects.add(Effect {
        kind: EffectType::MiningFatigue,
        duration: Effect::INFINITE,
        amplifier: u8::MAX,
        ambient: true,
        particles: false,
        icon: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_forces_the_ending_phase() {
        for phase in [
            ServerPhase::Starting,
            ServerPhase::Waiting,
            ServerPhase::Countdown,
            ServerPhase::InGame,
            ServerPhase::Ending,
        ] {
            assert_eq!(phase_for(true, phase), ServerPhase::Ending);
            assert_eq!(phase_for(false, phase), phase);
        }
    }

    #[test]
    fn a_generation_is_admitted_until_the_epoch_advances() {
        let mut epoch = Epoch::default();
        let captured = epoch.current();
        assert!(epoch.admits(captured));
        epoch.advance();
        assert!(!epoch.admits(captured));
        assert!(epoch.admits(epoch.current()));
    }

    #[test]
    fn every_reset_invalidates_every_earlier_generation() {
        let mut epoch = Epoch::default();
        let first = epoch.current();
        epoch.advance();
        let second = epoch.current();
        epoch.advance();
        assert!(!epoch.admits(first));
        assert!(!epoch.admits(second));
    }

    #[tokio::test]
    async fn shutdown_returns_at_once_when_no_report_is_pending() {
        let in_flight = AtomicUsize::new(0);
        let started = Instant::now();
        assert!(wait_until_idle(&in_flight, Duration::from_secs(5)).await);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn shutdown_gives_up_on_a_report_that_never_settles() {
        let in_flight = AtomicUsize::new(1);
        let started = Instant::now();
        assert!(!wait_until_idle(&in_flight, Duration::from_millis(200)).await);
        assert!(started.elapsed() >= Duration::from_millis(200));
    }

    #[tokio::test]
    async fn shutdown_waits_for_a_report_that_settles_in_time() {
        let in_flight = Arc::new(AtomicUsize::new(1));
        let settle = Arc::clone(&in_flight);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            settle.fetch_sub(1, Ordering::AcqRel);
        });
        assert!(wait_until_idle(&in_flight, Duration::from_secs(2)).await);
    }

    #[test]
    fn the_hotbar_tools_are_recognised_by_their_item() {
        assert_eq!(tool_of(BUILD_ITEM), Some(Tool::Build));
        assert_eq!(tool_of(SEND_ITEM), Some(Tool::Send));
        assert_eq!(tool_of(TELEPORT_ITEM), Some(Tool::Teleport));
        assert_eq!(tool_of(HUB_ITEM), Some(Tool::Hub));
        assert_eq!(tool_of(Material::Stone), None);
    }

    #[test]
    fn tools_use_distinct_items_and_slots() {
        let items = [BUILD_ITEM, SEND_ITEM, TELEPORT_ITEM, HUB_ITEM];
        for (index, item) in items.iter().enumerate() {
            assert!(!items[..index].contains(item));
        }
        let slots = [BUILD_SLOT, SEND_SLOT, TELEPORT_SLOT, HUB_SLOT];
        assert_eq!(slots, [0, 1, 4, 8]);
    }

    #[test]
    fn a_player_id_is_the_player_uuid() {
        let uuid = Uuid::from_u128(5);
        assert_eq!(player_id(uuid).0, uuid);
    }
}
