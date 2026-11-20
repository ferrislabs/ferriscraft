use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ferriscraft_protocol::{HeartbeatRequest, ServerPhase};
use futures_util::future::join_all;
use rspigot_plugin::{
    BossBar, ChunkLoad, Entity, EntityState, EntityType, GameMode, ItemRemoval, ItemStack,
    Location, Player, Position, Rotation, Title, Uuid, Vector,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::display::lives_line;
use super::flight::{
    EYE_HEIGHT, Flight, LANDING_GRACE_TICKS, MAX_CONCURRENT_FLIGHTS, MAX_FLIGHT_TICKS, can_launch,
};
use super::intent::Intent;
use super::session::{ForceStartError, Session, VoidVerdict, launch_announcement};
use crate::arena::{Arena, ArenaError, Point};
use crate::config::SheepWarsConfig;
use crate::domain::{PlayerId, SheepKind, TeamId};
use crate::hud::{Hud, bossbar_progress, bossbar_title};
use crate::orchestrator::SheepWarsOrchestratorClient;
use crate::sheep::{
    Action, HEALING_DURATION_TICKS, HEALING_PULSE_TICKS, ParticleKind, SEEKER_DETONATE_DISTANCE,
    SEEKER_RANGE, SEEKER_SPEED, Vec3, apply, clamp_speed, display_name, healing_pulse,
    homes_in_flight, launch_power, nearest_enemy, plan, sheep_dye_color, sheep_wool, spawn_point,
    steer, step_ballistic, throw_velocity,
};

pub const HUD_REFRESH_TICKS: u64 = 5;
pub const ACTION_BAR_REFRESH_TICKS: u64 = 20;

pub fn player_id(uuid: Uuid) -> PlayerId {
    PlayerId(uuid.as_u128())
}

#[derive(Debug)]
struct Airborne {
    flight: Flight,
    entity: Entity,
    velocity: Vec3,
    last_pos: Vec3,
    rider_attached: bool,
}

#[derive(Debug)]
struct ActiveHealing {
    entity: Entity,
    team: TeamId,
    remaining_ticks: u32,
    pulse_in: u32,
    pulses: u32,
}

struct State {
    session: Session,
    players: HashMap<PlayerId, Player>,
    flights: Vec<Airborne>,
    healings: Vec<ActiveHealing>,
    fall_protected_until: HashMap<PlayerId, u64>,
    hud: Hud,
    bar: BossBar,
    ticks: u64,
    last_reported_phase: Option<ServerPhase>,
}

pub struct Game {
    arena: Arena,
    config: SheepWarsConfig,
    orchestrator: Option<Arc<SheepWarsOrchestratorClient>>,
    heartbeat_notify: Arc<Notify>,
    state: Mutex<State>,
}

impl Game {
    pub fn new(
        arena: Arena,
        session: Session,
        hud: Hud,
        bar: BossBar,
        config: SheepWarsConfig,
        orchestrator: Option<Arc<SheepWarsOrchestratorClient>>,
    ) -> Self {
        Self {
            arena,
            config,
            orchestrator,
            heartbeat_notify: Arc::new(Notify::new()),
            state: Mutex::new(State {
                session,
                players: HashMap::new(),
                flights: Vec::new(),
                healings: Vec::new(),
                fall_protected_until: HashMap::new(),
                hud,
                bar,
                ticks: 0,
                last_reported_phase: None,
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
            loop {
                let payload = game.heartbeat_payload();
                if let Err(err) = orchestrator.send_heartbeat(&payload).await {
                    tracing::debug!(
                        server_id = %payload.server_id,
                        phase = ?payload.phase,
                        "Échec envoi heartbeat orchestrateur : {err}"
                    );
                }
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = game.heartbeat_notify.notified() => {}
                }
            }
        }))
    }

    pub fn heartbeat_payload(&self) -> HeartbeatRequest {
        let state = self.lock();
        let player_uuids: Vec<Uuid> = state.players.values().map(|p| p.uuid).collect();
        let player_count = u32::try_from(player_uuids.len()).unwrap_or(u32::MAX);
        let max_players = u32::try_from(self.config.rules.max_players).unwrap_or(u32::MAX);
        HeartbeatRequest {
            server_id: self.config.server_id.clone(),
            address: None,
            phase: state.session.server_phase(),
            player_count,
            max_players,
            player_uuids,
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

    pub async fn admit(&self, player: Player) {
        let id = player_id(player.uuid);
        let intents = {
            let mut state = self.lock();
            state.bar.add_player(&player);
            state.players.insert(id, player);
            state.session.join(id)
        };
        self.run(intents).await;
        self.heartbeat_notify.notify_one();
    }

    pub fn restore_platforms(&self) -> Result<usize, ArenaError> {
        self.arena.restore_platforms()
    }

    pub fn on_chunk_load(&self, event: &ChunkLoad) {
        if event.world().id() != self.arena.world.id() {
            return;
        }
        let chunk = event.chunk();
        match self.arena.place_chunk(chunk.x, chunk.z) {
            Ok(0) => {}
            Ok(placed) => {
                tracing::debug!(
                    "placé {placed} blocs dans le chunk ({}, {})",
                    chunk.x,
                    chunk.z
                )
            }
            Err(error) => {
                tracing::warn!(
                    "impossible de placer le chunk ({}, {}) : {error}",
                    chunk.x,
                    chunk.z
                )
            }
        }
    }

    pub async fn release(&self, player: &Player) {
        let id = player_id(player.uuid);
        let intents = {
            let mut state = self.lock();
            state.bar.remove_player(player);
            state.session.leave(id)
        };
        self.run(intents).await;
        self.lock().players.remove(&id);
        self.heartbeat_notify.notify_one();
    }

    pub async fn join(&self, player: &Player) {
        let id = player_id(player.uuid);
        let intents = {
            let mut state = self.lock();
            state.players.insert(id, player.clone());
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
            player
                .message
                .send("§eRetour au Hub FerrisCraft...");
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

    pub async fn died(&self, player: &Player) {
        let intents = self.lock().session.die(player_id(player.uuid));
        self.run(intents).await;
        self.check_phase_changed();
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

    pub fn respawn_location(&self, player: &Player) -> Location {
        let point = self.lock().session.respawn_point(player_id(player.uuid));
        self.location(point)
    }

    pub fn blocks_damage(&self) -> bool {
        self.lock().session.blocks_damage()
    }

    pub fn is_fall_protected(&self, uuid: Uuid) -> bool {
        let state = self.lock();
        state
            .fall_protected_until
            .get(&player_id(uuid))
            .is_some_and(|until| state.ticks <= *until)
    }

    pub fn blocks_hit(&self, attacker: &Player, victim: &Player) -> bool {
        self.lock()
            .session
            .blocks_hit(player_id(attacker.uuid), player_id(victim.uuid))
    }

    pub fn void_verdict(&self, player: &Player, y: f64) -> VoidVerdict {
        self.lock().session.void_verdict(player_id(player.uuid), y)
    }

    pub async fn rescue(&self, player: &Player) {
        let lobby = self.lock().session.layout().lobby();
        self.teleport(player, lobby).await;
    }

    pub async fn throw(&self, player: &Player, kind: SheepKind) -> bool {
        let id = player_id(player.uuid);
        let team = {
            let state = self.lock();
            let standing = state.session.standing();
            let Some((_, team)) = standing.into_iter().find(|(member, _)| *member == id) else {
                return false;
            };
            if !state.session.is_playing() || !can_launch(state.flights.len()) {
                return false;
            }
            team
        };
        let eye = Vec3 {
            x: player.position.x,
            y: player.position.y + EYE_HEIGHT,
            z: player.position.z,
        };
        let yaw = f64::from(player.rotation.yaw);
        let pitch = f64::from(player.rotation.pitch);
        let origin = spawn_point(eye, yaw, pitch);
        let sheep = match self
            .arena
            .world
            .spawn(
                EntityType::Sheep,
                Position {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z,
                },
            )
            .await
        {
            Ok(sheep) => sheep,
            Err(error) => {
                tracing::warn!(
                    "impossible de faire apparaître un mouton pour {} : {error}",
                    player.name
                );
                return false;
            }
        };
        if let Some(typed) = sheep.as_sheep() {
            typed.set_color(sheep_dye_color(kind));
        }
        sheep.ai.set(false);
        sheep.flags.set_gravity(false);
        sheep.flags.set_invulnerable(true);
        sheep.name.set(display_name(kind));

        let velocity = throw_velocity(yaw, pitch, launch_power(kind));
        sheep.set_velocity(vector(velocity));
        player
            .inventory
            .remove(ItemRemoval::new(sheep_wool(kind), 1));

        let rider_attached = kind == SheepKind::Boarding;
        if rider_attached {
            player.movement.set_velocity(vector(velocity));
        }

        {
            let mut state = self.lock();
            if rider_attached {
                let deadline = state.ticks + u64::from(MAX_FLIGHT_TICKS) + 40;
                state.fall_protected_until.insert(id, deadline);
            }
            state.flights.push(Airborne {
                flight: Flight::new(kind, team, id),
                entity: sheep,
                velocity,
                last_pos: origin,
                rider_attached,
            });
        }
        if let Some(announcement) = launch_announcement(kind, team) {
            self.run(vec![announcement]).await;
        }
        true
    }

    pub async fn tick(&self) {
        let intents = self.lock().session.tick();
        self.run(intents).await;
        self.advance_flights().await;
        self.advance_healings().await;
        self.refresh();
        self.check_phase_changed();
    }

    pub fn close(&self) {
        let mut state = self.lock();
        for airborne in state.flights.drain(..) {
            airborne.entity.remove();
        }
        for totem in state.healings.drain(..) {
            totem.entity.remove();
        }
        state.fall_protected_until.clear();
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
            position: Position {
                x: point.x,
                y: point.y,
                z: point.z,
            },
            rotation: Rotation::default(),
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
            Intent::Title { text, subtitle } => {
                for player in self.everyone() {
                    player
                        .title
                        .send(Title::new(text.as_str()).subtitle(subtitle.as_str()));
                }
            }
            Intent::Enroll { player, team } => {
                if let Some(player) = self.player(player) {
                    self.lock().hud.add_player(team, &player.name);
                    player.game_mode.set(GameMode::Survival);
                    player.inventory.clear();
                    player.health.set(20.0);
                    player.food.set_level(20);
                    player.food.set_saturation(5.0);
                }
            }
            Intent::Unenroll { player, team } => {
                if let Some(player) = self.player(player) {
                    self.lock().hud.remove_player(team, &player.name);
                }
            }
            Intent::SyncTeams => self.sync_teams(),
            Intent::Teleport { player, to } => {
                if let Some(player) = self.player(player) {
                    self.teleport(&player, to).await;
                }
            }
            Intent::ClearInventory { player } => {
                if let Some(player) = self.player(player) {
                    player.inventory.clear();
                    player.health.set(20.0);
                    player.health.set_fire_ticks(0);
                    player.food.set_level(20);
                    player.food.set_saturation(5.0);
                }
            }
            Intent::Give { player, kind } => {
                if let Some(player) = self.player(player) {
                    player
                        .inventory
                        .add(ItemStack::new(sheep_wool(kind), 1).name(display_name(kind)));
                }
            }
            Intent::Respawn { player } => {
                if let Some(player) = self.player(player) {
                    player.health.respawn();
                }
            }
            Intent::Spectate { player } => {
                if let Some(player) = self.player(player) {
                    player.game_mode.set(GameMode::Spectator);
                }
            }
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
                if let Some(orch) = &self.orchestrator {
                    let uuid = Uuid::from_u128(player.0);
                    if let Err(err) = orch.send_player_to_lobby(uuid, Some(reason)).await {
                        tracing::warn!(%uuid, "Échec retour au Hub orchestrateur : {err}");
                    }
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
            Intent::Reset => return self.reset(),
        }
        Vec::new()
    }

    fn sync_teams(&self) {
        let state = self.lock();
        for (id, player) in &state.players {
            for team in TeamId::ALL {
                state.hud.remove_player(team, &player.name);
            }
            if let Some(team) = state.session.round().team_of(*id) {
                state.hud.add_player(team, &player.name);
            }
        }
    }

    fn reset(&self) -> Vec<Intent> {
        if let Err(err) = self.arena.restore_platforms() {
            tracing::warn!("Impossible de restaurer les plateformes de l'arène : {err}");
        }
        let mut state = self.lock();
        for airborne in state.flights.drain(..) {
            airborne.entity.remove();
        }
        for totem in state.healings.drain(..) {
            totem.entity.remove();
        }
        state.fall_protected_until.clear();
        for player in state.players.values() {
            for team in TeamId::ALL {
                state.hud.remove_player(team, &player.name);
            }
        }
        let mut ids: Vec<PlayerId> = state.players.keys().copied().collect();
        ids.sort();
        ids.into_iter()
            .flat_map(|id| state.session.join(id))
            .collect()
    }

    fn refresh(&self) {
        let mut state = self.lock();
        state.ticks += 1;
        let ticks = state.ticks;
        let state = &mut *state;
        let round = state.session.round();
        if ticks.is_multiple_of(HUD_REFRESH_TICKS) {
            state.hud.render(round, None);
            state.bar.set_title(bossbar_title(round));
            state.bar.set_progress(bossbar_progress(round));
        }
        if ticks.is_multiple_of(ACTION_BAR_REFRESH_TICKS) {
            for (id, player) in &state.players {
                let lives = round
                    .team_of(*id)
                    .and_then(|team| round.team(team).lives_of(*id));
                if let Some(lives) = lives {
                    player.action_bar.send(lives_line(lives));
                }
            }
        }
    }

    async fn advance_flights(&self) {
        let (flights, players_with_teams, player_map, current_ticks, void_fall_y) = {
            let mut state = self.lock();
            if state.flights.is_empty() {
                return;
            }
            let teams: HashMap<PlayerId, TeamId> = state.session.standing().into_iter().collect();
            let players_with_teams: Vec<(Player, TeamId)> = teams
                .into_iter()
                .filter_map(|(id, team)| state.players.get(&id).cloned().map(|p| (p, team)))
                .collect();
            let player_map = state.players.clone();
            let current_ticks = state.ticks;
            let void_fall_y = state.session.layout().settings().void_fall_y;
            (
                std::mem::take(&mut state.flights),
                players_with_teams,
                player_map,
                current_ticks,
                void_fall_y,
            )
        };

        let mut next = Vec::with_capacity(flights.len());
        let mut children = Vec::new();
        let mut new_healings = Vec::new();
        let mut fall_updates: Vec<(PlayerId, u64)> = Vec::new();

        for mut airborne in flights {
            airborne.flight.age += 1;
            let sheep = match airborne.entity.get_state().await {
                Ok(sheep) => sheep,
                Err(error) => {
                    tracing::warn!("mouton perdu en vol : {error}");
                    airborne.entity.remove();
                    continue;
                }
            };
            let pos = vec3(sheep.entity.position);
            if pos.y < void_fall_y {
                airborne.entity.remove();
                continue;
            }

            if airborne.rider_attached
                && let Some(rider) = player_map.get(&airborne.flight.thrower)
            {
                match rider.get_state().await {
                    Ok(rider_state) if !rider_state.dead => {
                        if rider_state.sneaking && airborne.flight.age >= LANDING_GRACE_TICKS {
                            airborne.rider_attached = false;
                            fall_updates.push((airborne.flight.thrower, current_ticks + 40));
                        } else {
                            let rider_pos = vec3(rider_state.player.position);
                            let dx = pos.x - rider_pos.x;
                            let dy = (pos.y - 0.1) - rider_pos.y;
                            let dz = pos.z - rider_pos.z;
                            let dist_sq = dx * dx + dy * dy + dz * dz;
                            if dist_sq > 2.5 * 2.5 {
                                let _ = rider
                                    .location
                                    .teleport(Position {
                                        x: pos.x,
                                        y: pos.y + 0.1,
                                        z: pos.z,
                                    })
                                    .await;
                            }
                            let pull = clamp_speed(
                                Vec3 {
                                    x: airborne.velocity.x + dx * 0.55,
                                    y: airborne.velocity.y + dy * 0.55,
                                    z: airborne.velocity.z + dz * 0.55,
                                },
                                3.2,
                            );
                            rider.movement.set_velocity(vector(pull));
                            fall_updates.push((airborne.flight.thrower, current_ticks + 60));
                        }
                    }
                    _ => {
                        airborne.rider_attached = false;
                    }
                }
            }

            let seeker_target = if homes_in_flight(airborne.flight.kind) {
                nearest_enemy(
                    pos,
                    SEEKER_RANGE,
                    airborne.flight.team,
                    &players_with_teams,
                )
                .await
            } else {
                None
            };
            let target_hit = seeker_target.is_some_and(|(_, dist)| {
                dist <= SEEKER_DETONATE_DISTANCE && airborne.flight.age >= LANDING_GRACE_TICKS
            });

            let expected_horiz = airborne.velocity.x.hypot(airborne.velocity.z);
            let moved_horiz = (pos.x - airborne.last_pos.x).hypot(pos.z - airborne.last_pos.z);
            let wall_hit = expected_horiz > 0.35 && moved_horiz < expected_horiz * 0.2;

            if airborne.flight.should_land(sheep.on_ground, wall_hit) || target_hit {
                if airborne.flight.kind == SheepKind::Healing
                    && sheep.on_ground
                    && airborne.flight.age < MAX_FLIGHT_TICKS
                {
                    let actions = plan(SheepKind::Healing, pos, airborne.flight.team);
                    if let Err(error) =
                        apply(&self.arena.world, None, &players_with_teams, &actions).await
                    {
                        tracing::warn!("échec d'ancrage du mouton soigneur : {error}");
                    }
                    airborne.entity.ai.set(false);
                    airborne.entity.flags.set_gravity(true);
                    airborne.entity.flags.set_invulnerable(true);
                    airborne.entity.set_velocity(vector(Vec3::ZERO));
                    airborne.entity.name.set(display_name(SheepKind::Healing));
                    airborne.entity.name.set_visible(true);
                    new_healings.push(ActiveHealing {
                        entity: airborne.entity,
                        team: airborne.flight.team,
                        remaining_ticks: HEALING_DURATION_TICKS,
                        pulse_in: HEALING_PULSE_TICKS,
                        pulses: 0,
                    });
                } else {
                    if airborne.flight.kind == SheepKind::Boarding && airborne.rider_attached {
                        fall_updates.push((airborne.flight.thrower, current_ticks + 60));
                    }
                    children.extend(
                        self.land(&airborne, &sheep, &players_with_teams, &player_map)
                            .await,
                    );
                    airborne.entity.remove();
                }
            } else {
                let next_velocity = if let Some((target, _)) = seeker_target {
                    steer(pos, target, SEEKER_SPEED)
                } else {
                    step_ballistic(airborne.velocity)
                };
                airborne.entity.set_velocity(vector(next_velocity));
                airborne.velocity = next_velocity;
                airborne.last_pos = pos;
                next.push(airborne);
            }
        }

        let mut state = self.lock();
        state
            .fall_protected_until
            .retain(|_, until| *until >= current_ticks);
        for (id, deadline) in fall_updates {
            state.fall_protected_until.insert(id, deadline);
        }
        state.healings.extend(new_healings);
        next.extend(children);
        next.truncate(MAX_CONCURRENT_FLIGHTS.saturating_sub(state.flights.len()));
        state.flights.extend(next);
    }

    async fn advance_healings(&self) {
        let (healings, players_with_teams, void_fall_y) = {
            let mut state = self.lock();
            if state.healings.is_empty() {
                return;
            }
            let teams: HashMap<PlayerId, TeamId> = state.session.standing().into_iter().collect();
            let players_with_teams: Vec<(Player, TeamId)> = teams
                .into_iter()
                .filter_map(|(id, team)| state.players.get(&id).cloned().map(|p| (p, team)))
                .collect();
            let void_fall_y = state.session.layout().settings().void_fall_y;
            (
                std::mem::take(&mut state.healings),
                players_with_teams,
                void_fall_y,
            )
        };

        let mut next_healings = Vec::with_capacity(healings.len());
        for mut totem in healings {
            totem.remaining_ticks = totem.remaining_ticks.saturating_sub(1);
            let sheep = match totem.entity.get_state().await {
                Ok(sheep) => sheep,
                Err(error) => {
                    tracing::warn!("mouton soigneur perdu : {error}");
                    totem.entity.remove();
                    continue;
                }
            };
            let pos = vec3(sheep.entity.position);
            if pos.y < void_fall_y {
                totem.entity.remove();
                continue;
            }
            if totem.remaining_ticks == 0 {
                let _ = apply(
                    &self.arena.world,
                    None,
                    &players_with_teams,
                    &[Action::Particle {
                        kind: ParticleKind::Heart,
                        at: pos,
                        count: 10,
                    }],
                )
                .await;
                totem.entity.remove();
                continue;
            }
            if totem.pulse_in == 0 {
                totem.pulse_in = HEALING_PULSE_TICKS;
                totem.pulses += 1;
                let actions = healing_pulse(pos, totem.team, totem.pulses.is_multiple_of(2));
                if let Err(error) =
                    apply(&self.arena.world, None, &players_with_teams, &actions).await
                {
                    tracing::warn!("échec d'impulsion du mouton soigneur : {error}");
                }
            } else {
                totem.pulse_in -= 1;
            }
            next_healings.push(totem);
        }

        self.lock().healings.extend(next_healings);
    }

    async fn land(
        &self,
        airborne: &Airborne,
        sheep: &EntityState,
        players: &[(Player, TeamId)],
        player_map: &HashMap<PlayerId, Player>,
    ) -> Vec<Airborne> {
        let Flight {
            kind,
            team,
            thrower,
            ..
        } = airborne.flight;
        let impact_pos = vec3(sheep.entity.position);
        let actions = plan(kind, impact_pos, team);
        let thrower_player = if kind == SheepKind::Boarding && !airborne.rider_attached {
            None
        } else {
            player_map.get(&thrower)
        };
        match apply(&self.arena.world, thrower_player, players, &actions).await {
            Ok(spawned) => spawned
                .into_iter()
                .map(|(kind, entity, velocity)| Airborne {
                    flight: Flight::new(kind, team, thrower),
                    entity,
                    velocity,
                    last_pos: impact_pos,
                    rider_attached: false,
                })
                .collect(),
            Err(error) => {
                tracing::warn!("échec d'impact du mouton {kind:?} : {error}");
                Vec::new()
            }
        }
    }
}

const fn vec3(position: Position) -> Vec3 {
    Vec3 {
        x: position.x,
        y: position.y,
        z: position.z,
    }
}

const fn vector(vec: Vec3) -> Vector {
    Vector {
        x: vec.x,
        y: vec.y,
        z: vec.z,
    }
}
