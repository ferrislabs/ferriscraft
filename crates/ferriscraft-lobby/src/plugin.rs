use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use ferriscraft_protocol::QueueJoinStatus;
use rspigot_plugin::prelude::*;
use rspigot_plugin::{
    Action, BarColor, BarStyle, EquipmentSlot, EventResult, Explosion, Facing, Particle,
    ParticleBurst, SoundCue, Uuid, Vector,
};

use crate::auth::{AuthError, AuthManager, Rank, build_device_prompt, build_device_prompt_for};
use crate::capitainerie::{
    CANCEL_BUTTON_ID, CAP_LOGIN_BUTTON_ID, CAPTAIN_DISPLAY_NAME, LOGIN_BUTTON_ID,
    RESULT_BUTTON_PREFIX, RUN_BUTTON_PREFIX, RunStarted, TaskResultReport, TeamAgent,
    build_run_completion_message, build_run_result_dialog, click_slug, clip_text,
    extract_agent_reply, step_name,
};
use crate::chat::ChatSystem;
use crate::config::{ConfigError, EnvConfig, LayoutStore, WaypointPose};
use crate::gadgets::{AuraTrail, GadgetManager, LobbyGadget};
use crate::hud::{HubMetrics, HudManager, PlayerHudInput};
use crate::menus::{
    GadgetsMenuAction, GamesMenuAction, MenuManager, OptionsMenuAction, apply_lobby_hotbar,
    decode_gadgets_click, decode_games_click, decode_options_click,
};
use crate::npcs::{CaptainTurn, NpcClickTarget, NpcDirector};
use crate::orchestrator::{OrchestratorClient, ToggleQueueOutcome};
use crate::prefs::PrefsStore;

const MAX_TASK_TITLE: u32 = 120;
const MAX_TASK_DESC: u32 = 2000;
const HUD_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const PORTAL_DEBOUNCE: Duration = Duration::from_millis(400);
const PORTAL_BLAST_POWER: f32 = 6.0;
const PORTAL_LAUNCH_MIN_SPEED: f64 = 0.9;
const PORTAL_LAUNCH_MAX_SPEED: f64 = 2.0;
const PORTAL_LAUNCH_LIFT: f64 = 1.1;
const PORTAL_LAUNCH_DELAY: Duration = Duration::from_millis(60);
const RUN_WATCH_POLL_INTERVAL: Duration = Duration::from_secs(1);
const RUN_WATCH_MAX_POLLS: usize = 900;
const RUN_FINISH_BAR_LINGER: Duration = Duration::from_secs(3);
const MAX_CACHED_RESULTS: usize = 128;
const LOBBY_SUBCOMMANDS: [&str; 8] = [
    "info",
    "reload",
    "setspawn",
    "setpoi",
    "setpedestal",
    "setcaptain",
    "setstation",
    "setwaypoint",
];

struct SharedState {
    config: EnvConfig,
    layout: Arc<LayoutStore>,
    auth: Arc<AuthManager>,
    prefs: PrefsStore,
    menus: MenuManager,
    gadgets: GadgetManager,
    hud: HudManager,
    chat: ChatSystem,
    npcs: Arc<NpcDirector>,
    orchestrator: Option<Arc<OrchestratorClient>>,
    sheepwars_queue: RwLock<HashSet<Uuid>>,
    portal_last_blast: RwLock<HashMap<Uuid, Instant>>,
    last_steps: RwLock<HashMap<Uuid, Vector>>,
    task_results: RwLock<HashMap<String, TaskResultReport>>,
}

#[derive(Plugin, Clone)]
pub struct FerrisCraftLobby {
    state: Arc<SharedState>,
}

impl FerrisCraftLobby {
    pub fn new(config: EnvConfig, layout: LayoutStore) -> Self {
        let layout = Arc::new(layout);
        let auth = Arc::new(AuthManager::new(config.clone()));
        let npcs = Arc::new(NpcDirector::new(
            Arc::clone(&layout),
            config.capitainerie_url.clone(),
        ));
        let orchestrator = config.orchestrator_url.clone().map(|url| {
            Arc::new(OrchestratorClient::new(
                url,
                config.orchestrator_token.clone(),
            ))
        });
        Self {
            state: Arc::new(SharedState {
                config,
                layout,
                auth,
                prefs: PrefsStore::default(),
                menus: MenuManager::default(),
                gadgets: GadgetManager::default(),
                hud: HudManager::default(),
                chat: ChatSystem::default(),
                npcs,
                orchestrator,
                sheepwars_queue: RwLock::new(HashSet::new()),
                portal_last_blast: RwLock::new(HashMap::new()),
                last_steps: RwLock::new(HashMap::new()),
                task_results: RwLock::new(HashMap::new()),
            }),
        }
    }

    fn queued_count(&self) -> usize {
        if let Some(client) = &self.state.orchestrator {
            return client.cached_queue_len();
        }
        self.state
            .sheepwars_queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    async fn collect_metrics(&self, server: &Server) -> HubMetrics {
        let (online, max_players, tps_1m) = match server.get_state().await {
            Ok(st) => (st.online, st.max_players, st.tps[0]),
            Err(_) => (0, 100, 20.0),
        };
        let (agents_working, agents_total) = self.state.npcs.agent_counts();
        HubMetrics {
            online,
            max_players,
            tps_1m,
            sheepwars_queued: self.queued_count(),
            agents_working,
            agents_total,
        }
    }

    async fn refresh_single_player_ui(&self, player: &Player) {
        let claims = self.state.auth.claims_of(player.uuid);
        let prefs = self.state.prefs.get(player.uuid);
        apply_lobby_hotbar(player, claims.as_ref(), &prefs);
        self.state
            .menus
            .refresh_open_options(player.uuid, &prefs, claims.as_ref());
        self.state.menus.refresh_open_gadgets(player.uuid, &prefs);

        let (ping_ms, is_op) = match player.get_state().await {
            Ok(st) => (st.ping, st.op),
            Err(_) => (0, false),
        };
        let has_admin = player.permissions.has("ferriscraft.admin");
        let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
        let metrics = self.collect_metrics(&player.server()).await;
        self.state.hud.refresh_player(
            player,
            &PlayerHudInput {
                rank,
                ferriskey_username: claims.map(|c| c.username),
                ping_ms,
                metrics,
            },
        );
    }

    fn start_ferriskey_login(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            match lobby.state.auth.start_device_flow(player.uuid).await {
                Ok((device, cancel_rx)) => {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.8)
                            .pitch(1.2),
                    );
                    player.message.send_rich(build_device_prompt(&device));

                    match lobby
                        .state
                        .auth
                        .poll_device_flow(player.uuid, &device, cancel_rx)
                        .await
                    {
                        Ok(session) => {
                            player.sound.play(
                                SoundCue::new(Sound::EntityPlayerLevelup)
                                    .volume(0.9)
                                    .pitch(1.15),
                            );
                            player.title.send(
                                Title::new("§a§l✔ FERRISKEY LIÉ").subtitle(format!(
                                    "§fBienvenue, §b{}",
                                    session.claims.username
                                )),
                            );
                            player.message.send(format!(
                                "§a✔ Votre compte FerrisKey (§f{}§a) est désormais actif !",
                                session.claims.username
                            ));
                            lobby.refresh_single_player_ui(&player).await;
                        }
                        Err(AuthError::Cancelled) => {}
                        Err(AuthError::Expired) => {
                            player.message.send(
                                "§e⌛ Le lien FerrisKey a expiré. Tapez §f/login §epour recommencer.",
                            );
                        }
                        Err(AuthError::Denied) => {
                            player
                                .message
                                .send("§c✖ Connexion FerrisKey annulée dans le navigateur.");
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Échec du polling Device Flow pour {}: {err}",
                                player.name
                            );
                            player.message.send(format!("§c{err}"));
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!("Impossible d'initier FerrisKey pour {}: {err}", player.name);
                    let detail = err.to_string();
                    player.message.send(format!(
                        "§cImpossible de joindre FerrisKey ({}) : {detail}",
                        lobby.state.config.ferriskey_base_url
                    ));
                    if detail.contains("unauthorized_client") {
                        player.message.send(format!(
                            "§eAstuce : vérifiez dans FerrisKey (realm §f{}§e → client §f{}§e) que §aDevice Code Grant §eest activé.",
                            lobby.state.config.ferriskey_realm,
                            lobby.state.config.ferriskey_client_id
                        ));
                    }
                }
            }
        });
    }

    fn start_capitainerie_login(&self, player: Player) {
        if !self.state.auth.uses_separate_capitainerie_auth() {
            self.start_ferriskey_login(player);
            return;
        }
        let lobby = self.clone();
        tokio::spawn(async move {
            let realm = lobby.state.config.capitainerie_realm.clone();
            match lobby
                .state
                .auth
                .start_capitainerie_device_flow(player.uuid)
                .await
            {
                Ok((device, cancel_rx)) => {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.8)
                            .pitch(1.2),
                    );
                    let prompt_label = format!("Capitainerie (realm {realm})");
                    player
                        .message
                        .send_rich(build_device_prompt_for(&device, Some(&prompt_label)));

                    match lobby
                        .state
                        .auth
                        .poll_capitainerie_device_flow(player.uuid, &device, cancel_rx)
                        .await
                    {
                        Ok(session) => {
                            player.sound.play(
                                SoundCue::new(Sound::EntityPlayerLevelup)
                                    .volume(0.9)
                                    .pitch(1.15),
                            );
                            player.title.send(
                                Title::new("§a§l✔ CAPITAINERIE LIÉE")
                                    .subtitle(format!("§fCompte : §b{}", session.claims.username)),
                            );
                            player.message.send(format!(
                                "§a✔ Votre compte Capitainerie (§f{}§a, realm §b{realm}§a) est désormais actif !",
                                session.claims.username
                            ));
                        }
                        Err(AuthError::Cancelled) => {}
                        Err(AuthError::Expired) => {
                            player.message.send(
                                "§e⌛ Le lien Capitainerie a expiré. Tapez §f/caplogin §epour recommencer.",
                            );
                        }
                        Err(AuthError::Denied) => {
                            player
                                .message
                                .send("§c✖ Connexion Capitainerie annulée dans le navigateur.");
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Échec du polling Device Flow Capitainerie pour {}: {err}",
                                player.name
                            );
                            player.message.send(format!("§c{err}"));
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "Impossible d'initier FerrisKey Capitainerie pour {}: {err}",
                        player.name
                    );
                    let detail = err.to_string();
                    player.message.send(format!(
                        "§cImpossible de joindre FerrisKey ({}) pour Capitainerie : {detail}",
                        lobby.state.config.capitainerie_ferriskey_url
                    ));
                    if detail.contains("unauthorized_client") || detail.contains("invalid_client") {
                        player.message.send(format!(
                            "§eAstuce : vérifiez dans FerrisKey (realm §f{realm}§e → client §f{}§e) que §aDevice Code Grant §eest activé.",
                            lobby.state.config.capitainerie_client_id
                        ));
                    }
                }
            }
        });
    }

    fn prompt_login_required(&self, player: &Player, context_label: &str) {
        player.sound.play(
            SoundCue::new(Sound::BlockNoteBlockBell)
                .volume(0.8)
                .pitch(0.8),
        );
        player.dialog.show(
            Dialog::new("§6§lAuthentification FerrisKey requise")
                .line(format!(
                    "§7Pour utiliser §f{context_label}§7, connectez votre compte personnel FerrisKey."
                ))
                .line("§7Aucun mot de passe n'est saisi dans Minecraft : vous validez dans le navigateur.")
                .button(DialogButton::new(
                    "§a🔐 Se connecter avec FerrisKey",
                    LOGIN_BUTTON_ID,
                ))
                .button(DialogButton::new("§7Plus tard", CANCEL_BUTTON_ID)),
        );
    }

    fn prompt_capitainerie_login_required(&self, player: &Player, context_label: &str) {
        if !self.state.auth.uses_separate_capitainerie_auth() {
            self.prompt_login_required(player, context_label);
            return;
        }
        player.sound.play(
            SoundCue::new(Sound::BlockNoteBlockBell)
                .volume(0.8)
                .pitch(0.8),
        );
        player.dialog.show(
            Dialog::new("§6§lConnexion Capitainerie requise")
                .line(format!(
                    "§7Pour utiliser §f{context_label}§7, connectez votre compte du realm §b{}§7.",
                    self.state.config.capitainerie_realm
                ))
                .line("§7Cette session Capitainerie est indépendante de votre compte Lobby.")
                .button(DialogButton::new(
                    "§a🔐 Se connecter à Capitainerie",
                    CAP_LOGIN_BUTTON_ID,
                ))
                .button(DialogButton::new("§7Plus tard", CANCEL_BUTTON_ID)),
        );
    }

    fn portal_blast(&self, player: Player, at: Position, entry_step: Option<Vector>, yaw: f32) {
        let launch = portal_launch_velocity(entry_step, yaw);
        tokio::spawn(async move {
            let world = &player.world;
            world
                .explosion
                .create(Explosion::new(at, PORTAL_BLAST_POWER, false));
            world.particle.spawn(
                ParticleBurst::new(Particle::ExplosionEmitter, at)
                    .count(6)
                    .spread(Vector {
                        x: 1.8,
                        y: 1.2,
                        z: 1.8,
                    }),
            );
            world.particle.spawn(
                ParticleBurst::new(Particle::Portal, at)
                    .count(160)
                    .spread(Vector {
                        x: 1.5,
                        y: 1.5,
                        z: 1.5,
                    })
                    .speed(0.9),
            );
            player.sound.play(
                SoundCue::new(Sound::EntityGenericExplode)
                    .volume(1.4)
                    .pitch(0.8),
            );
            tokio::time::sleep(PORTAL_LAUNCH_DELAY).await;
            player.movement.set_velocity(launch);
        });
    }

    fn join_sheepwars(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            player.inventory.close();
            if let Some(orch) = lobby.state.orchestrator.clone() {
                match orch.toggle_queue(player.uuid, &player.name).await {
                    Ok(ToggleQueueOutcome::Joined(resp)) => {
                        let count = match orch.fetch_status().await {
                            Ok(snap) => snap.queue_len,
                            Err(_) => resp.total_queued,
                        };
                        lobby.state.menus.refresh_games_menu(count);
                        lobby.state.npcs.update_sheepwars_pedestal_label(count);
                        if resp.status == QueueJoinStatus::Transferring {
                            player.sound.play(
                                SoundCue::new(Sound::EntityEndermanTeleport)
                                    .volume(0.85)
                                    .pitch(1.15),
                            );
                            let target = resp.target_server.as_deref().unwrap_or("sheepwars");
                            player.message.send(format!(
                                "§d§lSheepWars §8» §aPartie trouvée ! Transfert vers §f{target}§a…"
                            ));
                        } else {
                            player.sound.play(
                                SoundCue::new(Sound::EntityExperienceOrbPickup)
                                    .volume(0.8)
                                    .pitch(1.3),
                            );
                            player.action_bar.send(format!(
                                "§d§lSheepWars §8• §aFile #{}/{} §7(min §f{}§7 • §b{}/{} srv§7)",
                                resp.position,
                                resp.total_queued.max(1),
                                resp.min_players,
                                resp.ready_servers,
                                resp.max_servers
                            ));
                            player.message.send(format!(
                                "§d§lSheepWars §8» §aFile d'attente rejointe (§f#{}§a, §f{}§a/§f{} min§a — §b{}§a prêt(s) / §f{} max§a). Recliquez pour quitter.",
                                resp.position,
                                resp.total_queued,
                                resp.min_players,
                                resp.ready_servers,
                                resp.max_servers
                            ));
                        }
                        lobby.refresh_single_player_ui(&player).await;
                    }
                    Ok(ToggleQueueOutcome::Left(resp)) => {
                        let count = match orch.fetch_status().await {
                            Ok(snap) => snap.queue_len,
                            Err(_) => resp.total_queued,
                        };
                        lobby.state.menus.refresh_games_menu(count);
                        lobby.state.npcs.update_sheepwars_pedestal_label(count);
                        player
                            .message
                            .send("§d§lSheepWars §8» §7Vous avez quitté la file d'attente.");
                        lobby.refresh_single_player_ui(&player).await;
                    }
                    Err(err) => {
                        tracing::warn!("Orchestrateur SheepWars injoignable : {err}");
                        player.message.send(format!(
                            "§cService de file d'attente SheepWars indisponible ({err})."
                        ));
                    }
                }
                return;
            }

            if let Some((host, port)) = lobby.state.config.sheepwars_transfer.clone() {
                let probe_target = lobby
                    .state
                    .config
                    .sheepwars_backend
                    .clone()
                    .unwrap_or_else(|| format!("{host}:{port}"));
                let backend_up = tokio::time::timeout(
                    Duration::from_millis(250),
                    tokio::net::TcpStream::connect(&probe_target),
                )
                .await
                .is_ok_and(|res| res.is_ok());

                if backend_up {
                    player
                        .message
                        .send("§d§lSheepWars §8» §7Transfert vers le serveur §fSheepWars§7…");
                    if let Err(err) = player.connection.transfer(host, port).await {
                        player.message.send(format!(
                            "§cImpossible de transférer vers SheepWars ({err})."
                        ));
                    }
                    return;
                }
            }

            if let Some(world_name) = lobby.state.config.sheepwars_world.clone() {
                let target_loc = Location {
                    world: world_name.clone(),
                    position: Position {
                        x: 0.5,
                        y: 65.0,
                        z: 0.5,
                    },
                    rotation: Rotation::default(),
                };
                if player.location.teleport_to(&target_loc).await.is_ok() {
                    player
                        .sound
                        .play(SoundCue::new(Sound::EntityEndermanTeleport));
                    player
                        .message
                        .send("§d§lSheepWars §8» §aBienvenue dans l'arène !");
                    return;
                }
            }

            // Local waiting queue fallback while colleague builds the SheepWars arena plugin
            let (joined, count) = {
                let mut q = lobby
                    .state
                    .sheepwars_queue
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                let joined = if q.contains(&player.uuid) {
                    q.remove(&player.uuid);
                    false
                } else {
                    q.insert(player.uuid);
                    true
                };
                (joined, q.len())
            };

            lobby.state.menus.refresh_games_menu(count);
            lobby.state.npcs.update_sheepwars_pedestal_label(count);

            if joined {
                player.sound.play(
                    SoundCue::new(Sound::EntityExperienceOrbPickup)
                        .volume(0.8)
                        .pitch(1.3),
                );
                player.action_bar.send(format!(
                    "§d§lSheepWars §8• §aFile d'attente rejointe (§f{count} §ajoueur(s))"
                ));
                player.message.send(format!(
                    "§d§lSheepWars §8» §aVous êtes inscrit dans la file d'attente (§f{count}§a prêt(s)). Recliquez pour quitter."
                ));
            } else {
                player
                    .message
                    .send("§d§lSheepWars §8» §7Vous avez quitté la file d'attente.");
            }
            lobby.refresh_single_player_ui(&player).await;
        });
    }

    fn teleport_to_poi(&self, player: Player, poi_key: &str) {
        let layout = self.state.layout.snapshot();
        let target = layout.pois.get(poi_key).copied().unwrap_or(layout.spawn);
        tokio::spawn(async move {
            player.inventory.close();
            if player
                .location
                .teleport_facing(target.facing())
                .await
                .is_ok()
            {
                player.sound.play(
                    SoundCue::new(Sound::EntityEndermanTeleport)
                        .volume(0.65)
                        .pitch(1.1),
                );
            }
        });
    }

    fn toggle_visibility(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let prefs = lobby
                .state
                .prefs
                .update(player.uuid, |p| p.players_visible = !p.players_visible);

            if let Ok(others) = player.server().get_players().await {
                for other in others {
                    if other.uuid == player.uuid {
                        continue;
                    }
                    if prefs.players_visible {
                        player.visibility.show(&other);
                    } else {
                        player.visibility.hide(&other);
                    }
                }
            }

            let claims = lobby.state.auth.claims_of(player.uuid);
            apply_lobby_hotbar(&player, claims.as_ref(), &prefs);
            lobby
                .state
                .menus
                .refresh_open_options(player.uuid, &prefs, claims.as_ref());

            player.sound.play(
                SoundCue::new(Sound::UiButtonClick)
                    .volume(0.7)
                    .pitch(if prefs.players_visible { 1.3 } else { 0.8 }),
            );
            player.action_bar.send(if prefs.players_visible {
                "§a👁 Joueurs du Hub affichés"
            } else {
                "§7👁 Joueurs du Hub masqués"
            });
        });
    }

    fn toggle_flight_for(&self, player: &Player) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.flight_enabled = !p.flight_enabled);
        updated.apply_movement(player);
        let claims = self.state.auth.claims_of(player.uuid);
        self.state
            .menus
            .refresh_open_options(player.uuid, &updated, claims.as_ref());
        self.state.menus.refresh_open_gadgets(player.uuid, &updated);

        if updated.flight_enabled {
            player.sound.play(
                SoundCue::new(Sound::EntityBatTakeoff)
                    .volume(0.75)
                    .pitch(1.2),
            );
            player.world.particle.spawn(
                ParticleBurst::new(Particle::Cloud, player.position)
                    .count(12)
                    .spread(Vector {
                        x: 0.35,
                        y: 0.25,
                        z: 0.35,
                    })
                    .speed(0.05),
            );
            player
                .action_bar
                .send("§b✈ Vol dans le Lobby ACTIVÉ §7(Double-espace)");
            player.message.send(
                "§b✈ §aVol dans le Lobby activé ! Appuyez deux fois sur Espace pour décoller.",
            );
        } else {
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.7).pitch(0.75));
            player.action_bar.send("§7✈ Vol dans le Lobby DÉSACTIVÉ");
            player.message.send("§7✈ Vol dans le Lobby désactivé.");
        }
    }

    fn cycle_speed_for(&self, player: &Player) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.speed_tier = p.speed_tier.next());
        updated.apply_movement(player);
        let claims = self.state.auth.claims_of(player.uuid);
        self.state
            .menus
            .refresh_open_options(player.uuid, &updated, claims.as_ref());
        self.state.menus.refresh_open_gadgets(player.uuid, &updated);

        player.sound.play(
            SoundCue::new(Sound::EntityExperienceOrbPickup)
                .volume(0.75)
                .pitch(1.25),
        );
        player
            .action_bar
            .send(format!("§e⚡ Vitesse : {}", updated.speed_tier.label()));
        player.message.send(format!(
            "§e⚡ Vitesse réglée sur : {}",
            updated.speed_tier.label()
        ));
    }

    fn cycle_trail_for(&self, player: &Player) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.aura_trail = p.aura_trail.next());
        self.state.menus.refresh_open_gadgets(player.uuid, &updated);
        player.sound.play(
            SoundCue::new(Sound::EntityExperienceOrbPickup)
                .volume(0.75)
                .pitch(1.35),
        );
        if let Some(particle) = updated.aura_trail.particle() {
            player.world.particle.spawn(
                ParticleBurst::new(particle, player.position)
                    .count(10)
                    .spread(Vector {
                        x: 0.35,
                        y: 0.4,
                        z: 0.35,
                    })
                    .speed(0.03),
            );
        }
        player
            .action_bar
            .send(format!("§d✦ Traînée : {}", updated.aura_trail.label()));
    }

    fn equip_gadget_for(&self, player: &Player, gadget: Option<LobbyGadget>) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.equipped_gadget = gadget);
        let claims = self.state.auth.claims_of(player.uuid);
        apply_lobby_hotbar(player, claims.as_ref(), &updated);
        self.state.menus.refresh_open_gadgets(player.uuid, &updated);

        player.sound.play(
            SoundCue::new(Sound::EntityItemPickup)
                .volume(0.75)
                .pitch(1.2),
        );
        match gadget {
            Some(g) => {
                player
                    .action_bar
                    .send(format!("{} §aéquipé ! §7(Slot 7 · Clic droit)", g.title()));
                player.message.send(format!(
                    "§b✦ Gadget équipé : {} §7— clic droit en main (slot 7) pour l'utiliser !",
                    g.title()
                ));
            }
            None => {
                player.action_bar.send("§7Gadget retiré de votre barre.");
            }
        }
    }

    fn open_gadgets_menu_for(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let cur_prefs = lobby.state.prefs.get(player.uuid);
            let _ = lobby
                .state
                .menus
                .open_gadgets(&player.server(), &player, &cur_prefs)
                .await;
        });
    }

    fn handle_menu_click(&self, player: Player, clicked_inv: Option<&InventoryRef>, slot: i32) {
        if self.state.menus.is_games_inventory(clicked_inv) {
            if let Some(action) = decode_games_click(slot) {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                match action {
                    GamesMenuAction::JoinSheepWars => self.join_sheepwars(player),
                    GamesMenuAction::ComingSoon(game) => {
                        player.inventory.close();
                        player.message.send(format!(
                            "§6§lFerrisCraft §8» §e{game} §7arrive très prochainement sur le réseau !"
                        ));
                    }
                    GamesMenuAction::OpenGadgets => self.open_gadgets_menu_for(player),
                    GamesMenuAction::TeleportPoi(poi) => self.teleport_to_poi(player, poi),
                    GamesMenuAction::OpenOptions => {
                        let lobby = self.clone();
                        tokio::spawn(async move {
                            let claims = lobby.state.auth.claims_of(player.uuid);
                            let cur = lobby.state.prefs.get(player.uuid);
                            let _ = lobby
                                .state
                                .menus
                                .open_options(&player.server(), &player, &cur, claims.as_ref())
                                .await;
                        });
                    }
                }
            }
            return;
        }

        if self
            .state
            .menus
            .is_player_options_inventory(player.uuid, clicked_inv)
        {
            if let Some(action) = decode_options_click(slot) {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                match action {
                    OptionsMenuAction::ToggleVisibility => self.toggle_visibility(player),
                    OptionsMenuAction::ToggleFlight => self.toggle_flight_for(&player),
                    OptionsMenuAction::CycleSpeed => self.cycle_speed_for(&player),
                    OptionsMenuAction::ToggleChatSounds => {
                        let updated = self
                            .state
                            .prefs
                            .update(player.uuid, |p| p.chat_sounds = !p.chat_sounds);
                        let claims = self.state.auth.claims_of(player.uuid);
                        self.state.menus.refresh_open_options(
                            player.uuid,
                            &updated,
                            claims.as_ref(),
                        );
                        player.action_bar.send(if updated.chat_sounds {
                            "§a🔔 Sons & Mentions tchat activés"
                        } else {
                            "§7🔕 Sons & Mentions tchat coupés"
                        });
                    }
                    OptionsMenuAction::FerrisKeyAction => {
                        player.inventory.close();
                        if self.state.auth.is_authenticated(player.uuid) {
                            self.state.auth.logout(player.uuid);
                            player
                                .message
                                .send("§eVotre compte FerrisKey a été délié de cette session.");
                            let lobby = self.clone();
                            tokio::spawn(async move {
                                lobby.refresh_single_player_ui(&player).await;
                            });
                        } else {
                            self.start_ferriskey_login(player);
                        }
                    }
                    OptionsMenuAction::BackToGames => {
                        let lobby = self.clone();
                        tokio::spawn(async move {
                            let _ = lobby.state.menus.open_games(&player).await;
                        });
                    }
                }
            }
            return;
        }

        if self
            .state
            .menus
            .is_player_gadgets_inventory(player.uuid, clicked_inv)
            && let Some(action) = decode_gadgets_click(slot)
        {
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
            match action {
                GadgetsMenuAction::Equip(gadget) => {
                    self.equip_gadget_for(&player, Some(gadget));
                }
                GadgetsMenuAction::UnequipGadget => {
                    self.equip_gadget_for(&player, None);
                }
                GadgetsMenuAction::ToggleFlight => self.toggle_flight_for(&player),
                GadgetsMenuAction::CycleTrail => self.cycle_trail_for(&player),
                GadgetsMenuAction::CycleSpeed => self.cycle_speed_for(&player),
                GadgetsMenuAction::BackToGames => {
                    let lobby = self.clone();
                    tokio::spawn(async move {
                        let _ = lobby.state.menus.open_games(&player).await;
                    });
                }
            }
        }
    }

    fn show_whoami(&self, player: &Player) {
        let mut msg =
            RichMessage::new().text("§8§m                                                  \n");
        match self.state.auth.claims_of(player.uuid) {
            Some(claims) => {
                let has_admin = player.permissions.has("ferriscraft.admin");
                let rank = Rank::resolve(Some(&claims), false, has_admin);
                let roles = if claims.roles.is_empty() {
                    "aucun rôle spécifique".to_owned()
                } else {
                    claims.roles.join(", ")
                };
                msg = msg
                    .text(format!("§a§l✔ Profil FerrisKey — §f{}\n", claims.username))
                    .text(format!("§7Grade Lobby : {}\n", rank.sidebar_label()))
                    .text(format!(
                        "§7E-mail : §f{}\n",
                        claims.email.as_deref().unwrap_or("non exposé")
                    ))
                    .text(format!("§7Rôles OIDC : §b{roles}\n  "))
                    .part(
                        Part::new("§c[Déconnecter ce compte]")
                            .hover("§7Cliquez pour exécuter /logout")
                            .click(Click::RunCommand("/logout".to_owned())),
                    );
            }
            None => {
                msg = msg.text("§eAucun compte FerrisKey Lobby n'est lié. ").part(
                    Part::new("§a§l[Se connecter]")
                        .hover("§7Lancer la connexion FerrisKey (/login)")
                        .click(Click::RunCommand("/login".to_owned())),
                );
            }
        }

        if self.state.npcs.has_capitainerie() && self.state.auth.uses_separate_capitainerie_auth() {
            let realm = &self.state.config.capitainerie_realm;
            msg = msg.text(format!("\n§7Capitainerie (realm §b{realm}§7) : "));
            match self.state.auth.capitainerie_claims_of(player.uuid) {
                Some(cap_claims) => {
                    msg = msg.text(format!("§a✔ {} ", cap_claims.username)).part(
                        Part::new("§c[Délier]")
                            .hover("§7Cliquez pour exécuter /caplogout")
                            .click(Click::RunCommand("/caplogout".to_owned())),
                    );
                }
                None => {
                    msg = msg.text("§eNon lié ").part(
                        Part::new("§b[/caplogin]")
                            .hover("§7Connecter votre compte Capitainerie")
                            .click(Click::RunCommand("/caplogin".to_owned())),
                    );
                }
            }
        }

        msg = msg.text("\n§8§m                                                  ");
        player.message.send_rich(msg);
    }

    async fn suggest_lobby_subcommands(&self, _player: &Player, typed: &str) -> Vec<String> {
        LOBBY_SUBCOMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(typed))
            .map(|cmd| (*cmd).to_owned())
            .collect()
    }

    fn store_task_result(&self, key: String, report: TaskResultReport) {
        let mut guard = self
            .state
            .task_results
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if guard.len() >= MAX_CACHED_RESULTS
            && !guard.contains_key(&key)
            && let Some(evict) = guard.keys().next().cloned()
        {
            guard.remove(&evict);
        }
        guard.insert(key, report);
    }

    fn spawn_run_watcher(
        &self,
        player: Player,
        agent: TeamAgent,
        task_title: String,
        started: RunStarted,
    ) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let Some(client) = lobby.state.npcs.capitainerie_client() else {
                return;
            };
            let server = player.server();
            let short_title = clip_text(&task_title, 28);
            let bar = server
                .boss_bars
                .create(
                    format!("§e⏳ {} §8• §f{short_title}", agent.name),
                    BarColor::Yellow,
                    BarStyle::Segmented10,
                    &[],
                )
                .await
                .ok();
            if let Some(bar) = &bar {
                bar.add_player(&player);
                bar.set_progress(0.08);
            }
            let mut attached_entity = Some(player.entity_id);

            let mut consecutive_errors = 0usize;
            for _ in 0..RUN_WATCH_MAX_POLLS {
                tokio::time::sleep(RUN_WATCH_POLL_INTERVAL).await;

                let online_player = server
                    .get_players()
                    .await
                    .ok()
                    .and_then(|list| list.into_iter().find(|p| p.uuid == player.uuid));

                match (&online_player, &bar) {
                    (Some(p), Some(bar)) if attached_entity != Some(p.entity_id) => {
                        bar.add_player(p);
                        attached_entity = Some(p.entity_id);
                    }
                    (None, _) => attached_entity = None,
                    _ => {}
                }

                let Ok(bearer) = lobby.state.auth.valid_capitainerie_token(player.uuid).await
                else {
                    break;
                };

                let detail = match client.run(&bearer, &started.run_id).await {
                    Ok(d) => {
                        consecutive_errors = 0;
                        d
                    }
                    Err(err) => {
                        tracing::debug!(
                            "Suivi run Capitainerie {} interrompu/transitoire : {err}",
                            started.run_id
                        );
                        consecutive_errors += 1;
                        if consecutive_errors >= 5 {
                            break;
                        }
                        continue;
                    }
                };

                if !detail.is_terminal() {
                    let progress_info = lobby.state.npcs.run_progress(&started.run_id);
                    if detail.status == "queued" {
                        if let Some(bar) = &bar {
                            bar.set_color(BarColor::Yellow);
                            bar.set_progress(0.08);
                            bar.set_title(format!(
                                "§e⏳ {} §8• §7En file — §f{short_title}",
                                agent.name
                            ));
                        }
                        if let Some(p) = &online_player {
                            p.action_bar.send(format!(
                                "§e⏳ {} §8• §7En file d'attente : §f{}",
                                agent.name,
                                clip_text(&task_title, 36)
                            ));
                        }
                    } else {
                        let (step_suffix, ratio) = match progress_info {
                            Some((step_kind, done, total)) => {
                                let step_text = step_kind
                                    .as_deref()
                                    .map(|k| format!(" §7· {}", step_name(k)))
                                    .unwrap_or_default();
                                let frac = if total > 0 {
                                    (f64::from(done) / f64::from(total)).clamp(0.12, 0.95)
                                } else {
                                    0.45
                                };
                                (step_text, frac)
                            }
                            None => (String::new(), 0.45),
                        };
                        if let Some(bar) = &bar {
                            bar.set_color(BarColor::Blue);
                            bar.set_progress(ratio);
                            bar.set_title(format!(
                                "§b⚒ {}{} §8• §f{short_title}",
                                agent.name, step_suffix
                            ));
                        }
                        if let Some(p) = &online_player {
                            p.action_bar.send(format!(
                                "§b⚒ {}{} §8• §f{}",
                                agent.name,
                                step_suffix,
                                clip_text(&task_title, 36)
                            ));
                        }
                    }
                    continue;
                }

                let card_id = if !started.card_id.is_empty() {
                    started.card_id.clone()
                } else {
                    detail.card_id.clone().unwrap_or_default()
                };
                let card = if !card_id.is_empty() {
                    client.card(&bearer, &card_id).await.ok()
                } else {
                    None
                };

                let summary_text = if detail.is_succeeded() {
                    card.as_ref()
                        .and_then(|c| extract_agent_reply(c, &agent.slug))
                        .unwrap_or_else(|| "La tâche a été exécutée avec succès.".to_owned())
                } else {
                    detail
                        .failure
                        .clone()
                        .filter(|f| !f.trim().is_empty())
                        .or_else(|| {
                            card.as_ref()
                                .and_then(|c| extract_agent_reply(c, &agent.slug))
                        })
                        .unwrap_or_else(|| {
                            "La mission s'est arrêtée sans détail d'erreur.".to_owned()
                        })
                };

                let resolved_title = detail
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| task_title.clone());
                let console_url = lobby.state.config.console_task_url(&card_id);
                let report = TaskResultReport {
                    run_id: started.run_id.clone(),
                    card_id,
                    agent_name: agent.name.clone(),
                    agent_slug: agent.slug.clone(),
                    task_title: resolved_title.clone(),
                    status: detail.status.clone(),
                    summary_text,
                    console_url,
                };
                let click_key = click_slug(&started.run_id);
                lobby.store_task_result(click_key.clone(), report.clone());

                let clipped_title = clip_text(&resolved_title, 32);
                if let Some(bar) = &bar {
                    match detail.status.as_str() {
                        "succeeded" => {
                            bar.set_color(BarColor::Green);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§a§l✔ {} — Terminé : §f{clipped_title}",
                                agent.name
                            ));
                        }
                        "cancelled" => {
                            bar.set_color(BarColor::Yellow);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§e⚠ {} — Annulé : §f{clipped_title}",
                                agent.name
                            ));
                        }
                        _ => {
                            bar.set_color(BarColor::Red);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§c§l✖ {} — Échoué : §f{clipped_title}",
                                agent.name
                            ));
                        }
                    }
                }

                if let Some(pos) = lobby.state.npcs.agent_position(&agent.id).await {
                    let burst_pos = Position {
                        x: pos.x,
                        y: pos.y + 1.4,
                        z: pos.z,
                    };
                    if detail.is_succeeded() {
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::HappyVillager, burst_pos)
                                .count(18)
                                .spread(Vector {
                                    x: 0.45,
                                    y: 0.45,
                                    z: 0.45,
                                })
                                .speed(0.05),
                        );
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::EndRod, burst_pos)
                                .count(10)
                                .spread(Vector {
                                    x: 0.35,
                                    y: 0.4,
                                    z: 0.35,
                                })
                                .speed(0.08),
                        );
                    } else {
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::Smoke, burst_pos)
                                .count(16)
                                .spread(Vector {
                                    x: 0.35,
                                    y: 0.35,
                                    z: 0.35,
                                })
                                .speed(0.04),
                        );
                    }
                }

                if let Some(p) = online_player {
                    if detail.is_succeeded() {
                        p.sound.play(
                            SoundCue::new(Sound::EntityPlayerLevelup)
                                .volume(0.9)
                                .pitch(1.15),
                        );
                        p.title.send(
                            Title::new(format!("§a§l✔ {} A TERMINÉ", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 60, 20),
                        );
                        p.action_bar
                            .send(format!("§a✔ {} a terminé : §f{clipped_title}", agent.name));
                    } else if detail.status == "cancelled" {
                        p.sound.play(
                            SoundCue::new(Sound::EntityVillagerNo)
                                .volume(0.85)
                                .pitch(1.0),
                        );
                        p.title.send(
                            Title::new(format!("§e⚠ {} — ANNULÉ", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 50, 20),
                        );
                        p.action_bar
                            .send(format!("§e⚠ {} — mission annulée", agent.name));
                    } else {
                        p.sound.play(
                            SoundCue::new(Sound::EntityVillagerNo)
                                .volume(0.9)
                                .pitch(0.9),
                        );
                        p.title.send(
                            Title::new(format!("§c§l✖ {} — ÉCHEC", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 60, 20),
                        );
                        p.action_bar
                            .send(format!("§c✖ {} — mission échouée", agent.name));
                    }
                    p.message
                        .send_rich(build_run_completion_message(&report, &click_key));
                }

                tokio::time::sleep(RUN_FINISH_BAR_LINGER).await;
                break;
            }

            if let Some(bar) = bar {
                bar.remove();
            }
        });
    }
}

fn json_field(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[listeners]
impl FerrisCraftLobby {
    async fn on_enable(&self, server: &Server) {
        server.world.pvp.set(false);
        server.world.difficulty.set(Difficulty::Peaceful);
        server.world.weather.set(Weather::Clear);
        server.world.time.set(6000);

        // Lock environmental rules so custom trees, braziers, ice, and decorations never decay or burn
        server.command.dispatch("gamerule random_tick_speed 0");
        server
            .command
            .dispatch("gamerule fire_spread_radius_around_player 0");
        server.command.dispatch("gamerule advance_time false");
        server.command.dispatch("gamerule mob_griefing false");

        if let Err(err) = self.state.menus.init(server).await {
            tracing::warn!("Impossible de créer le menu des jeux : {err}");
        }

        self.state.npcs.spawn_all(server, 0).await;

        let npcs = Arc::clone(&self.state.npcs);
        let auth = Arc::clone(&self.state.auth);
        let srv_npcs = server.clone();
        tokio::spawn(async move {
            npcs.run_background_loops(srv_npcs, auth).await;
        });

        let lobby_hud = self.clone();
        let srv_hud = server.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HUD_REFRESH_INTERVAL);
            loop {
                ticker.tick().await;
                if let Some(orch) = &lobby_hud.state.orchestrator
                    && let Ok(snap) = orch.fetch_status().await
                {
                    lobby_hud.state.menus.refresh_games_menu(snap.queue_len);
                    lobby_hud
                        .state
                        .npcs
                        .update_sheepwars_pedestal_label(snap.queue_len);
                    if !snap.queued_players.is_empty()
                        && let Ok(online) = srv_hud.get_players().await
                    {
                        for qp in &snap.queued_players {
                            if let Some(p) = online.iter().find(|p| p.uuid == qp.player_uuid) {
                                let state_hint = if snap.ready_servers > 0 {
                                    format!("§b{} serveur(s) prêt(s)", snap.ready_servers)
                                } else if snap.starting_servers > 0 {
                                    "§eDémarrage serveur…".to_owned()
                                } else {
                                    format!("§7{}/{} max", snap.in_game_servers, snap.max_replicas)
                                };
                                p.action_bar.send(format!(
                                    "§d§lSheepWars §8• §aFile #{}/{} §7(min §f{}§7) §8• {state_hint}",
                                    qp.position, snap.queue_len, snap.min_players
                                ));
                            }
                        }
                    }
                }
                let metrics = lobby_hud.collect_metrics(&srv_hud).await;
                lobby_hud
                    .state
                    .hud
                    .refresh_all(&srv_hud, &lobby_hud.state.auth, metrics)
                    .await;
            }
        });

        tracing::info!(
            "FerrisCraft Lobby initialisé (FerrisKey={} realm={}, Capitainerie={:?} realm={}, Orchestrator={:?})",
            self.state.config.ferriskey_base_url,
            self.state.config.ferriskey_realm,
            self.state.config.capitainerie_url,
            self.state.config.capitainerie_realm,
            self.state.config.orchestrator_url,
        );
    }

    async fn on_player_join(&self, event: &PlayerJoin) {
        let player = event.player().clone();
        let lobby = self.clone();

        player.game_mode.set(GameMode::Adventure);
        player.health.set(20.0);
        player.health.set_fire_ticks(0);
        player.food.set_level(20);
        player.food.set_saturation(20.0);
        player.inventory.clear();
        player.inventory.set_held_slot(0);

        let prefs = lobby.state.prefs.get(player.uuid);
        prefs.apply_movement(&player);

        let claims = lobby.state.auth.claims_of(player.uuid);
        apply_lobby_hotbar(&player, claims.as_ref(), &prefs);

        player.title.send(
            Title::new("§6§lFERRISCRAFT")
                .subtitle(format!("§dBienvenue sur le Hub, §f{}", player.name)),
        );
        player.sound.play(
            SoundCue::new(Sound::EntityPlayerLevelup)
                .volume(0.75)
                .pitch(1.1),
        );

        match &claims {
            Some(user) => {
                player.message.send(format!(
                    "§6§lFerrisCraft §8» §7Bon retour §a✔ {} §7! Utilisez la §eBoussole §7ou les §dPlots §7pour jouer.",
                    user.username
                ));
            }
            None => {
                player.message.send_rich(
                    RichMessage::new()
                        .text("§6§lFerrisCraft §8» §7Bienvenue ! Liez votre compte personnel avec ")
                        .part(
                            Part::new("§b§l[/login]")
                                .hover("§eCliquez pour connecter votre compte FerrisKey")
                                .click(Click::RunCommand("/login".to_owned())),
                        )
                        .text(" §7pour débloquer votre grade et la Capitainerie."),
                );
            }
        }

        tokio::spawn(async move {
            let spawn = lobby.state.layout.snapshot().spawn;
            let _ = player.location.teleport_facing(spawn.facing()).await;

            let srv = player.server();
            if let Err(err) = lobby.state.hud.attach_player(&srv, &player).await {
                tracing::warn!(
                    "Impossible d'attacher le scoreboard de {}: {err}",
                    player.name
                );
            }

            // Respect existing players' visibility settings
            if let Ok(online) = srv.get_players().await {
                let hiding_set: HashSet<Uuid> = lobby
                    .state
                    .prefs
                    .viewers_hiding_others()
                    .into_iter()
                    .collect();
                for other in &online {
                    if other.uuid == player.uuid {
                        continue;
                    }
                    if hiding_set.contains(&other.uuid) {
                        other.visibility.hide(&player);
                    }
                    if !prefs.players_visible {
                        player.visibility.hide(other);
                    }
                }
            }

            lobby.refresh_single_player_ui(&player).await;
        });
    }

    async fn on_player_leave(&self, event: &PlayerLeave) {
        let player = event.player();
        self.state.auth.cancel_pending(player.uuid);
        self.state.menus.discard_options(player.uuid);
        self.state.menus.discard_gadgets(player.uuid);
        self.state.gadgets.clear_player(player.uuid);
        self.state.hud.detach_player(player.uuid, &player.name);
        self.state.npcs.leave_captain_chat(player.uuid);
        self.state.chat.clear_player(player.uuid);
        self.state
            .last_steps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player.uuid);
        self.state
            .portal_last_blast
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player.uuid);

        if let Some(orch) = self.state.orchestrator.clone() {
            let uuid = player.uuid;
            tokio::spawn(async move {
                let _ = orch.leave_if_queued(uuid).await;
            });
        }

        let new_len = {
            let mut q = self
                .state
                .sheepwars_queue
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if q.remove(&player.uuid) {
                Some(q.len())
            } else {
                None
            }
        };
        if let Some(count) = new_len {
            self.state.menus.refresh_games_menu(count);
            self.state.npcs.update_sheepwars_pedestal_label(count);
        }
    }

    async fn on_player_interact(&self, event: &mut PlayerInteract) {
        let player = event.player().clone();
        let prefs = self.state.prefs.get(player.uuid);
        let is_right_click = matches!(
            event.action(),
            Action::RightClickAir | Action::RightClickBlock
        );

        if !prefs.build_mode {
            event.set_use_interacted_block(EventResult::Deny);
            event.set_use_item_in_hand(EventResult::Deny);
        }

        if !is_right_click || event.hand() == Some(EquipmentSlot::OffHand) {
            return;
        }

        match event.material() {
            Material::Compass => {
                let lobby = self.clone();
                tokio::spawn(async move {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    let _ = lobby.state.menus.open_games(&player).await;
                });
            }
            Material::WhiteWool if !prefs.build_mode => {
                self.join_sheepwars(player);
            }
            Material::AmethystShard if !prefs.build_mode => {
                self.start_ferriskey_login(player);
            }
            Material::Emerald if !prefs.build_mode => {
                self.show_whoami(&player);
            }
            Material::Chest if !prefs.build_mode => {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                self.open_gadgets_menu_for(player);
            }
            Material::LimeDye | Material::GrayDye if !prefs.build_mode => {
                self.toggle_visibility(player);
            }
            Material::Comparator if !prefs.build_mode => {
                let lobby = self.clone();
                tokio::spawn(async move {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    let claims = lobby.state.auth.claims_of(player.uuid);
                    let cur_prefs = lobby.state.prefs.get(player.uuid);
                    let _ = lobby
                        .state
                        .menus
                        .open_options(&player.server(), &player, &cur_prefs, claims.as_ref())
                        .await;
                });
            }
            other if !prefs.build_mode => {
                if let Some(gadget) = LobbyGadget::from_material(&other)
                    && prefs.equipped_gadget == Some(gadget)
                {
                    self.state.gadgets.activate(player, gadget);
                }
            }
            _ => {}
        }
    }

    // Locked custom menus (Games, Options, Gadgets) are refused by rspigot and dispatched
    // only to Observe (`&InventoryClick`) handlers in Phase::Settled.
    async fn on_menu_click(&self, event: &InventoryClick) {
        self.handle_menu_click(
            event.player().clone(),
            event.clicked_inventory(),
            event.slot(),
        );
    }

    async fn on_inventory_click(&self, event: &mut InventoryClick) {
        let player = event.player();
        // Prevent moving hotbar items if top view is a custom menu or player is not in build mode
        let top_is_custom = event.inventory().custom().is_some();
        if top_is_custom || !self.state.prefs.get(player.uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_inventory_close(&self, event: &InventoryClose) {
        self.state
            .menus
            .handle_close(event.player().uuid, event.inventory());
    }

    async fn on_player_interact_at_entity(&self, event: &PlayerInteractAtEntity) {
        if event.hand() != EquipmentSlot::Hand {
            return;
        }
        let player = event.player().clone();
        let Some(target) = self
            .state
            .npcs
            .lookup_entity(event.right_clicked().entity_id)
        else {
            return;
        };

        match target {
            NpcClickTarget::Pedestal(spec) => {
                if spec.active && spec.game_id == "sheepwars" {
                    self.join_sheepwars(player);
                } else {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.7)
                            .pitch(0.9),
                    );
                    player.message.send(format!(
                        "{} §8» §7Ce mini-jeu est en préparation et ouvrira prochainement !",
                        spec.title
                    ));
                }
            }
            NpcClickTarget::Ambient { npc, spec, line } => {
                npc.look_at(player.position);
                npc.swing_main_hand();
                player.sound.play(
                    SoundCue::new(Sound::EntityVillagerYes)
                        .volume(0.8)
                        .pitch(1.05),
                );
                player
                    .message
                    .send(format!("{} §8» §f{line}", spec.display_name));
            }
            NpcClickTarget::Agent(agent) => {
                if !self.state.auth.is_capitainerie_authenticated(player.uuid) {
                    self.prompt_capitainerie_login_required(
                        &player,
                        &format!("l'agent {}", agent.name),
                    );
                    return;
                }
                if agent.paused {
                    player
                        .message
                        .send(format!("§c{} est actuellement en pause.", agent.name));
                    return;
                }
                let intro = agent.role.clone().unwrap_or_else(|| {
                    "Confiez une mission avec votre compte FerrisKey.".to_owned()
                });
                player.dialog.show(
                    Dialog::new(format!("§b§lMission pour {}", agent.name))
                        .line(intro)
                        .input(DialogInput::new("title", "Titre").max_length(MAX_TASK_TITLE))
                        .input(
                            DialogInput::new("description", "Description")
                                .multiline()
                                .max_length(MAX_TASK_DESC),
                        )
                        .button(DialogButton::new(
                            "§aLancer la mission",
                            format!("{RUN_BUTTON_PREFIX}{}", click_slug(&agent.slug)),
                        ))
                        .button(DialogButton::new("§cAnnuler", CANCEL_BUTTON_ID)),
                );
            }
            NpcClickTarget::Captain => {
                if !self.state.npcs.has_capitainerie() {
                    player.message.send(
                        "§6Capitaine §8» §7La passerelle Capitainerie n'est pas configurée sur ce lobby (CAPITAINERIE_URL).",
                    );
                    return;
                }
                if !self.state.auth.is_capitainerie_authenticated(player.uuid) {
                    self.prompt_capitainerie_login_required(&player, CAPTAIN_DISPLAY_NAME);
                    return;
                }
                if self.state.npcs.toggle_captain_chat(player.uuid) {
                    player.sound.play(SoundCue::new(Sound::BlockNoteBlockChime));
                    player.message.send(format!(
                        "§6§l{CAPTAIN_DISPLAY_NAME} §8» §aCanal privé ouvert sous votre identité FerrisKey. §7Écrivez dans le tchat ou tapez §fquitter§7."
                    ));
                } else {
                    player
                        .message
                        .send("§6Capitaine §8» §7Canal privé fermé, retour au tchat général.");
                }
            }
        }
    }

    async fn on_player_custom_click(&self, event: &PlayerCustomClick) {
        let click_id = event.click_id();
        let player = event.player().clone();

        if click_id == CANCEL_BUTTON_ID {
            return;
        }
        if click_id == LOGIN_BUTTON_ID {
            self.start_ferriskey_login(player);
            return;
        }
        if click_id == CAP_LOGIN_BUTTON_ID {
            self.start_capitainerie_login(player);
            return;
        }

        if let Some(key) = click_id.strip_prefix(RESULT_BUTTON_PREFIX) {
            let report = self
                .state
                .task_results
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(key)
                .cloned();
            if let Some(report) = report {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                player.dialog.show(build_run_result_dialog(&report));
            } else {
                player.message.send(
                    "§eCe rapport n'est plus en cache. Consultez-le directement sur Capitainerie.",
                );
            }
            return;
        }

        let Some(slug) = click_id.strip_prefix(RUN_BUTTON_PREFIX) else {
            return;
        };
        let Some(agent) = self.state.npcs.agent_by_click_slug(slug) else {
            player.message.send("§cCet agent n'est plus disponible.");
            return;
        };

        let payload: serde_json::Value = event
            .data()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        let title = json_field(&payload, "title");
        let description = json_field(&payload, "description");
        if title.is_empty() {
            player
                .message
                .send("§cVeuillez indiquer un titre pour lancer la mission.");
            return;
        }

        let lobby = self.clone();
        tokio::spawn(async move {
            let bearer = match lobby.state.auth.valid_capitainerie_token(player.uuid).await {
                Ok(token) => token,
                Err(err) => {
                    player.message.send(format!("§c{err}"));
                    return;
                }
            };

            match lobby
                .state
                .npcs
                .start_agent_task(&bearer, &agent, &title, &description)
                .await
            {
                Ok(started) => {
                    player
                        .sound
                        .play(SoundCue::new(Sound::BlockAnvilUse).volume(0.5).pitch(1.3));
                    player.message.send(format!(
                        "§a✔ {} rejoint son établi pour traiter : §f{title}",
                        agent.name
                    ));
                    lobby.spawn_run_watcher(player, agent, title, started);
                }
                Err(err) => {
                    tracing::warn!("Échec de lancement de tâche sur {}: {err}", agent.slug);
                    player.message.send(err.explain());
                }
            }
        });
    }

    async fn on_async_player_chat(&self, event: &mut AsyncPlayerChat) {
        event.cancel();
        let player = event.player().clone();
        let text = event.message().trim().to_owned();
        if text.is_empty() {
            return;
        }

        // Priority 1: Private SSE conversation with the Captain
        if self.state.npcs.is_in_captain_chat(player.uuid) {
            if NpcDirector::is_quit_word(&text) {
                self.state.npcs.leave_captain_chat(player.uuid);
                player
                    .message
                    .send("§6Capitaine §8» §7Conversation terminée, retour au tchat du Hub.");
                return;
            }
            let conversation = match self.state.npcs.begin_captain_turn(player.uuid) {
                CaptainTurn::Busy => {
                    player.message.send(format!(
                        "§7Le {CAPTAIN_DISPLAY_NAME} rédige encore sa réponse, patientez…"
                    ));
                    return;
                }
                CaptainTurn::Ready(conv) => conv,
            };

            player
                .message
                .send(format!("§bVous §8→ §6Capitaine §8» §f{text}"));
            let lobby = self.clone();
            tokio::spawn(async move {
                let bearer = match lobby.state.auth.valid_capitainerie_token(player.uuid).await {
                    Ok(token) => token,
                    Err(err) => {
                        lobby.state.npcs.leave_captain_chat(player.uuid);
                        player.message.send(format!("§c{err}"));
                        return;
                    }
                };

                match lobby
                    .state
                    .npcs
                    .chat_with_captain(&bearer, &player, conversation.as_deref(), &text)
                    .await
                {
                    Ok(outcome) => {
                        for (_, card_title) in &outcome.cards {
                            player
                                .message
                                .send(format!("§a+ Carte créée : §f{card_title}"));
                        }
                        lobby
                            .state
                            .npcs
                            .end_captain_turn(player.uuid, Some(outcome.conversation));
                    }
                    Err(err) => {
                        tracing::warn!("Erreur conversation Capitaine : {err}");
                        player.message.send(err.explain());
                        lobby.state.npcs.end_captain_turn(player.uuid, None);
                    }
                }
            });
            return;
        }

        // Priority 2: Formatted Lobby Chat with FerrisKey Badges & @Mentions
        let claims = self.state.auth.claims_of(player.uuid);
        let has_admin = player.permissions.has("ferriscraft.admin");
        let prelim_rank = Rank::resolve(claims.as_ref(), false, has_admin);
        if let Err(wait) = self.state.chat.check_cooldown(player.uuid, prelim_rank) {
            player.message.send(format!(
                "§cVeuillez patienter {:.1}s avant d'envoyer un nouveau message.",
                wait.as_secs_f32()
            ));
            return;
        }

        let lobby = self.clone();
        tokio::spawn(async move {
            let srv = player.server();
            let is_op = player.get_state().await.is_ok_and(|s| s.op);
            let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
            let Ok(recipients) = srv.get_players().await else {
                return;
            };
            lobby.state.chat.broadcast_chat(
                &player,
                rank,
                claims.as_ref(),
                &text,
                &recipients,
                &lobby.state.prefs,
            );
        });
    }

    async fn on_block_break(&self, event: &mut BlockBreak) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_block_place(&self, event: &mut BlockPlace) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_player_drop_item(&self, event: &mut PlayerDropItem) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_player_swap_hand_items(&self, event: &mut PlayerSwapHandItems) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_entity_damage(&self, event: &mut EntityDamage) {
        event.cancel();
    }

    async fn on_food_level_change(&self, event: &mut FoodLevelChange) {
        event.cancel();
    }

    async fn on_leaves_decay(&self, event: &mut LeavesDecay) {
        event.cancel();
    }

    async fn on_block_fade(&self, event: &mut BlockFade) {
        event.cancel();
    }

    async fn on_block_burn(&self, event: &mut BlockBurn) {
        event.cancel();
    }

    async fn on_block_spread(&self, event: &mut BlockSpread) {
        event.cancel();
    }

    async fn on_block_grow(&self, event: &mut BlockGrow) {
        event.cancel();
    }

    async fn on_moisture_change(&self, event: &mut MoistureChange) {
        event.cancel();
    }

    async fn on_player_move(&self, event: &mut PlayerMove) {
        let Some(to) = event.to() else { return };
        let layout = self.state.layout.snapshot();

        // Rescue player if they jump off the floating island
        if to.position.y < layout.void_y {
            let player = event.player().clone();
            let spawn: Facing = layout.spawn.facing();
            tokio::spawn(async move {
                if player.location.teleport_facing(spawn).await.is_ok() {
                    player
                        .sound
                        .play(SoundCue::new(Sound::EntityEndermanTeleport));
                    player
                        .action_bar
                        .send("§e✦ Retour sur l'île de FerrisCraft !");
                }
            });
            return;
        }

        let from = event.from();
        let step = Vector {
            x: to.position.x - from.position.x,
            y: 0.0,
            z: to.position.z - from.position.z,
        };
        self.state
            .last_steps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(event.player().uuid, step);

        let prefs = self.state.prefs.get(event.player().uuid);
        if prefs.aura_trail != AuraTrail::None {
            self.state
                .gadgets
                .emit_trail_if_due(event.player(), prefs.aura_trail);
        }
    }

    async fn on_player_portal(&self, event: &mut PlayerPortal) {
        if event.cause() == PlayerTeleportCause::EndPortal {
            event.cancel();
        }
    }

    async fn on_entity_portal_enter(&self, event: &EntityPortalEnter) {
        if event.entity_type() != EntityType::Player || self.state.orchestrator.is_none() {
            return;
        }
        let entity = event.entity();
        let uuid = entity.uuid;
        let now = Instant::now();
        let debounced = {
            let mut guard = self
                .state
                .portal_last_blast
                .write()
                .unwrap_or_else(|e| e.into_inner());
            match guard.get(&uuid) {
                Some(last) if now.saturating_duration_since(*last) < PORTAL_DEBOUNCE => true,
                _ => {
                    guard.insert(uuid, now);
                    false
                }
            }
        };
        if debounced {
            return;
        }
        let step = self
            .state
            .last_steps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uuid)
            .copied();
        let at = entity.position;
        let server = entity.server();
        let lobby = self.clone();
        tokio::spawn(async move {
            let Ok(players) = server.get_players().await else {
                return;
            };
            if let Some(player) = players.into_iter().find(|p| p.uuid == uuid) {
                lobby.portal_blast(player, at, step, 0.0);
            }
        });
    }

    #[command("menu")]
    async fn cmd_menu(&self, ctx: Context) {
        let player = ctx.sender.clone();
        player
            .sound
            .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
        if let Err(err) = self.state.menus.open_games(&player).await {
            ctx.reply(format!("§cImpossible d'ouvrir le menu : {err}"));
        }
    }

    #[command("options")]
    async fn cmd_options(&self, ctx: Context) {
        let player = ctx.sender.clone();
        let claims = self.state.auth.claims_of(player.uuid);
        let prefs = self.state.prefs.get(player.uuid);
        if let Err(err) = self
            .state
            .menus
            .open_options(&player.server(), &player, &prefs, claims.as_ref())
            .await
        {
            ctx.reply(format!("§cImpossible d'ouvrir vos options : {err}"));
        }
    }

    #[command("gadgets")]
    async fn cmd_gadgets(&self, ctx: Context) {
        let player = ctx.sender.clone();
        player
            .sound
            .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
        let prefs = self.state.prefs.get(player.uuid);
        if let Err(err) = self
            .state
            .menus
            .open_gadgets(&player.server(), &player, &prefs)
            .await
        {
            ctx.reply(format!("§cImpossible d'ouvrir les gadgets : {err}"));
        }
    }

    #[command("fly")]
    async fn cmd_fly(&self, ctx: Context) {
        self.toggle_flight_for(&ctx.sender);
    }

    #[command("speed")]
    async fn cmd_speed(&self, ctx: Context) {
        self.cycle_speed_for(&ctx.sender);
    }

    #[command("spawn")]
    async fn cmd_spawn(&self, ctx: Context) {
        self.teleport_to_poi(ctx.sender.clone(), "spawn");
    }

    #[command("sheepwars")]
    async fn cmd_sheepwars(&self, ctx: Context) {
        self.join_sheepwars(ctx.sender.clone());
    }

    #[command("login")]
    async fn cmd_login(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if let Some(claims) = self.state.auth.claims_of(player.uuid) {
            ctx.reply(format!(
                "§a✔ Vous êtes déjà connecté en tant que §f{}§a. Utilisez §e/logout §apour changer de compte.",
                claims.username
            ));
            return;
        }
        self.start_ferriskey_login(player);
    }

    #[command("logout")]
    async fn cmd_logout(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if self.state.auth.logout(player.uuid) {
            if !self.state.auth.uses_separate_capitainerie_auth() {
                self.state.npcs.leave_captain_chat(player.uuid);
            }
            ctx.reply("§eVotre compte FerrisKey a été déconnecté.");
            self.refresh_single_player_ui(&player).await;
        } else {
            ctx.reply("§7Aucun compte FerrisKey n'était actif.");
        }
    }

    #[command("caplogin")]
    async fn cmd_caplogin(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if !self.state.npcs.has_capitainerie() {
            ctx.reply("§7La passerelle Capitainerie n'est pas configurée sur ce lobby (CAPITAINERIE_URL).");
            return;
        }
        if !self.state.auth.uses_separate_capitainerie_auth() {
            if let Some(claims) = self.state.auth.claims_of(player.uuid) {
                ctx.reply(format!(
                    "§a✔ Capitainerie partage votre compte FerrisKey (§f{}§a).",
                    claims.username
                ));
            } else {
                self.start_ferriskey_login(player);
            }
            return;
        }
        if let Some(claims) = self.state.auth.capitainerie_claims_of(player.uuid) {
            ctx.reply(format!(
                "§a✔ Déjà connecté à Capitainerie (realm §b{}§a) en tant que §f{}§a. Utilisez §e/caplogout §apour changer de compte.",
                self.state.config.capitainerie_realm,
                claims.username
            ));
            return;
        }
        self.start_capitainerie_login(player);
    }

    #[command("caplogout")]
    async fn cmd_caplogout(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if !self.state.auth.uses_separate_capitainerie_auth() {
            ctx.reply("§7Capitainerie utilise la même session que le Lobby (/logout).");
            return;
        }
        if self.state.auth.capitainerie_logout(player.uuid) {
            self.state.npcs.leave_captain_chat(player.uuid);
            ctx.reply("§eVotre compte Capitainerie a été déconnecté.");
        } else {
            ctx.reply("§7Aucun compte Capitainerie n'était actif.");
        }
    }

    #[command("whoami")]
    async fn cmd_whoami(&self, ctx: Context) {
        self.show_whoami(&ctx.sender);
    }

    #[command("mp")]
    async fn cmd_mp(&self, ctx: Context, target: Player, message: String) {
        let sender = &ctx.sender;
        let body = message.trim();
        if body.is_empty() {
            ctx.reply("§cMessage vide.");
            return;
        }
        if target.uuid == sender.uuid {
            ctx.reply("§eVous ne pouvez pas vous écrire à vous-même.");
            return;
        }
        self.state
            .chat
            .send_whisper(sender, &target, body, &self.state.prefs);
    }

    #[command("r")]
    async fn cmd_reply(&self, ctx: Context, message: String) {
        let sender = ctx.sender.clone();
        let body = message.trim();
        if body.is_empty() {
            ctx.reply("§cMessage vide.");
            return;
        }
        let Some(partner_id) = self.state.chat.reply_partner(sender.uuid) else {
            ctx.reply("§cPersonne à qui répondre.");
            return;
        };
        let Ok(online) = sender.server().get_players().await else {
            return;
        };
        let Some(target) = online.into_iter().find(|p| p.uuid == partner_id) else {
            ctx.reply("§cCe joueur n'est plus connecté.");
            return;
        };
        self.state
            .chat
            .send_whisper(&sender, &target, body, &self.state.prefs);
    }

    #[command("build", permission = "ferriscraft.admin")]
    async fn cmd_build(&self, ctx: Context) {
        let player = &ctx.sender;
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.build_mode = !p.build_mode);
        if updated.build_mode {
            player.game_mode.set(GameMode::Creative);
            ctx.reply("§a🛠 Mode construction du Lobby ACTIVÉ.");
        } else {
            player.game_mode.set(GameMode::Adventure);
            updated.apply_movement(player);
            let claims = self.state.auth.claims_of(player.uuid);
            apply_lobby_hotbar(player, claims.as_ref(), &updated);
            ctx.reply("§e🛠 Mode construction du Lobby DÉSACTIVÉ.");
        }
    }

    #[command("lobby", permission = "ferriscraft.admin")]
    async fn cmd_lobby(
        &self,
        ctx: Context,
        #[suggest(Self::suggest_lobby_subcommands)] action: String,
        arg1: Option<String>,
        arg2: Option<usize>,
    ) {
        let player = ctx.sender.clone();
        let pose = WaypointPose::from_player(&player);

        match action.as_str() {
            "info" => {
                let snap = self.state.layout.snapshot();
                ctx.reply(format!(
                    "§6§lLobby §8» §7Spawn=({:.1}, {:.1}, {:.1}) | Plots={} | PNJ Village={} | Postes Capitainerie={}",
                    snap.spawn.x,
                    snap.spawn.y,
                    snap.spawn.z,
                    snap.pedestals.len(),
                    snap.ambient_npcs.len(),
                    snap.agent_workstations.len()
                ));
            }
            "reload" => match self.state.layout.reload() {
                Ok(_) => {
                    self.state
                        .npcs
                        .spawn_all(&player.server(), self.queued_count())
                        .await;
                    ctx.reply("§a✔ Configuration lobby.json rechargée et PNJ synchronisés.");
                }
                Err(err) => ctx.reply(format!("§cErreur de rechargement : {err}")),
            },
            "setspawn" => {
                let res = self.state.layout.update(|l| {
                    l.spawn = pose;
                    l.pois.insert("spawn".to_owned(), pose);
                    Ok(())
                });
                match res {
                    Ok(_) => ctx.reply("§a✔ Point de Spawn mis à jour sur votre position."),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setpoi" => {
                let Some(name) = arg1 else {
                    ctx.reply("§eUsage : /lobby setpoi <spawn|plaza|portal|capitainerie|castle>");
                    return;
                };
                let res = self.state.layout.update(|l| {
                    l.pois.insert(name.clone(), pose);
                    Ok(())
                });
                match res {
                    Ok(_) => ctx.reply(format!("§a✔ Point d'intérêt '{name}' enregistré.")),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setpedestal" => {
                let Some(idx) = arg1.as_deref().and_then(|s| s.parse::<usize>().ok()) else {
                    ctx.reply("§eUsage : /lobby setpedestal <1..7>");
                    return;
                };
                let res = self.state.layout.update(|l| {
                    let slot = idx
                        .checked_sub(1)
                        .and_then(|i| l.pedestals.get_mut(i))
                        .ok_or_else(|| {
                            ConfigError::OutOfBounds(format!("plot {idx} invalide (1..=7)"))
                        })?;
                    slot.pose = pose;
                    Ok(())
                });
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_count())
                            .await;
                        ctx.reply(format!("§a✔ Plot #{idx} repositionné et PNJ mis à jour."));
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setcaptain" => {
                let res = self.state.layout.update(|l| {
                    l.captain = pose;
                    Ok(())
                });
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_count())
                            .await;
                        ctx.reply("§a✔ Position du Capitaine enregistrée.");
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setstation" => {
                let Some(idx) = arg1.as_deref().and_then(|s| s.parse::<usize>().ok()) else {
                    ctx.reply("§eUsage : /lobby setstation <index_0_based>");
                    return;
                };
                let res = self.state.layout.update(|l| {
                    if idx < l.agent_workstations.len() {
                        l.agent_workstations[idx] = pose;
                    } else if idx == l.agent_workstations.len() {
                        l.agent_workstations.push(pose);
                    } else {
                        return Err(ConfigError::OutOfBounds(format!(
                            "poste {idx} hors limites"
                        )));
                    }
                    Ok(())
                });
                match res {
                    Ok(_) => ctx.reply(format!(
                        "§a✔ Poste de travail Capitainerie #{idx} enregistré."
                    )),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setwaypoint" => {
                let (Some(npc_id), Some(wp_idx)) = (arg1, arg2) else {
                    ctx.reply(
                        "§eUsage : /lobby setwaypoint <guard|lumberjack|mage|innkeeper> <index>",
                    );
                    return;
                };
                let res = self.state.layout.update(|l| {
                    let npc = l
                        .ambient_npcs
                        .iter_mut()
                        .find(|n| n.id == npc_id)
                        .ok_or_else(|| {
                            ConfigError::OutOfBounds(format!("PNJ '{npc_id}' introuvable"))
                        })?;
                    if wp_idx < npc.waypoints.len() {
                        npc.waypoints[wp_idx] = pose;
                    } else if wp_idx == npc.waypoints.len() {
                        npc.waypoints.push(pose);
                    } else {
                        return Err(ConfigError::OutOfBounds(format!(
                            "waypoint {wp_idx} hors limites"
                        )));
                    }
                    Ok(())
                });
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_count())
                            .await;
                        ctx.reply(format!(
                            "§a✔ Waypoint #{wp_idx} du PNJ '{npc_id}' enregistré."
                        ));
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            other => {
                ctx.reply(format!(
                    "§eSous-commande '{other}' inconnue. Choix : {}",
                    LOBBY_SUBCOMMANDS.join(", ")
                ));
            }
        }
    }
}

fn portal_launch_velocity(entry_step: Option<Vector>, yaw: f32) -> Vector {
    let (dx, dz, len) = match entry_step {
        Some(step) => {
            let len = step.x.hypot(step.z);
            (step.x, step.z, len)
        }
        None => (0.0, 0.0, 0.0),
    };
    let (dir_x, dir_z, speed) = if len > 1e-3 {
        (
            -dx / len,
            -dz / len,
            (len * 4.0).clamp(PORTAL_LAUNCH_MIN_SPEED, PORTAL_LAUNCH_MAX_SPEED),
        )
    } else {
        let rad = f64::from(yaw.to_radians());
        (rad.sin(), -rad.cos(), PORTAL_LAUNCH_MIN_SPEED)
    };
    Vector {
        x: dir_x * speed,
        y: PORTAL_LAUNCH_LIFT,
        z: dir_z * speed,
    }
}
