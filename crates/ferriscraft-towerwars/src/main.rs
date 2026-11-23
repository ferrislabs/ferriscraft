use ferriscraft_towerwars::{TowerWarsConfig, TowerWarsPlugin};
use rspigot_plugin::Error;
use tokio::signal::unix::{SignalKind, signal};

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

    let config = TowerWarsConfig::from_env();
    tracing::info!(
        server_id = %config.server_id,
        orchestrator = ?config.orchestrator_url,
        "Démarrage de ferriscraft-towerwars"
    );
    let plugin = TowerWarsPlugin::new(config);
    let shutdown = plugin.shutdown_handle();
    let Ok(mut terminate) = signal(SignalKind::terminate()) else {
        return rspigot_plugin::run(plugin).await;
    };
    tokio::select! {
        result = rspigot_plugin::run(plugin) => result,
        _ = terminate.recv() => {
            tracing::info!("SIGTERM reçu, arrêt propre");
            shutdown.drain().await;
            Ok(())
        }
    }
}
