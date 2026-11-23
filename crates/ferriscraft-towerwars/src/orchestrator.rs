use std::time::Duration;

use ferriscraft_protocol::{
    GameKind, HeartbeatRequest, HeartbeatResponse, MatchReport, MatchReportAck, PhaseUpdateRequest,
    PlayerProfile, PlayerQueueStatus, SendToLobbyRequest, ServerKind, ServerPhase,
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

pub const SERVER_KIND: ServerKind = ServerKind::TowerWars;
pub const GAME_KIND: GameKind = GameKind::TowerWars;
pub const QUEUE_STATUS_TIMEOUT: Duration = Duration::from_secs(2);

pub fn parse_heartbeat_response(status: u16, body: &[u8]) -> Option<HeartbeatResponse> {
    if status != 200 {
        return None;
    }
    serde_json::from_slice(body).ok()
}

pub fn spectates_here(status: &PlayerQueueStatus, server_id: &str) -> bool {
    status.spectate_server.as_deref() == Some(server_id)
}

#[derive(Debug, Clone)]
pub struct TowerWarsOrchestratorClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl TowerWarsOrchestratorClient {
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
    ) -> Result<Option<HeartbeatResponse>, OrchestratorClientError> {
        let resp = self
            .apply_auth(
                self.http
                    .post(format!("{}/api/v1/servers/heartbeat", self.base_url)),
            )
            .json(req)
            .send()
            .await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(status));
        }
        let body = resp.bytes().await.unwrap_or_default();
        Ok(parse_heartbeat_response(status, &body))
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

    pub async fn post_match_report(
        &self,
        report: &MatchReport,
    ) -> Result<MatchReportAck, OrchestratorClientError> {
        let resp = self
            .apply_auth(self.http.post(format!("{}/api/v1/matches", self.base_url)))
            .json(report)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(resp.json().await?)
    }

    pub async fn fetch_profile(
        &self,
        player_uuid: Uuid,
    ) -> Result<PlayerProfile, OrchestratorClientError> {
        let resp = self
            .apply_auth(self.http.get(format!(
                "{}/api/v1/players/{player_uuid}/profile",
                self.base_url
            )))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(resp.json().await?)
    }

    pub async fn fetch_queue_status(
        &self,
        player_uuid: Uuid,
    ) -> Result<PlayerQueueStatus, OrchestratorClientError> {
        let resp = self
            .apply_auth(self.http.get(format!(
                "{}/api/v1/queue/player/{player_uuid}",
                self.base_url
            )))
            .timeout(QUEUE_STATUS_TIMEOUT)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(resp.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use ferriscraft_protocol::PlayerQueueState;

    use super::*;

    fn status(spectate_server: Option<&str>) -> PlayerQueueStatus {
        PlayerQueueStatus {
            state: PlayerQueueState::NotQueued,
            position: None,
            total_queued: 0,
            min_players: 2,
            target_server: None,
            spectate_server: spectate_server.map(str::to_owned),
        }
    }

    #[test]
    fn a_200_json_body_carries_the_expected_players() {
        let parsed = parse_heartbeat_response(200, br#"{"expected_players":3}"#);
        assert_eq!(
            parsed.and_then(|response| response.expected_players),
            Some(3)
        );
        let empty = parse_heartbeat_response(200, b"{}");
        assert_eq!(empty, Some(HeartbeatResponse::default()));
    }

    #[test]
    fn a_204_has_no_instruction() {
        assert_eq!(parse_heartbeat_response(204, b""), None);
        assert_eq!(
            parse_heartbeat_response(204, br#"{"expected_players":3}"#),
            None
        );
    }

    #[test]
    fn a_garbage_body_is_ignored() {
        assert_eq!(parse_heartbeat_response(200, b"<html>nope"), None);
        assert_eq!(parse_heartbeat_response(200, b""), None);
        assert_eq!(
            parse_heartbeat_response(200, br#"{"expected_players":"x"}"#),
            None
        );
    }

    #[test]
    fn spectates_only_when_the_target_is_this_server() {
        assert!(spectates_here(&status(Some("sw-1")), "sw-1"));
        assert!(!spectates_here(&status(Some("sw-2")), "sw-1"));
        assert!(!spectates_here(&status(None), "sw-1"));
    }
}
