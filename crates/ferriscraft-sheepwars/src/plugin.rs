use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ferriscraft_protocol::{HeartbeatRequest, ServerPhase};
use rspigot_plugin::prelude::*;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::SheepWarsConfig;
use crate::arena::{Arena, Layout, Platform};
use crate::config::load_settings;
use crate::game::{EndBehavior, Game, HUB_ITEM, Session, VoidVerdict, kind_of};
use crate::hud::{Hud, bossbar_title};
use crate::orchestrator::SheepWarsOrchestratorClient;

const ADMIN_PERMISSION: &str = "sheepwars.admin";
const TICK: Duration = Duration::from_millis(50);
const ACTIONS: [&str; 6] = ["join", "leave", "hub", "version", "forcestart", "stop"];
const EXPLOSION_YIELD: f32 = 0.0;

struct ArenaGuard {
    world_id: String,
    layout: Layout,
}

const fn explosion_yield() -> f32 {
    EXPLOSION_YIELD
}

const fn blocks_creature_spawn(reason: CreatureSpawnReason) -> bool {
    !matches!(
        reason,
        CreatureSpawnReason::Custom | CreatureSpawnReason::Default | CreatureSpawnReason::Command
    )
}

fn cancels_vanilla_explosion(entity: EntityType, cause: EntityDamageCause) -> bool {
    entity == EntityType::Player
        && matches!(
            cause,
            EntityDamageCause::BlockExplosion | EntityDamageCause::EntityExplosion
        )
}

const fn is_fire(kind: BlockType) -> bool {
    matches!(kind, BlockType::Fire | BlockType::SoulFire)
}

const fn is_unbreakable(kind: BlockType) -> bool {
    matches!(kind, BlockType::Bedrock | BlockType::Barrier)
}

fn platform_contains(platform: &Platform, pos: BlockPos) -> bool {
    let min_x = platform.center.x - platform.width / 2;
    let min_z = platform.center.z - platform.depth / 2;
    let min_y = platform.center.y - platform.thickness + 1;
    (min_x..min_x + platform.width).contains(&pos.x)
        && (min_z..min_z + platform.depth).contains(&pos.z)
        && (min_y..=platform.center.y).contains(&pos.y)
}

fn on_platform(layout: &Layout, pos: BlockPos) -> bool {
    let settings = layout.settings();
    platform_contains(&settings.red.platform, pos)
        || platform_contains(&settings.blue.platform, pos)
        || platform_contains(&settings.lobby_platform, pos)
}

fn may_break(kind: BlockType, pos: BlockPos, layout: &Layout) -> bool {
    !is_unbreakable(kind) && !on_platform(layout, pos)
}

#[derive(Plugin)]
pub struct SheepWarsPlugin {
    config: SheepWarsConfig,
    orchestrator: Option<Arc<SheepWarsOrchestratorClient>>,
    game: OnceLock<Arc<Game>>,
    guard: OnceLock<ArenaGuard>,
    looper: Mutex<Option<JoinHandle<()>>>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
}

impl Default for SheepWarsPlugin {
    fn default() -> Self {
        Self::new(SheepWarsConfig::default())
    }
}

impl SheepWarsPlugin {
    pub fn new(config: SheepWarsConfig) -> Self {
        let orchestrator = config.orchestrator_url.as_ref().map(|url| {
            Arc::new(SheepWarsOrchestratorClient::new(
                url.clone(),
                config.orchestrator_token.clone(),
            ))
        });
        Self {
            config,
            orchestrator,
            game: OnceLock::new(),
            guard: OnceLock::new(),
            looper: Mutex::new(None),
            heartbeat_task: Mutex::new(None),
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
}

#[listeners]
impl SheepWarsPlugin {
    async fn on_enable(&self, server: &Server) {
        if let Some(orch) = &self.orchestrator {
            let starting = HeartbeatRequest {
                server_id: self.config.server_id.clone(),
                address: None,
                phase: ServerPhase::Starting,
                player_count: 0,
                max_players: u32::try_from(self.config.rules.max_players).unwrap_or(u32::MAX),
                player_uuids: Vec::new(),
                version: self.config.version.clone(),
            };
            let _ = orch.send_heartbeat(&starting).await;
        }

        let arena_settings = load_settings(&self.config.arena_path);
        let arena = match Arena::create(server, arena_settings).await {
            Ok(arena) => arena,
            Err(error) => {
                tracing::error!("impossible de créer l'arène SheepWars : {error}");
                return;
            }
        };

        let end_behavior = if self.orchestrator.is_some() {
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
        };

        let session = Session::new_with(
            arena.layout.clone(),
            self.config.rules,
            self.config.end_delay_ticks,
            end_behavior,
        );
        let bar = match server
            .boss_bars
            .create(
                bossbar_title(session.round()),
                BarColor::Purple,
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
        let reused = arena.reused;
        let _ = self.guard.set(ArenaGuard {
            world_id: arena.world.id().to_owned(),
            layout: arena.layout.clone(),
        });
        let hud = Hud::new(server.scoreboard_manager.main_scoreboard());
        let game = Arc::new(Game::new(
            arena,
            session,
            hud,
            bar,
            self.config.clone(),
            self.orchestrator.clone(),
        ));
        if self.game.set(Arc::clone(&game)).is_err() {
            return;
        }
        if reused && let Err(error) = game.restore_platforms() {
            tracing::warn!("impossible de restaurer les plateformes de l'arène : {error}");
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
            min_players = self.config.rules.min_players,
            max_players = self.config.rules.max_players,
            "ferriscraft-sheepwars initialisé"
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

    #[command("sheepwars")]
    async fn sheepwars(&self, ctx: Context, #[suggest(Self::actions)] action: String) {
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
                "§dSheepWars §8» §7serveur §f{} §8• §7plugin §f{}",
                self.config.server_id,
                self.config.version.as_deref().unwrap_or("dev")
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
        if !right_click || event.hand() != Some(EquipmentSlot::Hand) {
            return;
        }
        if event.material() == HUB_ITEM {
            event.cancel();
            let player = event.player().clone();
            tokio::spawn(async move { game.send_player_hub(&player).await });
            return;
        }
        let Some(kind) = kind_of(event.material()) else {
            return;
        };
        event.cancel();
        let player = event.player().clone();
        tokio::spawn(async move {
            game.throw(&player, kind).await;
        });
    }

    async fn on_player_death(&self, event: &mut PlayerDeath) {
        let Some(game) = self.game() else { return };
        event.set_keep_inventory(true);
        let player = event.player().clone();
        tokio::spawn(async move { game.died(&player).await });
    }

    async fn on_player_respawn(&self, event: &mut PlayerRespawn) {
        if let Some(game) = self.game() {
            let location = game.respawn_location(event.player());
            event.set_respawn_location(location);
        }
    }

    async fn on_player_move(&self, event: &PlayerMove) {
        let Some(game) = self.game() else { return };
        let Some(to) = event.to() else { return };
        match game.void_verdict(event.player(), to.position.y) {
            VoidVerdict::Safe => {}
            VoidVerdict::Kill => event.player().health.kill(),
            VoidVerdict::ToLobby => {
                let player = event.player().clone();
                tokio::spawn(async move { game.rescue(&player).await });
            }
        }
    }

    async fn on_entity_damage(&self, event: &mut EntityDamage) {
        let Some(game) = self.game() else { return };
        if cancels_vanilla_explosion(event.entity_type(), event.cause()) {
            event.cancel();
            return;
        }
        if event.entity_type() == EntityType::Player
            && (game.blocks_damage()
                || (event.cause() == EntityDamageCause::Fall
                    && game.is_fall_protected(event.entity().uuid)))
        {
            event.cancel();
            return;
        }
        if event.entity_type() == EntityType::Sheep
            && game.is_managed_sheep(event.entity().entity_id)
            && (game.blocks_damage()
                || matches!(
                    event.cause(),
                    EntityDamageCause::Fall
                        | EntityDamageCause::Suffocation
                        | EntityDamageCause::Cramming
                        | EntityDamageCause::Drowning
                        | EntityDamageCause::Fire
                        | EntityDamageCause::FireTick
                        | EntityDamageCause::Lava
                        | EntityDamageCause::BlockExplosion
                        | EntityDamageCause::EntityExplosion
                        | EntityDamageCause::Lightning
                ))
        {
            event.cancel();
        }
    }

    async fn on_entity_damage_by_entity(&self, event: &mut EntityDamageByEntity) {
        let Some(game) = self.game() else { return };
        let attacker = event.player().or_else(|| {
            event
                .damage_source()
                .causing
                .as_ref()
                .and_then(|e| e.player.as_ref())
        });
        let Some(attacker) = attacker else { return };
        if let Some(victim) = event.entity().player.as_ref() {
            if game.blocks_hit(attacker, victim) {
                event.cancel();
            }
            return;
        }
        if event.entity_type() == EntityType::Sheep {
            game.record_sheep_damager(event.entity().entity_id, attacker);
        }
    }

    async fn on_entity_death(&self, event: &mut EntityDeath) {
        let Some(game) = self.game() else { return };
        if event.entity_type() != EntityType::Sheep {
            return;
        }
        let killer_uuid = event.damage_source().causing.as_ref().map(|e| e.uuid);
        if game.on_sheep_killed(event.entity().entity_id, killer_uuid) {
            event.set_drops(Vec::new());
            event.set_dropped_exp(0);
        }
    }

    async fn on_food_level_change(&self, event: &mut FoodLevelChange) {
        event.cancel();
    }

    async fn on_block_break(&self, event: &mut BlockBreak) {
        let (Some(game), Some(guard)) = (self.game(), self.guard.get()) else {
            event.cancel();
            return;
        };
        let player = event.player();
        if game.blocks_damage() || player.world.id() != guard.world_id {
            event.cancel();
            return;
        }
        let pos = event.block().pos;
        let allowed = match player.world.get_block(pos).await {
            Ok(state) => may_break(state.kind, pos, &guard.layout),
            Err(_) => false,
        };
        if allowed {
            event.set_drop_items(false);
            event.set_exp_to_drop(0);
        } else {
            event.cancel();
        }
    }

    async fn on_block_damage(&self, event: &mut BlockDamage) {
        let (Some(game), Some(guard)) = (self.game(), self.guard.get()) else {
            return;
        };
        let player = event.player();
        if game.blocks_damage() || player.world.id() != guard.world_id {
            return;
        }
        let pos = event.block().pos;
        if let Ok(state) = player.world.get_block(pos).await
            && is_fire(state.kind)
        {
            player.world.block(pos).set(BlockType::Air);
        }
    }

    async fn on_entity_shoot_bow(&self, event: &mut EntityShootBow) {
        let Some(game) = self.game() else { return };
        if let Some(player) = event.player() {
            game.track_arrow(player.uuid, event.projectile().clone());
        }
    }

    async fn on_creature_spawn(&self, event: &mut CreatureSpawn) {
        if blocks_creature_spawn(event.spawn_reason()) {
            event.cancel();
        }
    }

    async fn on_entity_explode(&self, event: &mut EntityExplode) {
        if self
            .guard
            .get()
            .is_some_and(|guard| guard.world_id == event.location().world)
        {
            event.set_yield(explosion_yield());
        }
    }

    async fn on_block_explode(&self, event: &mut BlockExplode) {
        event.set_yield(explosion_yield());
    }

    async fn on_item_spawn(&self, event: &mut ItemSpawn) {
        if self
            .guard
            .get()
            .is_some_and(|guard| guard.world_id == event.location().world)
        {
            event.cancel();
        }
    }

    async fn on_player_drop_item(&self, event: &mut PlayerDropItem) {
        event.cancel();
    }

    async fn on_block_place(&self, event: &mut BlockPlace) {
        event.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::ArenaSettings;

    fn layout() -> Layout {
        Layout::new(ArenaSettings::default())
    }

    fn pos(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos { x, y, z }
    }

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
            CreatureSpawnReason::TrialSpawner,
            CreatureSpawnReason::Jockey,
            CreatureSpawnReason::Reinforcements,
            CreatureSpawnReason::Patrol,
            CreatureSpawnReason::Raid,
        ] {
            assert!(blocks_creature_spawn(reason));
        }
    }

    #[test]
    fn vanilla_explosions_never_hurt_players() {
        for cause in [
            EntityDamageCause::BlockExplosion,
            EntityDamageCause::EntityExplosion,
        ] {
            assert!(cancels_vanilla_explosion(EntityType::Player, cause));
            assert!(!cancels_vanilla_explosion(EntityType::Sheep, cause));
        }
        assert!(!cancels_vanilla_explosion(
            EntityType::Player,
            EntityDamageCause::Fall
        ));
        assert!(!cancels_vanilla_explosion(
            EntityType::Player,
            EntityDamageCause::EntityAttack
        ));
    }

    #[test]
    fn ordinary_blocks_break_but_bedrock_and_barriers_do_not() {
        let layout = layout();
        let far = pos(200, 70, 200);
        assert!(may_break(BlockType::Stone, far, &layout));
        assert!(may_break(BlockType::Dirt, far, &layout));
        assert!(!may_break(BlockType::Bedrock, far, &layout));
        assert!(!may_break(BlockType::Barrier, far, &layout));
    }

    #[test]
    fn spawn_and_lobby_platforms_are_unbreakable() {
        let layout = layout();
        let settings = layout.settings();
        for platform in [
            &settings.red.platform,
            &settings.blue.platform,
            &settings.lobby_platform,
        ] {
            let center = pos(platform.center.x, platform.center.y, platform.center.z);
            assert!(!may_break(BlockType::QuartzBlock, center, &layout));
            let above = pos(center.x, center.y + 1, center.z);
            assert!(may_break(BlockType::QuartzBlock, above, &layout));
        }
    }

    #[test]
    fn fire_breaks_like_any_ordinary_block() {
        let layout = layout();
        assert!(is_fire(BlockType::Fire));
        assert!(is_fire(BlockType::SoulFire));
        assert!(!is_fire(BlockType::Stone));
        assert!(may_break(BlockType::Fire, pos(200, 70, 200), &layout));
    }

    #[test]
    fn explosions_yield_no_items() {
        assert_eq!(explosion_yield(), 0.0);
    }
}
