use ferriscraft_lobby::FerrisCraftLobby;
use ferriscraft_lobby::config::{EnvConfig, LayoutStore};
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

    let config = EnvConfig::from_env();
    let layout = match LayoutStore::load_or_create(&config.layout_path) {
        Ok(store) => store,
        Err(err) => {
            tracing::error!("Impossible d'initialiser la configuration spatiale : {err}");
            std::process::exit(2);
        }
    };

    tracing::info!(
        "Démarrage de ferriscraft-lobby (layout={})",
        config.layout_path.display()
    );

    rspigot_plugin::run(FerrisCraftLobby::new(config, layout)).await
}
