use std::time::{Duration, Instant};

use crate::api::AppState;
use crate::config::{GameConfig, OrchestratorConfig};

pub async fn run_static_reconcile_loop(state: AppState, config: OrchestratorConfig) {
    for game in config.games() {
        tracing::info!(
            game = game.spec().display_name,
            servers = game.static_servers.len(),
            queue_min_players = game.queue_min_players,
            "Réconciliateur statique local démarré"
        );
    }

    let mut ticker = tokio::time::interval(config.reconcile_interval);
    loop {
        ticker.tick().await;
        for game in config.games() {
            reconcile_static_once(&state, game).await;
        }
    }
}

async fn reconcile_static_once(state: &AppState, game: &GameConfig) {
    let mut statuses = Vec::with_capacity(game.static_servers.len());
    for spec in &game.static_servers {
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
        let mut games = state.lock_games();
        let Ok(engine) = games.engine_mut(game.game) else {
            return;
        };
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
    state.flush_queue_logged().await;
}
