use std::time::Duration;

use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::engine::{FleetEngine, FleetPolicy};
use ferriscraft_protocol::{
    DrainResponse, FleetStatusSnapshot, HeartbeatRequest, PROXY_VERSION_HEADER, PlayerQueueStatus,
    ServerKind, ServerPhase, SpectateRequest,
};
use tokio::net::TcpListener;
use uuid::Uuid;

async fn spawn() -> String {
    let policy = FleetPolicy {
        lobby_server_name: "lobby".to_owned(),
        queue_min_players: 2,
        max_players_per_server: 8,
        min_replicas: 1,
        max_replicas: 4,
        standby_servers: 1,
        idle_ttl: Duration::from_secs(60),
        starting_timeout: Duration::from_secs(60),
        transfer_timeout: Duration::from_secs(15),
        empty_grace: Duration::from_secs(30),
        ending_timeout: Duration::from_secs(60),
        heartbeat_timeout: Duration::from_secs(30),
    };
    let state = AppState::new(FleetEngine::new(policy), Some("tok".to_owned()));
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn beat(base: &str, id: &str, kind: ServerKind, players: &[u128]) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/servers/heartbeat"))
        .bearer_auth("tok")
        .json(&HeartbeatRequest {
            server_id: id.to_owned(),
            address: Some("10.0.0.5:25565".to_owned()),
            kind,
            phase: ServerPhase::Waiting,
            player_count: players.len() as u32,
            max_players: 50,
            player_uuids: players.iter().map(|n| Uuid::from_u128(*n)).collect(),
            version: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
}

async fn drain(base: &str, id: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/api/v1/servers/{id}/drain"))
        .bearer_auth("tok")
        .send()
        .await
        .unwrap()
}

async fn status(base: &str) -> FleetStatusSnapshot {
    reqwest::Client::new()
        .get(format!("{base}/api/v1/status"))
        .bearer_auth("tok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn drain_answers_unknown_not_a_lobby_no_other_lobby_and_remaining_players() {
    let base = spawn().await;

    let unauth = reqwest::Client::new()
        .post(format!("{base}/api/v1/servers/lobby-a/drain"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let unknown = drain(&base, "lobby-a").await;
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = unknown.json().await.unwrap();
    assert_eq!(body["error"], "UnknownServer");

    beat(&base, "sheepwars-1", ServerKind::SheepWars, &[]).await;
    let not_lobby = drain(&base, "sheepwars-1").await;
    assert_eq!(not_lobby.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = not_lobby.json().await.unwrap();
    assert_eq!(body["error"], "NotALobby");

    beat(&base, "lobby-a", ServerKind::Lobby, &[1, 2, 3]).await;
    let alone = drain(&base, "lobby-a").await;
    assert_eq!(alone.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = alone.json().await.unwrap();
    assert_eq!(body["error"], "NoOtherLobby");

    beat(&base, "lobby-b", ServerKind::Lobby, &[]).await;
    let ok = drain(&base, "lobby-a").await;
    assert_eq!(ok.status(), reqwest::StatusCode::OK);
    let body: DrainResponse = ok.json().await.unwrap();
    assert_eq!(body.remaining_players, 3);

    beat(&base, "lobby-a", ServerKind::Lobby, &[]).await;
    let polled: DrainResponse = drain(&base, "lobby-a").await.json().await.unwrap();
    assert_eq!(polled.remaining_players, 0);

    let snapshot = status(&base).await;
    assert_eq!(snapshot.servers.len(), 1);
    assert_eq!(snapshot.lobbies.len(), 2);
    let a = snapshot
        .lobbies
        .iter()
        .find(|l| l.server_id == "lobby-a")
        .unwrap();
    assert!(a.draining);
    assert_eq!(a.kind, ServerKind::Lobby);
}

#[tokio::test]
async fn lobby_heartbeat_is_announced_and_listed_apart_from_sheepwars_servers() {
    let base = spawn().await;
    beat(&base, "lobby-a", ServerKind::Lobby, &[]).await;
    let proxy: ferriscraft_protocol::ProxyStateSnapshot = reqwest::Client::new()
        .get(format!("{base}/api/v1/proxy/state"))
        .bearer_auth("tok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(proxy.servers.len(), 1);
    assert_eq!(proxy.servers[0].kind, ServerKind::Lobby);
    let snapshot = status(&base).await;
    assert!(snapshot.servers.is_empty());
    assert_eq!(snapshot.lobbies[0].server_id, "lobby-a");
}

#[tokio::test]
async fn proxy_version_header_is_exposed_in_the_status() {
    let base = spawn().await;
    assert_eq!(status(&base).await.proxy_version, None);
    reqwest::Client::new()
        .get(format!("{base}/api/v1/proxy/state"))
        .bearer_auth("tok")
        .header(PROXY_VERSION_HEADER, "velocity-1.2.3")
        .send()
        .await
        .unwrap();
    assert_eq!(
        status(&base).await.proxy_version.as_deref(),
        Some("velocity-1.2.3")
    );

    let events = reqwest::Client::new()
        .get(format!("{base}/api/v1/proxy/events"))
        .bearer_auth("tok")
        .header(PROXY_VERSION_HEADER, "velocity-1.2.4")
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), reqwest::StatusCode::OK);
    assert_eq!(
        status(&base).await.proxy_version.as_deref(),
        Some("velocity-1.2.4")
    );
    drop(events);
}

#[tokio::test]
async fn spectate_records_the_server_in_the_player_status() {
    let base = spawn().await;
    beat(&base, "sheepwars-1", ServerKind::SheepWars, &[]).await;
    let player = Uuid::from_u128(7);
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/servers/sheepwars-1/spectate"))
        .bearer_auth("tok")
        .json(&SpectateRequest {
            player_uuid: player,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    let status: PlayerQueueStatus = reqwest::Client::new()
        .get(format!("{base}/api/v1/queue/player/{player}"))
        .bearer_auth("tok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.spectate_server.as_deref(), Some("sheepwars-1"));
}
