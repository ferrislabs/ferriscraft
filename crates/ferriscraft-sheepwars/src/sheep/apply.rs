use std::time::Duration;

use futures_util::future::join_all;
use rspigot_plugin::{
    Attribute, BlockPos, BlockState, BlockType, Color, DyeColor, Effect, EffectType, Entity,
    EntityType, Error, Explosion, Material, Particle, ParticleBurst, Player, PlayerState, Position,
    Sound, SoundCue, Vector, World,
};

use super::catalog::{self, WoolColor};
use super::impact::{
    Action, BlockKind, Cell, EffectKind, ParticleKind, SoundKind, Vec3, outward_velocity,
};
use crate::domain::{SheepKind, TeamId};

pub async fn apply(
    world: &World,
    thrower: Option<&Player>,
    players: &[(Player, TeamId)],
    actions: &[Action],
) -> Result<Vec<(SheepKind, Entity, Vec3)>, Error> {
    let needs_player_states = actions.iter().any(|action| {
        matches!(
            action,
            Action::Effect { .. }
                | Action::Heal { .. }
                | Action::Ignite { .. }
                | Action::Velocity { .. }
                | Action::Push { .. }
                | Action::Damage { .. }
                | Action::SwapPositions { .. }
        )
    });
    let live_players = if needs_player_states {
        fetch_live_players(players).await
    } else {
        Vec::new()
    };

    let mut spawned = Vec::new();
    for action in actions {
        match *action {
            Action::Explode {
                at,
                power,
                breaks_blocks,
            } => world
                .explosion
                .create(Explosion::new(position(at), power, breaks_blocks)),
            Action::SetBlock { at, block } => {
                set_block_if_valid(world, at, block).await;
            }
            Action::Particle { kind, at, count } => world
                .particle
                .spawn(ParticleBurst::new(particle(kind), position(at)).count(count)),
            Action::Sound { kind, at } => world
                .sound
                .play(SoundCue::new(sound(kind)).at(position(at))),
            Action::Effect {
                team,
                center,
                radius,
                effect,
                duration_ticks,
                amplifier,
            } => {
                let targets: Vec<&Player> = matching_players(&live_players, center, radius, team)
                    .map(|(player, _)| player)
                    .collect();
                tracing::info!(
                    ?effect,
                    ?team,
                    radius,
                    candidates = live_players.len(),
                    targets = targets.len(),
                    "effet de mouton appliqué"
                );
                for player in targets {
                    player
                        .effects
                        .add(Effect::new(effect_type(effect), duration_ticks, amplifier));
                }
            }
            Action::Heal {
                team,
                center,
                radius,
                amount,
            } => {
                for (player, state) in matching_players(&live_players, center, radius, team) {
                    if state.health > 0.0 && state.health < state.max_health {
                        let healed = (state.health + amount).min(state.max_health);
                        player.health.set(healed);
                    }
                }
            }
            Action::Ignite {
                team,
                center,
                radius,
                fire_ticks,
            } => {
                for (player, _) in matching_players(&live_players, center, radius, team) {
                    player.health.set_fire_ticks(fire_ticks);
                }
            }
            Action::Velocity {
                team,
                center,
                radius,
                velocity,
            } => {
                for (player, _) in matching_players(&live_players, center, radius, team) {
                    player.movement.set_velocity(vector(velocity));
                }
            }
            Action::Push {
                team,
                center,
                radius,
                strength,
                lift,
            } => {
                for (player, state) in matching_players(&live_players, center, radius, team) {
                    let velocity =
                        outward_velocity(center, vec3(state.player.position), strength, lift);
                    player.movement.set_velocity(vector(velocity));
                }
            }
            Action::Damage {
                team,
                center,
                radius,
                damage,
            } => {
                for (player, state) in matching_players(&live_players, center, radius, team) {
                    let distance = distance_squared(center, vec3(state.player.position)).sqrt();
                    let points = damage.at_distance(distance, radius);
                    if points.is_infinite() {
                        player.health.kill();
                    } else if points > 0.0 {
                        player.health.damage(f64::from(points));
                    }
                }
            }
            Action::ClearSphere { center, radius } => {
                clear_cells(world, sphere_cells(center, radius)).await;
            }
            Action::ClearAbove {
                center,
                radius,
                height,
            } => {
                clear_cells(world, column_cells(center, radius, height)).await;
            }
            Action::SwapBlocks { a, b } => {
                swap_blocks(world, a, b).await;
            }
            Action::SwapPositions {
                team,
                center,
                radius,
            } => {
                let targets: Vec<(&Player, Position)> =
                    matching_players(&live_players, center, radius, team)
                        .map(|(player, state)| (player, state.player.position))
                        .collect();
                if targets.len() >= 2 {
                    for (index, (player, _)) in targets.iter().enumerate() {
                        let destination = targets[(index + 1) % targets.len()].1;
                        if let Err(error) = player.location.teleport(destination).await {
                            tracing::warn!(
                                "échec téléportation distorsion pour {} : {error}",
                                player.name
                            );
                        }
                    }
                }
            }
            Action::Lightning { at } => {
                world.spawn(EntityType::LightningBolt, position(at)).await?;
            }
            Action::SpawnSheep { kind, at, velocity } => {
                let entity = world.spawn(EntityType::Sheep, position(at)).await?;
                if let Some(sheep) = entity.as_sheep() {
                    sheep.set_color(sheep_dye_color(kind));
                }
                entity.flags.set_gravity(false);
                entity.flags.set_invulnerable(false);
                entity.name.set(catalog::display_name(kind));
                entity.name.set_visible(true);
                if kind == SheepKind::Fragment {
                    entity
                        .attributes
                        .set(Attribute::Scale, catalog::FRAGMENT_SCALE);
                }
                entity.set_velocity(vector(velocity));
                spawned.push((kind, entity, velocity));
            }
            Action::TeleportThrower { to } => {
                if let Some(player) = thrower {
                    let target = Position {
                        x: to.x,
                        y: to.y + 0.25,
                        z: to.z,
                    };
                    if let Err(error) = player.location.teleport(target).await {
                        tracing::warn!(
                            "échec téléportation abordage pour {} : {error}",
                            player.name
                        );
                    }
                    player.movement.set_velocity(vector(Vec3::ZERO));
                }
            }
        }
    }
    Ok(spawned)
}

pub async fn nearest_enemy(
    from: Vec3,
    range: f64,
    thrower_team: TeamId,
    players: &[(Player, TeamId)],
) -> Option<(Vec3, f64)> {
    let enemy_team = thrower_team.other();
    let queries = players
        .iter()
        .filter(|(_, team)| *team == enemy_team)
        .map(|(player, _)| player.get_position());
    let positions = join_all(queries).await;
    let range_sq = range * range;
    positions
        .into_iter()
        .filter_map(Result::ok)
        .map(|pos| Vec3 {
            x: pos.x,
            y: pos.y + 0.9,
            z: pos.z,
        })
        .map(|target| (target, distance_squared(from, target)))
        .filter(|(_, dist_sq)| *dist_sq <= range_sq)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(target, dist_sq)| (target, dist_sq.sqrt()))
}

async fn fetch_live_players(players: &[(Player, TeamId)]) -> Vec<(Player, PlayerState, TeamId)> {
    let states = join_all(players.iter().map(|(player, _)| player.get_state())).await;
    players
        .iter()
        .zip(states)
        .filter_map(|((player, team), state)| match state {
            Ok(state) if !state.dead => Some((player.clone(), state, *team)),
            _ => None,
        })
        .collect()
}

fn matching_players(
    live: &[(Player, PlayerState, TeamId)],
    center: Vec3,
    radius: f64,
    team: TeamId,
) -> impl Iterator<Item = (&Player, &PlayerState)> {
    let radius_sq = radius * radius;
    live.iter()
        .filter(move |(_, state, member_team)| {
            *member_team == team
                && distance_squared(center, vec3(state.player.position)) <= radius_sq
        })
        .map(|(player, state, _)| (player, state))
}

async fn set_block_if_valid(world: &World, at: Cell, block: BlockKind) {
    let pos = block_pos(at);
    match block {
        BlockKind::Ice => {
            if let Ok(existing) = world.get_block(pos).await
                && !is_air(existing.kind)
            {
                world.block(pos).set(BlockType::Ice);
            }
        }
        BlockKind::Fire => {
            let below = BlockPos {
                x: pos.x,
                y: pos.y - 1,
                z: pos.z,
            };
            if let (Ok(here), Ok(under)) =
                (world.get_block(pos).await, world.get_block(below).await)
                && is_air(here.kind)
                && !is_air(under.kind)
            {
                world.block(pos).set(BlockType::Fire);
            }
        }
    }
}

const CLEAR_BATCH: usize = 512;
const SWAP_VISUAL_DELAY: Duration = Duration::from_millis(500);
const SWAP_VISUAL_SPEED: f64 = 0.08;

fn sphere_cells(center: Vec3, radius: f64) -> Vec<Cell> {
    let reach = radius.ceil() as i32;
    let origin = center.cell();
    let mut cells = Vec::new();
    for dx in -reach..=reach {
        for dy in -reach..=reach {
            for dz in -reach..=reach {
                let cell = Cell {
                    x: origin.x + dx,
                    y: origin.y + dy,
                    z: origin.z + dz,
                };
                let offset = Vec3 {
                    x: f64::from(cell.x) + 0.5,
                    y: f64::from(cell.y) + 0.5,
                    z: f64::from(cell.z) + 0.5,
                };
                if distance_squared(center, offset) <= radius * radius {
                    cells.push(cell);
                }
            }
        }
    }
    cells
}

fn column_cells(center: Vec3, radius: f64, height: f64) -> Vec<Cell> {
    let reach = radius.ceil() as i32;
    let origin = center.cell();
    let mut cells = Vec::new();
    for dx in -reach..=reach {
        for dz in -reach..=reach {
            let x = f64::from(origin.x + dx) + 0.5 - center.x;
            let z = f64::from(origin.z + dz) + 0.5 - center.z;
            if x * x + z * z <= radius * radius {
                for dy in 0..=height.floor() as i32 {
                    cells.push(Cell {
                        x: origin.x + dx,
                        y: origin.y + dy,
                        z: origin.z + dz,
                    });
                }
            }
        }
    }
    cells
}

const fn is_protected(kind: BlockType) -> bool {
    matches!(kind, BlockType::Bedrock | BlockType::Barrier)
}

async fn clear_cells(world: &World, cells: Vec<Cell>) {
    for batch in cells.chunks(CLEAR_BATCH) {
        let states = join_all(batch.iter().map(|cell| world.get_block(block_pos(*cell)))).await;
        for (cell, state) in batch.iter().zip(states) {
            match state {
                Ok(state) if !is_air(state.kind) && !is_protected(state.kind) => {
                    world.block(block_pos(*cell)).set(BlockType::Air);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("échec lecture bloc {cell:?} : {error}"),
            }
        }
    }
}

fn block_center(cell: Cell) -> Position {
    Position {
        x: f64::from(cell.x) + 0.5,
        y: f64::from(cell.y),
        z: f64::from(cell.z) + 0.5,
    }
}

async fn swap_blocks(world: &World, a: Cell, b: Cell) {
    if a == b {
        return;
    }
    let (first, second) = (
        world.get_block(block_pos(a)).await,
        world.get_block(block_pos(b)).await,
    );
    let (first, second) = match (first, second) {
        (Ok(first), Ok(second)) => (first, second),
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!("échec lecture blocs distorsion : {error}");
            return;
        }
    };
    if is_protected(first.kind)
        || is_protected(second.kind)
        || (is_air(first.kind) && is_air(second.kind))
    {
        return;
    }
    world.block(block_pos(a)).set(BlockType::Air);
    world.block(block_pos(b)).set(BlockType::Air);
    let mut visuals = Vec::new();
    for (state, from, to) in [(&first, a, b), (&second, b, a)] {
        if is_air(state.kind) {
            continue;
        }
        let velocity = Vector {
            x: f64::from(to.x - from.x) * SWAP_VISUAL_SPEED,
            y: f64::from(to.y - from.y) * SWAP_VISUAL_SPEED,
            z: f64::from(to.z - from.z) * SWAP_VISUAL_SPEED,
        };
        match world
            .spawn_falling_block(state.kind, block_center(from), velocity)
            .await
        {
            Ok(entity) => {
                entity.flags.set_gravity(false);
                visuals.push(entity);
            }
            Err(error) => tracing::warn!("échec bloc visuel distorsion : {error}"),
        }
    }
    let world = world.clone();
    tokio::spawn(async move {
        tokio::time::sleep(SWAP_VISUAL_DELAY).await;
        for entity in &visuals {
            entity.remove();
        }
        place_state(&world, a, &second);
        place_state(&world, b, &first);
    });
}

fn place_state(world: &World, at: Cell, state: &BlockState) {
    let properties: Vec<(&str, &str)> = state
        .properties
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    world.block(block_pos(at)).set_with(state.kind, &properties);
}

const fn is_air(kind: BlockType) -> bool {
    matches!(
        kind,
        BlockType::Air | BlockType::CaveAir | BlockType::VoidAir
    )
}

pub const fn wool_material(color: WoolColor) -> Material {
    match color {
        WoolColor::White => Material::WhiteWool,
        WoolColor::Red => Material::RedWool,
        WoolColor::Cyan => Material::CyanWool,
        WoolColor::Blue => Material::BlueWool,
        WoolColor::Orange => Material::OrangeWool,
        WoolColor::LightBlue => Material::LightBlueWool,
        WoolColor::Green => Material::GreenWool,
        WoolColor::Yellow => Material::YellowWool,
        WoolColor::Pink => Material::PinkWool,
        WoolColor::Purple => Material::PurpleWool,
        WoolColor::Brown => Material::BrownWool,
        WoolColor::Black => Material::BlackWool,
        WoolColor::Gray => Material::GrayWool,
    }
}

pub const fn wool_dye_color(color: WoolColor) -> DyeColor {
    match color {
        WoolColor::White => DyeColor::White,
        WoolColor::Red => DyeColor::Red,
        WoolColor::Cyan => DyeColor::Cyan,
        WoolColor::Blue => DyeColor::Blue,
        WoolColor::Orange => DyeColor::Orange,
        WoolColor::LightBlue => DyeColor::LightBlue,
        WoolColor::Green => DyeColor::Green,
        WoolColor::Yellow => DyeColor::Yellow,
        WoolColor::Pink => DyeColor::Pink,
        WoolColor::Purple => DyeColor::Purple,
        WoolColor::Brown => DyeColor::Brown,
        WoolColor::Black => DyeColor::Black,
        WoolColor::Gray => DyeColor::Gray,
    }
}

pub const fn sheep_wool(kind: SheepKind) -> Material {
    wool_material(catalog::wool_color(kind))
}

pub const fn sheep_dye_color(kind: SheepKind) -> DyeColor {
    wool_dye_color(catalog::wool_color(kind))
}

fn distance_squared(a: Vec3, b: Vec3) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)
}

const fn position(v: Vec3) -> Position {
    Position {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

const fn vec3(p: Position) -> Vec3 {
    Vec3 {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

const fn vector(v: Vec3) -> Vector {
    Vector {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

const fn block_pos(c: Cell) -> BlockPos {
    BlockPos {
        x: c.x,
        y: c.y,
        z: c.z,
    }
}

const fn team_color(team: TeamId) -> Color {
    match team {
        TeamId::Red => Color::rgb(255, 70, 70),
        TeamId::Blue => Color::rgb(70, 110, 255),
    }
}

const fn rainbow_color(step: u8) -> Color {
    match step % 6 {
        0 => Color::rgb(255, 60, 60),
        1 => Color::rgb(255, 150, 40),
        2 => Color::rgb(255, 235, 60),
        3 => Color::rgb(70, 220, 90),
        4 => Color::rgb(70, 130, 255),
        _ => Color::rgb(170, 90, 230),
    }
}

const fn particle(kind: ParticleKind) -> Particle {
    match kind {
        ParticleKind::Explosion => Particle::Explosion,
        ParticleKind::Flame => Particle::Flame,
        ParticleKind::Snowflake => Particle::Snowflake,
        ParticleKind::Crit => Particle::Crit,
        ParticleKind::Heart => Particle::Heart,
        ParticleKind::Portal => Particle::Portal,
        ParticleKind::LargeSmoke => Particle::LargeSmoke,
        ParticleKind::TeamDust(team) => Particle::Dust {
            color: team_color(team),
            scale: 1.2,
        },
        ParticleKind::EndRod => Particle::EndRod,
        ParticleKind::Firework => Particle::Firework,
        ParticleKind::Totem => Particle::TotemOfUndying,
        ParticleKind::Note => Particle::Note,
        ParticleKind::ElectricSpark => Particle::ElectricSpark,
        ParticleKind::Rainbow(step) => Particle::Dust {
            color: rainbow_color(step),
            scale: 1.2,
        },
    }
}

const fn sound(kind: SoundKind) -> Sound {
    match kind {
        SoundKind::Explode | SoundKind::Quake => Sound::EntityGenericExplode,
        SoundKind::Ice => Sound::BlockGlassBreak,
        SoundKind::Thunder => Sound::EntityLightningBoltThunder,
        SoundKind::Heal => Sound::EntityPlayerLevelup,
        SoundKind::HealTick => Sound::EntityExperienceOrbPickup,
        SoundKind::Teleport | SoundKind::Warp => Sound::EntityEndermanTeleport,
        SoundKind::Darkness => Sound::EntityGhastShoot,
        SoundKind::Split => Sound::EntitySheepAmbient,
        SoundKind::Intergalactic => Sound::EntityWitherSpawn,
    }
}

const fn effect_type(kind: EffectKind) -> EffectType {
    match kind {
        EffectKind::Slowness => EffectType::Slowness,
        EffectKind::Regeneration => EffectType::Regeneration,
        EffectKind::Blindness => EffectType::Blindness,
        EffectKind::Darkness => EffectType::Darkness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTER: Vec3 = Vec3 {
        x: 0.5,
        y: 64.0,
        z: 0.5,
    };

    #[test]
    fn sphere_cells_stay_within_the_radius_and_include_the_center() {
        let cells = sphere_cells(CENTER, 3.0);
        assert!(cells.contains(&CENTER.cell()));
        for cell in &cells {
            let dx = f64::from(cell.x) + 0.5 - CENTER.x;
            let dy = f64::from(cell.y) + 0.5 - CENTER.y;
            let dz = f64::from(cell.z) + 0.5 - CENTER.z;
            assert!(dx * dx + dy * dy + dz * dz <= 9.0 + 1e-9);
        }
        assert!(cells.len() > 80);
    }

    #[test]
    fn column_cells_start_at_the_impact_and_never_go_below() {
        let cells = column_cells(CENTER, 8.0, 48.0);
        assert!(cells.iter().all(|c| c.y >= 64 && c.y <= 64 + 48));
        assert!(cells.contains(&Cell { x: 0, y: 112, z: 0 }));
        assert!(!cells.contains(&Cell { x: 9, y: 70, z: 0 }));
        assert!(cells.len() > 9000);
    }

    #[test]
    fn bedrock_and_barrier_are_protected() {
        assert!(is_protected(BlockType::Bedrock));
        assert!(is_protected(BlockType::Barrier));
        assert!(!is_protected(BlockType::Stone));
    }
}
