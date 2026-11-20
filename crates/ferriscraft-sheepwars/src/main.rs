use ferriscraft_sheepwars::{SheepWarsConfig, SheepWarsPlugin};
use rspigot_plugin::Error;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(err) = dotenvy::dotenv()
        && !err.not_found()
    {
        tracing::warn!("Impossible de charger le fichier .env : {err}");
    }

    let config = SheepWarsConfig::from_env();
    tracing::info!(
        server_id = %config.server_id,
        orchestrator = ?config.orchestrator_url,
        "Démarrage de ferriscraft-sheepwars"
    );
    rspigot_plugin::run(SheepWarsPlugin::new(config)).await
}
