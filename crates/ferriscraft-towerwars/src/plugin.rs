use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ferriscraft_protocol::{HeartbeatRequest, ServerPhase};
use rspigot_plugin::prelude::*;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::TowerWarsConfig;
use crate::config::load_map;
use crate::game::{
    Arena, EndBehavior, Game, Layout, Rules, SHUTDOWN_REPORT_GRACE, Session, tool_of,
};
use crate::hud::{Hud, waiting_title};
use crate::orchestrator::{SERVER_KIND, TowerWarsOrchestratorClient};

const ADMIN_PERMISSION: &str = "towerwars.admin";
const TICK: Duration = Duration::from_millis(50);
const ACTIONS: [&str; 6] = ["join", "leave", "hub", "version", "forcestart", "stop"];

fn short_ref(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| "?".to_owned(), |value| value.chars().take(7).collect())
}

const fn blocks_creature_spawn(reason: CreatureSpawnReason) -> bool {
    !matches!(
        reason,
        CreatureSpawnReason::Custom | CreatureSpawnReason::Default | CreatureSpawnReason::Command
    )
}

#[derive(Clone)]
pub struct ShutdownHandle {
    game: Arc<OnceLock<Arc<Game>>>,
}

impl ShutdownHandle {
    pub fn game(&self) -> Option<Arc<Game>> {
        self.game.get().cloned()
    }

    pub async fn drain(&self) {
        if let Some(game) = self.game.get() {
            game.shutdown(SHUTDOWN_REPORT_GRACE).await;
        }
    }
}

#[derive(Plugin)]
pub struct TowerWarsPlugin {
    config: TowerWarsConfig,
    orchestrator: Option<Arc<TowerWarsOrchestratorClient>>,
    game: Arc<OnceLock<Arc<Game>>>,
    looper: Mutex<Option<JoinHandle<()>>>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
}

impl Default for TowerWarsPlugin {
    fn default() -> Self {
        Self::new(TowerWarsConfig::default())
    }
}

impl TowerWarsPlugin {
    pub fn new(config: TowerWarsConfig) -> Self {
        let orchestrator = config.orchestrator_url.as_ref().map(|url| {
            Arc::new(TowerWarsOrchestratorClient::new(
                url.clone(),
                config.orchestrator_token.clone(),
            ))
        });
        Self {
            config,
            orchestrator,
            game: Arc::new(OnceLock::new()),
            looper: Mutex::new(None),
            heartbeat_task: Mutex::new(None),
        }
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            game: Arc::clone(&self.game),
        }
    }

    pub fn game(&self) -> Option<Arc<Game>> {
        self.game.get().cloned()
    }

    async fn actions(&self, _player: &Player, typed: &str) -> Vec<String> {
        ACTIONS
            .iter()
            .filter(|action| action.starts_with(typed))
            .map(|action| (*action).to_owned())
            .collect()
    }

    fn spawn_loop(game: Arc<Game>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                game.tick().await;
            }
        })
    }

    fn end_behavior(&self) -> EndBehavior {
        if self.orchestrator.is_some() {
            if self.config.ephemeral_pod {
                EndBehavior::EphemeralPod
            } else {
                EndBehavior::ResetRound {
                    return_to_lobby: true,
                }
            }
        } else {
            EndBehavior::ResetRound {
                return_to_lobby: false,
            }
        }
    }
}

#[listeners]
impl TowerWarsPlugin {
    async fn on_enable(&self, server: &Server) {
        if let Some(orch) = &self.orchestrator {
            let starting = HeartbeatRequest {
                server_id: self.config.server_id.clone(),
                address: None,
                phase: ServerPhase::Starting,
                player_count: 0,
                max_players: u32::try_from(self.config.max_players).unwrap_or(u32::MAX),
                player_uuids: Vec::new(),
                version: self.config.version.clone(),
                kind: SERVER_KIND,
            };
            let _ = orch.send_heartbeat(&starting).await;
        }

        let map = load_map(self.config.map_path.as_deref());
        let layout = Layout::from_map_with(&map, self.config.decor);
        let arena = match Arena::create(server, layout).await {
            Ok(arena) => arena,
            Err(error) => {
                tracing::error!("impossible de créer l'arène TowerWars : {error}");
                return;
            }
        };
        let reused = arena.reused;
        let session = Session::new(map, Rules::from(&self.config), self.end_behavior());
        let target = session.threshold();
        let bar = match server
            .boss_bars
            .create(
                waiting_title(0, target),
                BarColor::Yellow,
                BarStyle::Solid,
                &[],
            )
            .await
        {
            Ok(bar) => bar,
            Err(error) => {
                tracing::error!("impossible de créer la boss bar : {error}");
                return;
            }
        };
        let hud = Hud::new(server.scoreboard_manager.main_scoreboard());
        let game = Arc::new(Game::new(
            arena,
            server.clone(),
            session,
            hud,
            bar,
            self.config.clone(),
            self.orchestrator.clone(),
        ));
        if self.game.set(Arc::clone(&game)).is_err() {
            return;
        }
        if reused {
            game.restore_arena(self.config.world_prebuilt).await;
        }
        match server.get_players().await {
            Ok(players) => {
                for player in players {
                    game.admit(player).await;
                }
            }
            Err(error) => tracing::warn!("impossible de lister les joueurs : {error}"),
        }

        if let Some(hb) = game.spawn_heartbeat_loop()
            && let Ok(mut guard) = self.heartbeat_task.lock()
        {
            *guard = Some(hb);
        }
        if let Ok(mut looper) = self.looper.lock() {
            *looper = Some(Self::spawn_loop(game));
        }

        tracing::info!(
            server_id = %self.config.server_id,
            orchestrator = ?self.config.orchestrator_url,
            ephemeral = self.config.ephemeral_pod,
            min_players = self.config.min_players,
            max_players = self.config.max_players,
            "ferriscraft-towerwars initialisé"
        );
    }

    async fn on_disable(&self) {
        if let Ok(mut looper) = self.looper.lock()
            && let Some(handle) = looper.take()
        {
            handle.abort();
        }
        if let Ok(mut hb) = self.heartbeat_task.lock()
            && let Some(handle) = hb.take()
        {
            handle.abort();
        }
        if let Some(game) = self.game() {
            game.close();
        }
    }

    #[command("towerwars")]
    async fn towerwars(&self, ctx: Context, #[suggest(Self::actions)] action: String) {
        let Some(game) = self.game() else {
            ctx.reply("§cLe jeu n'est pas prêt.");
            return;
        };
        let player = &ctx.sender;
        let admin = player.permissions.has(ADMIN_PERMISSION);
        match action.as_str() {
            "join" => game.join(player).await,
            "leave" => game.quit(player).await,
            "hub" => game.send_player_hub(player).await,
            "version" => ctx.reply(format!(
                "§dTowerWars §8» §7serveur §f{} §8• §7plugin §f{} §8• §7rspigot §f{}",
                self.config.server_id,
                self.config.version.as_deref().unwrap_or("dev"),
                short_ref(std::env::var("RSPIGOT_REF").ok().as_deref())
            )),
            "forcestart" | "stop" if !admin => ctx.reply("§cVous ne pouvez pas faire ça."),
            "forcestart" => {
                if let Err(error) = game.force_start().await {
                    ctx.reply(format!("§c{error}"));
                }
            }
            "stop" => game.stop().await,
            other => ctx.reply(format!("§cAction inconnue : {other}")),
        }
    }

    async fn on_chunk_load(&self, event: &ChunkLoad) {
        if let Some(game) = self.game() {
            game.on_chunk_load(event);
        }
    }

    async fn on_player_join(&self, event: &PlayerJoin) {
        if let Some(game) = self.game() {
            game.admit(event.player().clone()).await;
        }
    }

    async fn on_player_leave(&self, event: &PlayerLeave) {
        if let Some(game) = self.game() {
            game.release(event.player()).await;
        }
    }

    async fn on_player_interact(&self, event: &mut PlayerInteract) {
        let Some(game) = self.game() else { return };
        let right_click = matches!(
            event.action(),
            Action::RightClickAir | Action::RightClickBlock
        );
        let left_click = matches!(
            event.action(),
            Action::LeftClickAir | Action::LeftClickBlock
        );
        if !(right_click || left_click) || event.hand() != Some(EquipmentSlot::Hand) {
            return;
        }
        let Some(tool) = tool_of(event.material()) else {
            return;
        };
        tracing::debug!(?tool, right_click, "interaction avec un outil");
        event.cancel();
        let player = event.player().clone();
        tokio::spawn(async move { game.use_tool(&player, tool, right_click).await });
    }

    async fn on_player_interact_at_entity(&self, event: &mut PlayerInteractAtEntity) {
        let Some(game) = self.game() else { return };
        if event.hand() != EquipmentSlot::Hand {
            return;
        }
        let entity_id = event.right_clicked().entity_id;
        let player = event.player().clone();
        tokio::spawn(async move {
            game.tower_clicked(&player, entity_id).await;
        });
    }

    async fn on_inventory_click(&self, event: &mut InventoryClick) {
        event.cancel();
    }

    // rspigot refuse les clics dans un menu verrouillé et ne les transmet qu'aux handlers
    // en observation (`&InventoryClick`) : un handler `&mut` ne les voit jamais.
    async fn on_menu_click(&self, event: &InventoryClick) {
        let Some(game) = self.game() else { return };
        let Some(inventory) = event.clicked_inventory() else {
            return;
        };
        let InventoryOwner::Custom(custom) = &inventory.owner else {
            return;
        };
        if !custom.is_mine() || !game.owns_menu(custom.id) {
            return;
        }
        let (id, slot) = (custom.id, event.slot());
        let (right, shift) = (event.is_right_click(), event.is_shift_click());
        let player = event.player().clone();
        tokio::spawn(async move { game.menu_click(&player, id, slot, right, shift).await });
    }

    async fn on_inventory_close(&self, event: &InventoryClose) {
        let Some(game) = self.game() else { return };
        if let InventoryOwner::Custom(custom) = &event.inventory().owner
            && custom.is_mine()
        {
            game.menu_closed(custom.id);
        }
    }

    async fn on_player_move(&self, event: &PlayerMove) {
        let Some(game) = self.game() else { return };
        let Some(to) = event.to() else { return };
        if game.is_lost(to.position.x, to.position.y, to.position.z) {
            let player = event.player().clone();
            tokio::spawn(async move { game.rescue(&player).await });
        }
    }

    async fn on_entity_damage(&self, event: &mut EntityDamage) {
        if event.entity_type() == EntityType::Player {
            event.cancel();
        }
    }

    async fn on_entity_damage_by_entity(&self, event: &mut EntityDamageByEntity) {
        let Some(game) = self.game() else { return };
        if event.entity_type() == EntityType::Interaction {
            event.cancel();
            if let Some(player) = event.player().cloned() {
                let entity_id = event.entity().entity_id;
                tokio::spawn(async move {
                    game.tower_clicked(&player, entity_id).await;
                });
            }
        }
    }

    async fn on_food_level_change(&self, event: &mut FoodLevelChange) {
        event.cancel();
    }

    async fn on_block_break(&self, event: &mut BlockBreak) {
        event.cancel();
    }

    async fn on_block_place(&self, event: &mut BlockPlace) {
        event.cancel();
    }

    async fn on_creature_spawn(&self, event: &mut CreatureSpawn) {
        if blocks_creature_spawn(event.spawn_reason()) {
            event.cancel();
        }
    }

    async fn on_item_spawn(&self, event: &mut ItemSpawn) {
        event.cancel();
    }

    async fn on_player_drop_item(&self, event: &mut PlayerDropItem) {
        event.cancel();
    }

    async fn on_player_swap_hand_items(&self, event: &mut PlayerSwapHandItems) {
        event.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plugin_and_command_spawns_are_allowed() {
        for reason in [
            CreatureSpawnReason::Custom,
            CreatureSpawnReason::Default,
            CreatureSpawnReason::Command,
        ] {
            assert!(!blocks_creature_spawn(reason));
        }
        for reason in [
            CreatureSpawnReason::Natural,
            CreatureSpawnReason::ChunkGen,
            CreatureSpawnReason::Spawner,
            CreatureSpawnReason::Jockey,
            CreatureSpawnReason::Raid,
        ] {
            assert!(blocks_creature_spawn(reason));
        }
    }

    #[test]
    fn the_rspigot_ref_is_cut_to_seven_characters() {
        assert_eq!(short_ref(Some("0123456789abcdef")), "0123456");
        assert_eq!(short_ref(Some("abc")), "abc");
        assert_eq!(short_ref(Some("  ")), "?");
        assert_eq!(short_ref(None), "?");
    }

    #[test]
    fn the_command_actions_match_the_contract() {
        assert_eq!(
            ACTIONS,
            ["join", "leave", "hub", "version", "forcestart", "stop"]
        );
    }
}
