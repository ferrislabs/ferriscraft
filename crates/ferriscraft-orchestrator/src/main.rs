use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::config::{BackendMode, OrchestratorConfig};
use std::time::Instant;

use ferriscraft_orchestrator::engine::FleetPolicy;
use ferriscraft_orchestrator::k8s::run_k8s_reconcile_loop;
use ferriscraft_orchestrator::progression::{ProgressionService, ProgressionStore};
use ferriscraft_orchestrator::queue::{
    MemoryQueueRepository, PgQueueRepository, QueueStore, restore_games,
};
use ferriscraft_orchestrator::static_backend::run_static_reconcile_loop;
use ferriscraft_progression::MemoryProgressionRepository;
use ferriscraft_progression_pg::PgProgressionRepository;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = OrchestratorConfig::from_env()?;
    let lobby_host = config.policy(&config.sheepwars);
    let others: Vec<FleetPolicy> = config
        .towerwars
        .iter()
        .map(|game| config.policy(game))
        .collect();

    let queue_store = match config.progression_database_url.as_deref() {
        Some(url) => match PgQueueRepository::connect(url).await {
            Ok(repo) => QueueStore::Postgres(repo),
            Err(err) => {
                tracing::error!(error = %err, "connexion à la base de la file d'attente impossible");
                std::process::exit(1);
            }
        },
        None => {
            tracing::warn!("file d'attente en mémoire, perdue au redémarrage");
            QueueStore::Memory(MemoryQueueRepository::default())
        }
    };
    let games = match restore_games(lobby_host, others, &queue_store, Instant::now()).await {
        Ok(games) => games,
        Err(err) => {
            tracing::error!(error = %err, "rechargement de la file d'attente impossible");
            std::process::exit(1);
        }
    };
    let store = match config.progression_database_url.as_deref() {
        Some(url) => match PgProgressionRepository::connect(url).await {
            Ok(repo) => ProgressionStore::Postgres(repo),
            Err(err) => {
                tracing::error!(error = %err, "connexion à la base de progression impossible");
                std::process::exit(1);
            }
        },
        None => {
            tracing::warn!("progression en mémoire, perdue au redémarrage");
            ProgressionStore::Memory(MemoryProgressionRepository::default())
        }
    };
    let state = AppState::from_games(games, config.api_token.clone())
        .with_queue(queue_store)
        .with_version(config.version.clone())
        .with_progression(ProgressionService::new(store));

    let reconciler_state = state.clone();
    let reconciler_config = config.clone();
    tokio::spawn(async move {
        match reconciler_config.backend {
            BackendMode::Kubernetes => {
                run_k8s_reconcile_loop(reconciler_state, reconciler_config).await
            }
            BackendMode::Static => {
                run_static_reconcile_loop(reconciler_state, reconciler_config).await
            }
        }
    });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(bind = %config.bind_addr, backend = ?config.backend, "ferriscraft-orchestrator en écoute");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Arrêt propre de ferriscraft-orchestrator");
}
