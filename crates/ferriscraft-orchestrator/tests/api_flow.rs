use std::sync::Arc;
use std::time::Duration;

use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::engine::{FleetEngine, FleetPolicy};
use ferriscraft_protocol::{
    FleetStatusSnapshot, HeartbeatRequest, JoinQueueRequest, JoinQueueResponse, LeaveQueueRequest,
    LeaveReason, PlayerQueueState, PlayerQueueStatus, ProxyStateSnapshot, QueueJoinStatus,
    SendToLobbyRequest, ServerPhase, TransferAckRequest,
};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use uuid::Uuid;

async fn spawn_test_server(token: Option<&str>) -> String {
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
        ending_grace: Duration::from_secs(60),
    };
    let state = AppState::new(FleetEngine::new(policy), token.map(str::to_owned));
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn end_to_end_queue_matchmaking_sse_and_auth() {
    let base = spawn_test_server(Some("secret-token")).await;
    let client = reqwest::Client::new();

    // Unauthorized request must fail
    let unauth = client
        .get(format!("{base}/api/v1/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Subscribe to SSE stream as Velocity plugin would
    let sse_resp = client
        .get(format!("{base}/api/v1/proxy/events"))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(sse_resp.status(), reqwest::StatusCode::OK);
    let sse_lines = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let sse_lines_bg = Arc::clone(&sse_lines);
    let mut byte_stream = sse_resp.bytes_stream();
    tokio::spawn(async move {
        while let Some(Ok(chunk)) = byte_stream.next().await {
            let text = String::from_utf8_lossy(&chunk).to_string();
            sse_lines_bg.lock().await.push(text);
        }
    });

    // Register a SheepWars server via heartbeat
    let hb = client
        .post(format!("{base}/api/v1/servers/heartbeat"))
        .bearer_auth("secret-token")
        .json(&HeartbeatRequest {
            server_id: "sheepwars-1".to_owned(),
            address: Some("10.0.0.5:25565".to_owned()),
            phase: ServerPhase::Waiting,
            player_count: 0,
            max_players: 8,
            player_uuids: Vec::new(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(hb.status(), reqwest::StatusCode::NO_CONTENT);

    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();

    let r1: JoinQueueResponse = client
        .post(format!("{base}/api/v1/queue/join"))
        .bearer_auth("secret-token")
        .json(&JoinQueueRequest {
            player_uuid: p1,
            player_name: "Alice".to_owned(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r1.status, QueueJoinStatus::Queued);
    assert_eq!(r1.position, 1);

    let r2: JoinQueueResponse = client
        .post(format!("{base}/api/v1/queue/join"))
        .bearer_auth("secret-token")
        .json(&JoinQueueRequest {
            player_uuid: p2,
            player_name: "Bob".to_owned(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r2.total_queued, 0);

    // Proxy state should show sheepwars-1 and 2 pending transfers
    let proxy: ProxyStateSnapshot = client
        .get(format!("{base}/api/v1/proxy/state"))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(proxy.servers.len(), 1);
    assert_eq!(proxy.servers[0].name, "sheepwars-1");
    assert_eq!(proxy.pending_transfers.len(), 2);

    // Lobby quit must not cancel p1's transfer
    client
        .post(format!("{base}/api/v1/queue/leave"))
        .bearer_auth("secret-token")
        .json(&LeaveQueueRequest {
            player_uuid: p1,
            reason: LeaveReason::LobbyQuit,
        })
        .send()
        .await
        .unwrap();

    let p1_status: PlayerQueueStatus = client
        .get(format!("{base}/api/v1/queue/player/{p1}"))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p1_status.state, PlayerQueueState::Transferring);

    // Ack transfer
    let tid = proxy.pending_transfers[0].transfer_id;
    let ack = client
        .post(format!("{base}/api/v1/proxy/transfers/{tid}/ack"))
        .bearer_auth("secret-token")
        .json(&TransferAckRequest {
            success: true,
            message: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(ack.status(), reqwest::StatusCode::NO_CONTENT);

    // Send player back to lobby
    let send_lobby = client
        .post(format!("{base}/api/v1/players/{p2}/send-lobby"))
        .bearer_auth("secret-token")
        .json(&SendToLobbyRequest {
            reason: Some("fin de partie".to_owned()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(send_lobby.status(), reqwest::StatusCode::ACCEPTED);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let combined_sse = sse_lines.lock().await.join("");
    assert!(combined_sse.contains("server_upsert"));
    assert!(combined_sse.contains("transfer_player"));

    let status: FleetStatusSnapshot = client
        .get(format!("{base}/api/v1/status"))
        .bearer_auth("secret-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.ready_servers, 1);
    assert_eq!(status.max_replicas, 4);
}
