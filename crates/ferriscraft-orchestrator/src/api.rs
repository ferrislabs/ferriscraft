use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use ferriscraft_progression::{
    EquipError, MemoryProgressionRepository, ProgressionStoreError, Unlock, UnlockError,
};
use ferriscraft_protocol::{
    AdminCoinGrant, EquipRequest, FleetStatusSnapshot, HeartbeatRequest, JoinQueueRequest,
    JoinQueueResponse, LeaveQueueRequest, LeaveQueueResponse, MatchReport, MatchReportAck,
    PhaseUpdateRequest, PlayerIdentityUpdate, PlayerProfile, PlayerQueueStatus,
    ProxyDisconnectRequest, ProxyEvent, ProxyStateSnapshot, PurchaseRequest, SendToLobbyRequest,
    SpectateRequest, TransferAckRequest,
};
use futures_util::stream::{self, Stream};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::engine::{EngineError, FleetEngine};
use crate::progression::{ProgressionError, ProgressionService, ProgressionStore};

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<FleetEngine>>,
    pub proxy_tx: broadcast::Sender<ProxyEvent>,
    pub api_token: Option<String>,
    pub version: Option<String>,
    pub progression: ProgressionService<ProgressionStore>,
}

impl AppState {
    pub fn new(engine: FleetEngine, api_token: Option<String>) -> Self {
        let (proxy_tx, _) = broadcast::channel(512);
        Self {
            engine: Arc::new(Mutex::new(engine)),
            proxy_tx,
            api_token,
            version: None,
            progression: ProgressionService::new(ProgressionStore::Memory(
                MemoryProgressionRepository::default(),
            )),
        }
    }

    #[must_use]
    pub fn with_progression(mut self, service: ProgressionService<ProgressionStore>) -> Self {
        self.progression = service;
        self
    }

    #[must_use]
    pub fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }

    pub fn emit_proxy_events(&self, events: Vec<ProxyEvent>) {
        for ev in events {
            let _ = self.proxy_tx.send(ev);
        }
    }
}

pub enum ApiError {
    Unauthorized,
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Progression(ProgressionError),
}

fn unlock_json(unlock: Unlock) -> serde_json::Value {
    match unlock {
        Unlock::Free => json!({ "free": null }),
        Unlock::Price(price) => json!({ "price": price.get() }),
        Unlock::Rank(rank) => json!({ "rank": rank }),
        Unlock::NetworkLevel(level) => json!({ "network_level": level.get() }),
    }
}

fn progression_response(error: ProgressionError) -> Response {
    let (status, body) = match error {
        ProgressionError::Store(err) => {
            tracing::warn!(error = %err, "Stockage de progression indisponible");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "error": "progression_unavailable" }),
            )
        }
        ProgressionError::EmptyMatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": "empty_match" }),
        ),
        ProgressionError::Unlock(err) => match err {
            UnlockError::AlreadyUnlocked => {
                (StatusCode::CONFLICT, json!({ "error": "already_unlocked" }))
            }
            UnlockError::InsufficientCoins { missing } => (
                StatusCode::PAYMENT_REQUIRED,
                json!({ "error": "insufficient_coins", "missing": missing.get() }),
            ),
            UnlockError::RankRequired(rank) => (
                StatusCode::FORBIDDEN,
                json!({ "error": "rank_required", "rank": rank }),
            ),
            UnlockError::LevelRequired(level) => (
                StatusCode::FORBIDDEN,
                json!({ "error": "level_required", "level": level.get() }),
            ),
            UnlockError::UnknownCosmetic => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "unknown_cosmetic" }),
            ),
        },
        ProgressionError::Equip(err) => match err {
            EquipError::NotUnlocked { requires } => (
                StatusCode::FORBIDDEN,
                json!({ "error": "not_unlocked", "requires": unlock_json(requires) }),
            ),
            EquipError::WrongSlot => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "wrong_slot" }),
            ),
        },
    };
    (status, Json(body)).into_response()
}

impl From<ProgressionError> for ApiError {
    fn from(value: ProgressionError) -> Self {
        Self::Progression(value)
    }
}

impl From<ProgressionStoreError> for ApiError {
    fn from(value: ProgressionStoreError) -> Self {
        Self::Progression(ProgressionError::Store(value))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::Progression(err) => return progression_response(err),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "jeton orchestrateur invalide".to_owned(),
            ),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

impl From<EngineError> for ApiError {
    fn from(value: EngineError) -> Self {
        match value {
            EngineError::UnknownServer(id) => Self::NotFound(format!("serveur inconnu : {id}")),
            EngineError::NotJoinable(id) => Self::Conflict(format!("serveur non joignable : {id}")),
        }
    }
}

fn check_auth(headers: &HeaderMap, expected: Option<&str>) -> Result<(), ApiError> {
    let Some(expected_token) = expected else {
        return Ok(());
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer "))
        .map(str::trim);
    if supplied == Some(expected_token) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/status", get(get_status))
        .route("/api/v1/queue", get(get_status))
        .route("/api/v1/queue/join", post(join_queue))
        .route("/api/v1/queue/leave", post(leave_queue))
        .route("/api/v1/queue/player/{uuid}", get(player_status))
        .route("/api/v1/servers", get(list_servers))
        .route("/api/v1/servers/heartbeat", post(server_heartbeat))
        .route("/api/v1/servers/{server_id}/phase", post(update_phase))
        .route(
            "/api/v1/servers/{server_id}/spectate",
            post(spectate_server),
        )
        .route(
            "/api/v1/players/{uuid}/send-lobby",
            post(send_player_to_lobby),
        )
        .route("/api/v1/matches", post(report_match))
        .route("/api/v1/players/{uuid}/profile", get(player_profile))
        .route("/api/v1/players/{uuid}/identity", post(set_identity))
        .route("/api/v1/players/{uuid}/purchases", post(purchase_cosmetic))
        .route("/api/v1/players/{uuid}/equipped", put(equip_cosmetic))
        .route("/api/v1/players/{uuid}/coins", post(grant_coins))
        .route("/api/v1/proxy/state", get(proxy_state))
        .route("/api/v1/proxy/events", get(proxy_events))
        .route(
            "/api/v1/proxy/transfers/{transfer_id}/ack",
            post(ack_transfer),
        )
        .route(
            "/api/v1/proxy/player-disconnected",
            post(proxy_player_disconnected),
        )
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let snap = {
        let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.snapshot(Instant::now())
    };
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "ready_servers": snap.ready_servers,
            "queue_len": snap.queue_len,
        })),
    )
}

async fn get_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FleetStatusSnapshot>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let mut snap = {
        let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.snapshot(Instant::now())
    };
    snap.orchestrator_version.clone_from(&state.version);
    Ok(Json(snap))
}

async fn list_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FleetStatusSnapshot>, ApiError> {
    get_status(State(state), headers).await
}

async fn join_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JoinQueueRequest>,
) -> Result<Json<JoinQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    if req.player_name.trim().is_empty() {
        return Err(ApiError::BadRequest("player_name vide".to_owned()));
    }
    let player_uuid = req.player_uuid;
    let (mut resp, events) = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let resp = engine.join_queue(now, req);
        let actions = engine.tick(now);
        let snap = engine.snapshot(now);
        let post_status = engine.player_status(player_uuid);
        let mut updated = resp;
        updated.total_queued = post_status.total_queued;
        updated.ready_servers = snap.ready_servers;
        updated.active_servers = snap.ready_servers + snap.in_game_servers + snap.starting_servers;
        if let Some(target) = post_status.target_server {
            updated.status = ferriscraft_protocol::QueueJoinStatus::Transferring;
            updated.position = 0;
            updated.target_server = Some(target);
        } else if let Some(pos) = post_status.position {
            updated.position = pos;
        }
        (updated, actions.proxy_events)
    };
    let _ = &mut resp;
    state.emit_proxy_events(events);
    Ok(Json(resp))
}

async fn leave_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LeaveQueueRequest>,
) -> Result<Json<LeaveQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let resp = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.leave_queue(req.player_uuid, req.reason)
    };
    Ok(Json(resp))
}

async fn player_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
) -> Result<Json<PlayerQueueStatus>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let status = {
        let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.player_status(uuid)
    };
    Ok(Json(status))
}

async fn server_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let events = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let mut events = engine.apply_heartbeat(now, req);
        let actions = engine.tick(now);
        events.extend(actions.proxy_events);
        events
    };
    state.emit_proxy_events(events);
    Ok(StatusCode::NO_CONTENT)
}

async fn update_phase(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<PhaseUpdateRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let events = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let mut events = engine.update_server_phase(now, &server_id, req.phase)?;
        let actions = engine.tick(now);
        events.extend(actions.proxy_events);
        events
    };
    state.emit_proxy_events(events);
    Ok(StatusCode::NO_CONTENT)
}

async fn spectate_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    Json(req): Json<SpectateRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let event = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.queue_spectate_transfer(Instant::now(), &server_id, req.player_uuid)?
    };
    state.emit_proxy_events(vec![event]);
    Ok(StatusCode::ACCEPTED)
}

async fn send_player_to_lobby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<SendToLobbyRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let event = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.queue_lobby_transfer(Instant::now(), uuid, req.reason)
    };
    state.emit_proxy_events(vec![event]);
    Ok(StatusCode::ACCEPTED)
}

async fn proxy_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProxyStateSnapshot>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let snap = {
        let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.proxy_snapshot()
    };
    Ok(Json(snap))
}

async fn proxy_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let rx = state.proxy_tx.subscribe();
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        return Some((Ok(Event::default().event("proxy").data(json)), rx));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "Flux SSE proxy en retard, resynchronisation nécessaire"
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn ack_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<Uuid>,
    Json(req): Json<TransferAckRequest>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    if !req.success {
        tracing::warn!(%transfer_id, message = ?req.message, "Transfert Velocity échoué");
    }
    let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    engine.ack_transfer(transfer_id, req.success);
    Ok(StatusCode::NO_CONTENT)
}

async fn proxy_player_disconnected(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProxyDisconnectRequest>,
) -> Result<Json<LeaveQueueResponse>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let resp = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.leave_queue(
            req.player_uuid,
            ferriscraft_protocol::LeaveReason::ProxyDisconnect,
        )
    };
    Ok(Json(resp))
}

async fn report_match(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<MatchReport>,
) -> Result<Json<MatchReportAck>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    let seen = {
        let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        engine.seen_players(&report.server_id)
    };
    let ack = state.progression.report_match(&report, &seen).await?;
    Ok(Json(ack))
}

async fn player_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.profile(uuid).await?))
}

async fn set_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<PlayerIdentityUpdate>,
) -> Result<StatusCode, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    state.progression.set_identity(uuid, &req).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn purchase_cosmetic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<PurchaseRequest>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.purchase(uuid, req.cosmetic).await?))
}

async fn equip_cosmetic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<EquipRequest>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.equip(uuid, &req).await?))
}

async fn grant_coins(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Json(req): Json<AdminCoinGrant>,
) -> Result<Json<PlayerProfile>, ApiError> {
    check_auth(&headers, state.api_token.as_deref())?;
    Ok(Json(state.progression.grant(uuid, &req).await?))
}
