use std::time::Duration;

use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::engine::{FleetEngine, FleetPolicy};
use ferriscraft_protocol::{
    FleetStatusSnapshot, GameKind, HeartbeatRequest, HeartbeatResponse, JoinQueueRequest,
    JoinQueueResponse, PlayerQueueState, PlayerQueueStatus, ProxyStateSnapshot, QueueJoinStatus,
    ServerKind, ServerPhase,
};
use reqwest::{Client, StatusCode};
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

fn policy(game: GameKind) -> FleetPolicy {
    FleetPolicy {
        game,
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
    }
}

async fn serve(state: AppState) -> String {
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn both_games() -> String {
    serve(
        AppState::new(FleetEngine::new(policy(GameKind::SheepWars)), None)
            .with_game_engine(FleetEngine::new(policy(GameKind::TowerWars))),
    )
    .await
}

async fn sheepwars_only() -> String {
    serve(AppState::new(
        FleetEngine::new(policy(GameKind::SheepWars)),
        None,
    ))
    .await
}

fn player(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

async fn heartbeat(base: &str, kind: ServerKind, server: &str) -> StatusCode {
    Client::new()
        .post(format!("{base}/api/v1/servers/heartbeat"))
        .json(&HeartbeatRequest {
            server_id: server.to_owned(),
            address: Some("10.0.0.5:25565".to_owned()),
            kind,
            phase: ServerPhase::Waiting,
            player_count: 0,
            max_players: 8,
            player_uuids: Vec::new(),
            version: None,
        })
        .send()
        .await
        .unwrap()
        .status()
}

async fn join(base: &str, n: u128, game: GameKind) -> JoinQueueResponse {
    join_sized(base, n, game, 0).await
}

async fn heartbeat_raw(
    base: &str,
    server: &str,
    phase: ServerPhase,
    players: &[u128],
) -> reqwest::Response {
    Client::new()
        .post(format!("{base}/api/v1/servers/heartbeat"))
        .json(&HeartbeatRequest {
            server_id: server.to_owned(),
            address: Some("10.0.0.5:25565".to_owned()),
            kind: ServerKind::TowerWars,
            phase,
            player_count: players.len() as u32,
            max_players: 12,
            player_uuids: players.iter().map(|n| player(*n)).collect(),
            version: None,
        })
        .send()
        .await
        .unwrap()
}

async fn join_sized(base: &str, n: u128, game: GameKind, match_size: u8) -> JoinQueueResponse {
    let resp = Client::new()
        .post(format!("{base}/api/v1/queue/join"))
        .json(&JoinQueueRequest {
            player_uuid: player(n),
            player_name: format!("p{n}"),
            game,
            match_size,
        })
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    resp.json().await.unwrap()
}

async fn status(base: &str, n: u128, game: &str) -> PlayerQueueStatus {
    reqwest::get(format!(
        "{base}/api/v1/queue/player/{}?game={game}",
        player(n)
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap()
}

#[tokio::test]
async fn spec_tw_26_towerwars_players_are_sent_to_towerwars_servers() {
    let base = both_games().await;
    assert!(
        heartbeat(&base, ServerKind::SheepWars, "sheepwars-1")
            .await
            .is_success()
    );
    assert!(
        heartbeat(&base, ServerKind::TowerWars, "towerwars-1")
            .await
            .is_success()
    );

    join_sized(&base, 1, GameKind::TowerWars, 2).await;
    let second = join_sized(&base, 2, GameKind::TowerWars, 2).await;
    assert_eq!(second.status, QueueJoinStatus::Transferring);
    assert_eq!(second.target_server.as_deref(), Some("towerwars-1"));
    assert_eq!(second.min_players, 2);

    join(&base, 3, GameKind::SheepWars).await;
    let fourth = join(&base, 4, GameKind::SheepWars).await;
    assert_eq!(fourth.target_server.as_deref(), Some("sheepwars-1"));

    let proxy: ProxyStateSnapshot = reqwest::get(format!("{base}/api/v1/proxy/state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for transfer in &proxy.pending_transfers {
        let expected = if transfer.player_uuid == player(1) || transfer.player_uuid == player(2) {
            "towerwars-1"
        } else {
            "sheepwars-1"
        };
        assert_eq!(transfer.target_server, expected);
        assert!(transfer.reason.ends_with("_matchmaking"));
    }
    assert_eq!(proxy.pending_transfers.len(), 4);
}

#[tokio::test]
async fn spec_tw_26_the_sheepwars_queue_is_not_affected_by_towerwars_players() {
    let base = both_games().await;
    join(&base, 1, GameKind::TowerWars).await;
    let sheep = status(&base, 9, "sheep_wars").await;
    assert_eq!(sheep.state, PlayerQueueState::NotQueued);
    assert_eq!(sheep.total_queued, 0);
    let tower = status(&base, 1, "tower_wars").await;
    assert_eq!(tower.state, PlayerQueueState::Queued);
    assert_eq!(tower.total_queued, 1);
}

#[tokio::test]
async fn spec_tw_26_a_player_in_one_queue_is_already_queued_in_the_other() {
    let base = both_games().await;
    assert_eq!(
        join(&base, 1, GameKind::SheepWars).await.status,
        QueueJoinStatus::Queued
    );
    assert_eq!(
        join(&base, 1, GameKind::TowerWars).await.status,
        QueueJoinStatus::AlreadyQueued
    );
    let tower_view = status(&base, 9, "tower_wars").await;
    assert_eq!(tower_view.total_queued, 0);
    let own = status(&base, 1, "tower_wars").await;
    assert_eq!(own.state, PlayerQueueState::Queued);
}

#[tokio::test]
async fn a_join_body_without_game_is_a_sheepwars_join() {
    let base = both_games().await;
    let resp = Client::new()
        .post(format!("{base}/api/v1/queue/join"))
        .json(&json!({ "player_uuid": player(1), "player_name": "p1" }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let sheep = status(&base, 1, "sheep_wars").await;
    assert_eq!(sheep.state, PlayerQueueState::Queued);
    let default_query: PlayerQueueStatus =
        reqwest::get(format!("{base}/api/v1/queue/player/{}", player(1)))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(default_query.state, PlayerQueueState::Queued);
}

#[tokio::test]
async fn a_leave_body_without_game_leaves_the_sheepwars_queue() {
    let base = both_games().await;
    join(&base, 1, GameKind::SheepWars).await;
    let resp = Client::new()
        .post(format!("{base}/api/v1/queue/leave"))
        .json(&json!({ "player_uuid": player(1) }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    assert_eq!(
        status(&base, 1, "sheep_wars").await.state,
        PlayerQueueState::NotQueued
    );
}

#[tokio::test]
async fn towerwars_requests_are_rejected_while_the_game_is_disabled() {
    let base = sheepwars_only().await;
    let resp = Client::new()
        .post(format!("{base}/api/v1/queue/join"))
        .json(&json!({ "player_uuid": player(1), "player_name": "p1", "game": "tower_wars" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        heartbeat(&base, ServerKind::TowerWars, "towerwars-1").await,
        StatusCode::BAD_REQUEST
    );
    assert!(
        heartbeat(&base, ServerKind::SheepWars, "sheepwars-1")
            .await
            .is_success()
    );
}

#[tokio::test]
async fn status_lists_the_servers_of_every_game() {
    let base = both_games().await;
    heartbeat(&base, ServerKind::SheepWars, "sheepwars-1").await;
    heartbeat(&base, ServerKind::TowerWars, "towerwars-1").await;
    let snapshot: FleetStatusSnapshot = reqwest::get(format!("{base}/api/v1/status"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut seen: Vec<(String, ServerKind)> = snapshot
        .servers
        .iter()
        .map(|s| (s.server_id.clone(), s.kind))
        .collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        [
            ("sheepwars-1".to_owned(), ServerKind::SheepWars),
            ("towerwars-1".to_owned(), ServerKind::TowerWars)
        ]
    );
    assert_eq!(snapshot.games.len(), 2);
}

#[tokio::test]
async fn heartbeat_answers_204_until_a_match_size_is_assigned_then_200_with_a_body() {
    let base = both_games().await;
    let idle = heartbeat_raw(&base, "towerwars-1", ServerPhase::Waiting, &[]).await;
    assert_eq!(idle.status(), StatusCode::NO_CONTENT);
    assert!(idle.bytes().await.unwrap().is_empty());

    let duo = join_sized(&base, 1, GameKind::TowerWars, 2).await;
    assert_eq!(duo.status, QueueJoinStatus::Queued);
    assert_eq!(duo.min_players, 2);
    let still_waiting = heartbeat_raw(&base, "towerwars-1", ServerPhase::Waiting, &[]).await;
    assert_eq!(still_waiting.status(), StatusCode::NO_CONTENT);

    join_sized(&base, 2, GameKind::TowerWars, 2).await;
    let assigned = heartbeat_raw(&base, "towerwars-1", ServerPhase::Waiting, &[]).await;
    assert_eq!(assigned.status(), StatusCode::OK);
    let body: HeartbeatResponse = assigned.json().await.unwrap();
    assert_eq!(body.expected_players, Some(2));

    let arriving = heartbeat_raw(&base, "towerwars-1", ServerPhase::Waiting, &[1]).await;
    assert_eq!(arriving.status(), StatusCode::OK);

    let in_game = heartbeat_raw(&base, "towerwars-1", ServerPhase::InGame, &[1, 2]).await;
    assert_eq!(in_game.status(), StatusCode::OK);

    let back = heartbeat_raw(&base, "towerwars-1", ServerPhase::Waiting, &[]).await;
    assert_eq!(back.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn sheepwars_heartbeats_never_carry_a_body() {
    let base = both_games().await;
    assert!(
        heartbeat(&base, ServerKind::SheepWars, "sheepwars-1")
            .await
            .is_success()
    );
    join(&base, 1, GameKind::SheepWars).await;
    join(&base, 2, GameKind::SheepWars).await;
    assert_eq!(
        heartbeat(&base, ServerKind::SheepWars, "sheepwars-1").await,
        StatusCode::NO_CONTENT
    );
}
