use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::config::{BackendMode, OrchestratorConfig};
use ferriscraft_orchestrator::engine::{FleetEngine, FleetPolicy};
use ferriscraft_orchestrator::k8s::run_k8s_reconcile_loop;
use ferriscraft_orchestrator::static_backend::run_static_reconcile_loop;

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
    let policy = FleetPolicy {
        lobby_server_name: config.lobby_server_name.clone(),
        queue_min_players: config.queue_min_players,
        max_players_per_server: config.max_players_per_server,
        min_replicas: config.min_replicas,
        max_replicas: config.max_replicas,
        standby_servers: config.standby_servers,
        idle_ttl: config.idle_ttl,
        starting_timeout: config.starting_timeout,
        transfer_timeout: config.transfer_timeout,
        empty_grace: config.empty_grace,
        ending_timeout: config.ending_timeout,
        heartbeat_timeout: config.heartbeat_timeout,
    };

    let engine = FleetEngine::new(policy);
    let state =
        AppState::new(engine, config.api_token.clone()).with_version(config.version.clone());

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
