pub mod arena;
pub mod config;
pub mod domain;
pub mod game;
pub mod hud;
pub mod orchestrator;
pub mod plugin;
pub mod sheep;

pub use config::SheepWarsConfig;
pub use plugin::{SheepWarsPlugin, ShutdownHandle};
