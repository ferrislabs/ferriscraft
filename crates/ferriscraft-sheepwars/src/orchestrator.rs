use std::time::Duration;

use ferriscraft_protocol::{
    HeartbeatRequest, PhaseUpdateRequest, SendToLobbyRequest, ServerPhase,
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

#[derive(Debug, Clone)]
pub struct SheepWarsOrchestratorClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl SheepWarsOrchestratorClient {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
            http,
        }
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(tok) => req.bearer_auth(tok),
            None => req,
        }
    }

    pub async fn send_heartbeat(
        &self,
        req: &HeartbeatRequest,
    ) -> Result<(), OrchestratorClientError> {
        let resp = self
            .apply_auth(
                self.http
                    .post(format!("{}/api/v1/servers/heartbeat", self.base_url)),
            )
            .json(req)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(())
    }

    pub async fn update_phase(
        &self,
        server_id: &str,
        phase: ServerPhase,
    ) -> Result<(), OrchestratorClientError> {
        let resp = self
            .apply_auth(self.http.post(format!(
                "{}/api/v1/servers/{server_id}/phase",
                self.base_url
            )))
            .json(&PhaseUpdateRequest { phase })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(())
    }

    pub async fn send_player_to_lobby(
        &self,
        player_uuid: Uuid,
        reason: Option<String>,
    ) -> Result<(), OrchestratorClientError> {
        let resp = self
            .apply_auth(self.http.post(format!(
                "{}/api/v1/players/{player_uuid}/send-lobby",
                self.base_url
            )))
            .json(&SendToLobbyRequest { reason })
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(())
    }
}
