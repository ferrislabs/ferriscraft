use std::time::{Duration, Instant};

use crate::api::AppState;
use crate::config::OrchestratorConfig;

pub async fn run_static_reconcile_loop(state: AppState, config: OrchestratorConfig) {
    tracing::info!(
        servers = config.static_servers.len(),
        queue_min_players = config.queue_min_players,
        "Réconciliateur statique local démarré"
    );

    let mut ticker = tokio::time::interval(config.reconcile_interval);
    loop {
        ticker.tick().await;
        reconcile_static_once(&state, &config).await;
    }
}

async fn reconcile_static_once(state: &AppState, config: &OrchestratorConfig) {
    let mut statuses = Vec::with_capacity(config.static_servers.len());
    for spec in &config.static_servers {
        let ready = tokio::time::timeout(
            Duration::from_millis(250),
            tokio::net::TcpStream::connect(&spec.address),
        )
        .await
        .is_ok_and(|res| res.is_ok());
        statuses.push((spec.server_id.clone(), spec.address.clone(), ready));
    }

    let now = Instant::now();
    let mut events = Vec::new();
    {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        for (server_id, address, ready) in statuses {
            events.extend(engine.upsert_infra_server(
                now,
                server_id,
                None,
                Some(address),
                ready,
                false,
            ));
        }
        let actions = engine.tick(now);
        events.extend(actions.proxy_events);
    }

    state.emit_proxy_events(events);
}
