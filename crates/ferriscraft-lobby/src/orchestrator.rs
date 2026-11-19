use std::sync::RwLock;
use std::time::Duration;

use ferriscraft_protocol::{
    FleetStatusSnapshot, JoinQueueRequest, JoinQueueResponse, LeaveQueueRequest,
    LeaveQueueResponse, LeaveReason, PlayerQueueState, PlayerQueueStatus,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum OrchestratorClientError {
    #[error("orchestrateur injoignable : {0}")]
    Transport(#[from] reqwest::Error),
    #[error("réponse orchestrateur HTTP {0}")]
    HttpStatus(u16),
}

pub struct OrchestratorClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
    last_snapshot: RwLock<Option<FleetStatusSnapshot>>,
}

impl OrchestratorClient {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
            http,
            last_snapshot: RwLock::new(None),
        }
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(tok) => req.bearer_auth(tok),
            None => req,
        }
    }

    pub async fn fetch_status(&self) -> Result<FleetStatusSnapshot, OrchestratorClientError> {
        let req = self.apply_auth(self.http.get(format!("{}/api/v1/status", self.base_url)));
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        let snap: FleetStatusSnapshot = resp.json().await?;
        *self
            .last_snapshot
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(snap.clone());
        Ok(snap)
    }

    pub async fn toggle_queue(
        &self,
        player_uuid: Uuid,
        player_name: &str,
    ) -> Result<ToggleQueueOutcome, OrchestratorClientError> {
        let status_req = self.apply_auth(self.http.get(format!(
            "{}/api/v1/queue/player/{player_uuid}",
            self.base_url
        )));
        let status_resp = status_req.send().await?;
        if !status_resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(
                status_resp.status().as_u16(),
            ));
        }
        let current: PlayerQueueStatus = status_resp.json().await?;
        if current.state == PlayerQueueState::Queued {
            let leave_req = self
                .apply_auth(
                    self.http
                        .post(format!("{}/api/v1/queue/leave", self.base_url)),
                )
                .json(&LeaveQueueRequest {
                    player_uuid,
                    reason: LeaveReason::UserToggle,
                });
            let resp = leave_req.send().await?;
            if !resp.status().is_success() {
                return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
            }
            let left: LeaveQueueResponse = resp.json().await?;
            return Ok(ToggleQueueOutcome::Left(left));
        }

        let join_req = self
            .apply_auth(
                self.http
                    .post(format!("{}/api/v1/queue/join", self.base_url)),
            )
            .json(&JoinQueueRequest {
                player_uuid,
                player_name: player_name.to_owned(),
            });
        let resp = join_req.send().await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        let joined: JoinQueueResponse = resp.json().await?;
        Ok(ToggleQueueOutcome::Joined(joined))
    }

    pub async fn leave_if_queued(&self, player_uuid: Uuid) -> Result<(), OrchestratorClientError> {
        let req = self
            .apply_auth(
                self.http
                    .post(format!("{}/api/v1/queue/leave", self.base_url)),
            )
            .json(&LeaveQueueRequest {
                player_uuid,
                reason: LeaveReason::LobbyQuit,
            });
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(())
    }

    pub fn cached_snapshot(&self) -> Option<FleetStatusSnapshot> {
        self.last_snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn cached_queue_len(&self) -> usize {
        self.last_snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(0, |s| s.queue_len)
    }
}

#[derive(Debug, Clone)]
pub enum ToggleQueueOutcome {
    Joined(JoinQueueResponse),
    Left(LeaveQueueResponse),
}
