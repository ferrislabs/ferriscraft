use rspigot_plugin::{
    ActionError, ActionErrorKind, BlockPos, BlockType, Dimension, Error, GameRule,
    GameRuleSetting, Generator, Server, Weather, World, WorldOptions,
};

use super::layout::{Layout, PlacedBlock};
use super::settings::{ArenaError, ArenaSettings};

const WORLD_SEED: i64 = 0;
const WORLD_NAMESPACE: &str = "rspigot";
const ALREADY_EXISTS: &str = "already exists";

fn created_world_id(name: &str) -> String {
    format!("{WORLD_NAMESPACE}:{name}")
}

fn is_already_created(error: &ActionError) -> bool {
    error.kind == ActionErrorKind::InvalidArgument && error.reason.contains(ALREADY_EXISTS)
}

const DEFAULT_WORLD_NAMES: [&str; 3] = ["world", "overworld", "minecraft:overworld"];

fn is_default_world(name: &str) -> bool {
    DEFAULT_WORLD_NAMES.contains(&name)
}

pub struct Arena {
    pub world: World,
    pub layout: Layout,
    pub reused: bool,
}

impl Arena {
    pub async fn create(server: &Server, settings: ArenaSettings) -> Result<Self, ArenaError> {
        let (world, reused) = if is_default_world(&settings.world_name) {
            (server.world.clone(), false)
        } else if settings.prebuilt {
            let id = if settings.world_name.contains(':') {
                settings.world_name.clone()
            } else {
                created_world_id(&settings.world_name)
            };
            (server.world.by_id(id), false)
        } else {
            let options = WorldOptions {
                name: settings.world_name.clone(),
                dimension: Dimension::Overworld,
                seed: WORLD_SEED,
                generator: Generator::Void,
                structures: false,
            };
            match server.world.create(options).await {
                Ok(world) => (world, false),
                Err(Error::Action(error)) if is_already_created(&error) => {
                    (server.world.by_id(created_world_id(&settings.world_name)), true)
                }
                Err(error) => return Err(error.into()),
            }
        };
        world.time.set(6000);
        world.weather.set(Weather::Clear);
        world
            .game_rule
            .set(GameRuleSetting::new(GameRule::DaylightCycle, false));
        let layout = Layout::new(settings);
        Ok(Self {
            world,
            layout,
            reused,
        })
    }

    pub fn place_chunk(&self, chunk_x: i32, chunk_z: i32) -> Result<usize, ArenaError> {
        let placed = self.layout.blocks_in_chunk(chunk_x, chunk_z);
        let resolved = resolve(&placed)?;
        for (pos, kind) in &resolved {
            self.world.block(*pos).set(*kind);
        }
        Ok(resolved.len())
    }

    pub fn restore_platforms(&self) -> Result<usize, ArenaError> {
        let mut total = 0;
        for (cx, cz) in self.layout.chunks() {
            total += self.place_chunk(cx, cz)?;
        }
        Ok(total)
    }
}

fn resolve(placed: &[PlacedBlock]) -> Result<Vec<(BlockPos, BlockType)>, ArenaError> {
    placed
        .iter()
        .map(|block| {
            let kind = BlockType::from_key(&block.block)
                .ok_or_else(|| ArenaError::UnknownBlock(block.block.clone()))?;
            Ok((
                BlockPos {
                    x: block.x,
                    y: block.y,
                    z: block.z,
                },
                kind,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(block: &str) -> PlacedBlock {
        PlacedBlock {
            x: 1,
            y: 2,
            z: 3,
            block: block.to_owned(),
        }
    }

    #[test]
    fn a_created_world_id_carries_the_server_namespace() {
        assert_eq!(created_world_id("sheepwars"), "rspigot:sheepwars");
    }

    #[test]
    fn only_an_already_exists_refusal_reuses_the_world() {
        let refusal = |kind, reason: &str| ActionError {
            kind,
            reason: reason.to_owned(),
        };
        assert!(is_already_created(&refusal(
            ActionErrorKind::InvalidArgument,
            "the world rspigot:sheepwars already exists"
        )));
        assert!(!is_already_created(&refusal(
            ActionErrorKind::InvalidArgument,
            "`Bad Name` is not a world name"
        )));
        assert!(!is_already_created(&refusal(
            ActionErrorKind::LimitReached,
            "the world already exists"
        )));
    }

    #[test]
    fn resolves_known_blocks() {
        let resolved = resolve(&[placed("minecraft:stone")]).expect("known");
        assert_eq!(resolved[0].0, BlockPos { x: 1, y: 2, z: 3 });
    }

    #[test]
    fn unknown_block_is_a_typed_error() {
        assert!(matches!(
            resolve(&[placed("minecraft:nope")]),
            Err(ArenaError::UnknownBlock(_))
        ));
    }

    #[test]
    fn default_blocks_all_resolve() {
        let layout = Layout::new(ArenaSettings::default());
        assert!(resolve(&layout.blocks()).is_ok());
    }
}
