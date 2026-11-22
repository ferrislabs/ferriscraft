use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ferriscraft_lobby::orchestrator::{
    OrchestratorClient, OrchestratorClientError, RetryPolicy, ToggleQueueOutcome,
};
use ferriscraft_protocol::{
    JoinQueueResponse, PlayerQueueState, PlayerQueueStatus, QueueJoinStatus,
};
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Clone)]
struct Mock {
    join_attempts: Arc<AtomicUsize>,
    fail_first: usize,
    fail_status: StatusCode,
}

fn router(mock: Mock) -> Router {
    Router::new()
        .route(
            "/api/v1/queue/player/{uuid}",
            get(|| async {
                Json(PlayerQueueStatus {
                    state: PlayerQueueState::NotQueued,
                    position: None,
                    total_queued: 0,
                    min_players: 2,
                    target_server: None,
                    spectate_server: None,
                })
            }),
        )
        .route(
            "/api/v1/queue/join",
            post(|State(mock): State<Mock>| async move {
                let attempt = mock.join_attempts.fetch_add(1, Ordering::SeqCst);
                if attempt < mock.fail_first {
                    return mock.fail_status.into_response();
                }
                Json(JoinQueueResponse {
                    status: QueueJoinStatus::Queued,
                    position: 1,
                    total_queued: 1,
                    min_players: 2,
                    target_server: None,
                    ready_servers: 0,
                    active_servers: 0,
                    max_servers: 4,
                })
                .into_response()
            }),
        )
        .with_state(mock)
}

fn fast_policy(budget: Duration) -> RetryPolicy {
    RetryPolicy {
        budget,
        initial: Duration::from_millis(40),
        max: Duration::from_millis(150),
    }
}

fn mock(fail_first: usize, fail_status: StatusCode) -> Mock {
    Mock {
        join_attempts: Arc::new(AtomicUsize::new(0)),
        fail_first,
        fail_status,
    }
}

async fn serve_on(listener: TcpListener, mock: Mock) {
    let _ = axum::serve(listener, router(mock)).await;
}

fn client(addr: SocketAddr, policy: RetryPolicy) -> OrchestratorClient {
    OrchestratorClient::new(format!("http://{addr}"), None).with_retry(policy)
}

#[tokio::test]
async fn join_is_retried_on_5xx_until_the_orchestrator_recovers() {
    let mock = mock(2, StatusCode::SERVICE_UNAVAILABLE);
    let attempts = Arc::clone(&mock.join_attempts);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve_on(listener, mock));

    let outcome = client(addr, fast_policy(Duration::from_secs(5)))
        .toggle_queue(Uuid::new_v4(), "ferris")
        .await
        .expect("joined after retries");
    assert!(matches!(outcome, ToggleQueueOutcome::Joined(_)));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn plc_16_join_succeeds_when_the_orchestrator_comes_back_after_a_delay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    let mock = mock(0, StatusCode::OK);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let listener = TcpListener::bind(addr).await.expect("rebind");
        serve_on(listener, mock).await;
    });

    let started = Instant::now();
    let outcome = client(addr, fast_policy(Duration::from_secs(10)))
        .toggle_queue(Uuid::new_v4(), "ferris")
        .await
        .expect("joined once the orchestrator answers");
    assert!(matches!(outcome, ToggleQueueOutcome::Joined(_)));
    assert!(started.elapsed() >= Duration::from_millis(600));
}

#[tokio::test]
async fn join_gives_up_after_the_budget_and_reports_the_error() {
    let mock = mock(usize::MAX, StatusCode::BAD_GATEWAY);
    let attempts = Arc::clone(&mock.join_attempts);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve_on(listener, mock));

    let started = Instant::now();
    let err = client(addr, fast_policy(Duration::from_millis(400)))
        .toggle_queue(Uuid::new_v4(), "ferris")
        .await
        .expect_err("budget exhausted");
    assert!(matches!(err, OrchestratorClientError::HttpStatus(502)));
    assert!(attempts.load(Ordering::SeqCst) > 1);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn join_gives_up_after_the_budget_when_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let started = Instant::now();
    let err = client(addr, fast_policy(Duration::from_millis(400)))
        .toggle_queue(Uuid::new_v4(), "ferris")
        .await
        .expect_err("unreachable");
    assert!(matches!(err, OrchestratorClientError::Transport(_)));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    let mock = mock(usize::MAX, StatusCode::CONFLICT);
    let attempts = Arc::clone(&mock.join_attempts);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve_on(listener, mock));

    let err = client(addr, fast_policy(Duration::from_secs(5)))
        .toggle_queue(Uuid::new_v4(), "ferris")
        .await
        .expect_err("conflict");
    assert!(matches!(err, OrchestratorClientError::HttpStatus(409)));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
