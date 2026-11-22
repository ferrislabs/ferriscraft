use std::time::Duration;

use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::engine::{FleetEngine, FleetPolicy};
use ferriscraft_progression::{CoinDelta, CoinReason, ProgressionRepository};
use ferriscraft_protocol::{
    AdminCoinGrant, CosmeticId, CosmeticSlot, EquipRequest, GameKind, HeartbeatRequest,
    MatchOutcome, MatchParticipant, MatchReport, MatchReportAck, PlayerIdentityUpdate,
    PlayerProfile, PlayerRank, PurchaseRequest, ServerPhase,
};
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

const SERVER: &str = "sheepwars-1";

struct Harness {
    base: String,
    state: AppState,
    client: Client,
}

fn policy() -> FleetPolicy {
    FleetPolicy {
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

async fn harness(token: Option<&str>) -> Harness {
    let state = AppState::new(FleetEngine::new(policy()), token.map(str::to_owned));
    let app = build_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Harness {
        base: format!("http://{addr}"),
        state,
        client: Client::new(),
    }
}

fn participant(uuid: Uuid, name: &str, outcome: MatchOutcome, completed: bool) -> MatchParticipant {
    MatchParticipant {
        uuid,
        name: name.to_owned(),
        outcome,
        completed,
    }
}

fn report(match_id: Uuid, participants: Vec<MatchParticipant>) -> MatchReport {
    MatchReport {
        match_id,
        game: GameKind::SheepWars,
        server_id: SERVER.to_owned(),
        forced_start: false,
        duration_secs: 300,
        participants,
    }
}

fn duel(match_id: Uuid, a: Uuid, b: Uuid) -> MatchReport {
    report(
        match_id,
        vec![
            participant(a, "Alice", MatchOutcome::Win, true),
            participant(b, "Bob", MatchOutcome::Loss, true),
        ],
    )
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn heartbeat(&self, players: &[Uuid]) {
        let resp = self
            .client
            .post(self.url("/api/v1/servers/heartbeat"))
            .json(&HeartbeatRequest {
                server_id: SERVER.to_owned(),
                address: Some("10.0.0.5:25565".to_owned()),
                phase: ServerPhase::InGame,
                player_count: u32::try_from(players.len()).unwrap(),
                max_players: 8,
                player_uuids: players.to_vec(),
                version: None,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    async fn send_report(&self, report: &MatchReport) -> Response {
        self.client
            .post(self.url("/api/v1/matches"))
            .json(report)
            .send()
            .await
            .unwrap()
    }

    async fn ack(&self, report: &MatchReport) -> MatchReportAck {
        let resp = self.send_report(report).await;
        assert_eq!(resp.status(), StatusCode::OK);
        resp.json().await.unwrap()
    }

    async fn profile(&self, uuid: Uuid) -> PlayerProfile {
        self.client
            .get(self.url(&format!("/api/v1/players/{uuid}/profile")))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn grant(&self, uuid: Uuid, amount: u64) -> PlayerProfile {
        let resp = self
            .client
            .post(self.url(&format!("/api/v1/players/{uuid}/coins")))
            .json(&AdminCoinGrant {
                amount,
                admin: "root".to_owned(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        resp.json().await.unwrap()
    }

    async fn identity(&self, uuid: Uuid, name: &str, rank: PlayerRank) {
        let resp = self
            .client
            .post(self.url(&format!("/api/v1/players/{uuid}/identity")))
            .json(&PlayerIdentityUpdate {
                name: name.to_owned(),
                rank,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    async fn purchase(&self, uuid: Uuid, cosmetic: CosmeticId) -> Response {
        self.client
            .post(self.url(&format!("/api/v1/players/{uuid}/purchases")))
            .json(&PurchaseRequest { cosmetic })
            .send()
            .await
            .unwrap()
    }

    async fn equip(
        &self,
        uuid: Uuid,
        slot: CosmeticSlot,
        cosmetic: Option<CosmeticId>,
    ) -> Response {
        self.client
            .put(self.url(&format!("/api/v1/players/{uuid}/equipped")))
            .json(&EquipRequest { slot, cosmetic })
            .send()
            .await
            .unwrap()
    }
}

async fn body(resp: Response) -> (StatusCode, Value) {
    let status = resp.status();
    (status, resp.json().await.unwrap())
}

#[tokio::test]
async fn spec_prg_1_win_and_loss_credit_xp_and_coins() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let ack = h.ack(&duel(Uuid::new_v4(), a, b)).await;
    assert!(ack.counted && !ack.duplicate);
    assert_eq!(ack.rewards.len(), 2);
    let pa = h.profile(a).await;
    let pb = h.profile(b).await;
    assert_eq!((pa.coins, pa.network.total_xp), (30, 150));
    assert_eq!((pb.coins, pb.network.total_xp), (10, 50));
    assert_eq!(pa.name, "Alice");
    assert_eq!(pa.games[0].wins, 1);
    assert_eq!(pb.games[0].losses, 1);
}

#[tokio::test]
async fn spec_prg_2_replayed_report_is_a_duplicate_and_credits_nothing() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let rep = duel(Uuid::new_v4(), a, b);
    let first = h.ack(&rep).await;
    let second = h.ack(&rep).await;
    assert!(!first.duplicate);
    assert!(second.counted && second.duplicate);
    assert_eq!(second.rewards.len(), 2);
    assert_eq!(h.profile(a).await.coins, 30);
    assert_eq!(h.profile(b).await.coins, 10);
    assert_eq!(
        h.state
            .progression
            .repository()
            .ledger(a)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn spec_prg_3_forced_start_credits_nothing() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let mut rep = duel(Uuid::new_v4(), a, b);
    rep.forced_start = true;
    let ack = h.ack(&rep).await;
    assert!(!ack.counted && !ack.duplicate && ack.rewards.is_empty());
    assert_eq!(h.profile(a).await.coins, 0);
    assert_eq!(h.profile(b).await.coins, 0);
    assert!(
        h.state
            .progression
            .repository()
            .load(a)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn spec_prg_4_single_player_or_short_round_credits_nothing() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let solo = report(
        Uuid::new_v4(),
        vec![participant(a, "Alice", MatchOutcome::Win, true)],
    );
    assert!(!h.ack(&solo).await.counted);
    let mut short = duel(Uuid::new_v4(), a, b);
    short.duration_secs = 10;
    assert!(!h.ack(&short).await.counted);
    assert_eq!(h.profile(a).await.coins, 0);
    assert_eq!(h.profile(b).await.coins, 0);
}

#[tokio::test]
async fn spec_prg_5_player_who_left_earns_nothing() {
    let h = harness(None).await;
    let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b, c]).await;
    let rep = report(
        Uuid::new_v4(),
        vec![
            participant(a, "Alice", MatchOutcome::Win, true),
            participant(b, "Bob", MatchOutcome::Loss, true),
            participant(c, "Carol", MatchOutcome::Loss, false),
        ],
    );
    let ack = h.ack(&rep).await;
    assert_eq!(ack.rewards.len(), 2);
    assert_eq!(h.profile(c).await.coins, 0);
    assert_eq!(h.profile(c).await.network.total_xp, 0);
}

#[tokio::test]
async fn spec_prg_6_level_up_is_reported() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let first = h.ack(&duel(Uuid::new_v4(), a, b)).await;
    assert!(!first.rewards[0].game_level_up);
    let second = h.ack(&duel(Uuid::new_v4(), a, b)).await;
    let reward = second.rewards.iter().find(|r| r.uuid == a).unwrap();
    assert!(reward.game_level_up);
    assert_eq!(reward.game.level, 2);
    assert!(!reward.network_level_up);
}

#[tokio::test]
async fn spec_prg_7_purchase_debits_unlocks_and_equips() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    h.grant(a, 600).await;
    let resp = h.purchase(a, CosmeticId::SheepTrailFlame).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let profile: PlayerProfile = resp.json().await.unwrap();
    assert_eq!(profile.coins, 100);
    assert!(profile.unlocked.contains(&CosmeticId::SheepTrailFlame));
    let again = h.purchase(a, CosmeticId::SheepTrailFlame).await;
    assert_eq!(body(again).await.1, json!({ "error": "already_unlocked" }));
    let equipped = h
        .equip(
            a,
            CosmeticSlot::SheepTrail,
            Some(CosmeticId::SheepTrailFlame),
        )
        .await;
    assert_eq!(equipped.status(), StatusCode::OK);
    let profile: PlayerProfile = equipped.json().await.unwrap();
    assert_eq!(
        profile.equipped.get(&CosmeticSlot::SheepTrail),
        Some(&CosmeticId::SheepTrailFlame)
    );
    let cleared = h.equip(a, CosmeticSlot::SheepTrail, None).await;
    let profile: PlayerProfile = cleared.json().await.unwrap();
    assert!(profile.equipped.is_empty());
}

#[tokio::test]
async fn spec_prg_8_purchase_without_coins_is_refused() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    h.grant(a, 400).await;
    let (status, json) = body(h.purchase(a, CosmeticId::SheepTrailFlame).await).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        json,
        json!({ "error": "insufficient_coins", "missing": 100 })
    );
    let profile = h.profile(a).await;
    assert_eq!(profile.coins, 400);
    assert!(!profile.unlocked.contains(&CosmeticId::SheepTrailFlame));
}

#[tokio::test]
async fn spec_prg_9_concurrent_purchases_never_overdraw() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    h.grant(a, 600).await;
    let (first, second) = tokio::join!(
        h.purchase(a, CosmeticId::SheepTrailFlame),
        h.purchase(a, CosmeticId::SheepTrailHearts)
    );
    let mut statuses = [first.status(), second.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::PAYMENT_REQUIRED]);
    assert_eq!(h.profile(a).await.coins, 100);
}

#[tokio::test]
async fn spec_prg_10_vip_bonus_applies_to_coins_not_xp() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.identity(a, "Alice", PlayerRank::Vip).await;
    h.heartbeat(&[a, b]).await;
    let rep = report(
        Uuid::new_v4(),
        vec![
            participant(a, "Alice", MatchOutcome::Win, true),
            participant(b, "Bob", MatchOutcome::Win, true),
        ],
    );
    h.ack(&rep).await;
    let (vip, member) = (h.profile(a).await, h.profile(b).await);
    assert_eq!(vip.network.total_xp, member.network.total_xp);
    assert_eq!(member.coins, 30);
    assert_eq!(vip.coins, 45);
    assert_eq!(vip.rank, PlayerRank::Vip);
}

#[tokio::test]
async fn spec_prg_11_rank_cosmetic_needs_the_rank() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    h.identity(a, "Alice", PlayerRank::Member).await;
    let (status, json) = body(
        h.equip(a, CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json,
        json!({ "error": "not_unlocked", "requires": { "rank": "vip" } })
    );
    let (status, json) = body(h.purchase(a, CosmeticId::TrailEmerald).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json, json!({ "error": "rank_required", "rank": "vip" }));
    h.identity(a, "Alice", PlayerRank::Vip).await;
    let ok = h
        .equip(a, CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailEmerald))
        .await;
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn spec_prg_17_admin_grant_is_journaled_with_the_admin() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    let profile = h.grant(a, 250).await;
    assert_eq!(profile.coins, 250);
    let ledger = h.state.progression.repository().ledger(a).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(
        ledger[0].delta,
        CoinDelta::Credit(ferriscraft_progression::Coins(250))
    );
    assert_eq!(ledger[0].reason, CoinReason::Admin("root".to_owned()));
}

#[tokio::test]
async fn participant_never_seen_in_a_heartbeat_is_ignored() {
    let h = harness(None).await;
    let (a, b, ghost) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    let rep = report(
        Uuid::new_v4(),
        vec![
            participant(a, "Alice", MatchOutcome::Win, true),
            participant(b, "Bob", MatchOutcome::Loss, true),
            participant(ghost, "Ghost", MatchOutcome::Loss, true),
        ],
    );
    let ack = h.ack(&rep).await;
    assert_eq!(ack.rewards.len(), 2);
    assert!(ack.rewards.iter().all(|r| r.uuid != ghost));
    assert_eq!(h.profile(ghost).await.coins, 0);
    assert!(
        h.state
            .progression
            .repository()
            .load(ghost)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn report_from_an_unknown_server_credits_nothing() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let ack = h.ack(&duel(Uuid::new_v4(), a, b)).await;
    assert!(!ack.counted);
    assert_eq!(h.profile(a).await.coins, 0);
}

#[tokio::test]
async fn empty_report_is_unprocessable() {
    let h = harness(None).await;
    let (status, json) = body(h.send_report(&report(Uuid::new_v4(), Vec::new())).await).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json, json!({ "error": "empty_match" }));
}

#[tokio::test]
async fn unknown_cosmetic_and_wrong_slot_are_unprocessable() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    h.grant(a, 5_000).await;
    let (status, json) = body(h.purchase(a, CosmeticId::Unknown).await).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json, json!({ "error": "unknown_cosmetic" }));
    let (status, json) = body(
        h.equip(a, CosmeticSlot::SheepTrail, Some(CosmeticId::TrailFlame))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json, json!({ "error": "wrong_slot" }));
}

#[tokio::test]
async fn level_gated_cosmetic_reports_the_required_level() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    let (status, json) = body(h.purchase(a, CosmeticId::TrailMagic).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json, json!({ "error": "level_required", "level": 10 }));
    let (status, json) = body(
        h.equip(a, CosmeticSlot::LobbyTrail, Some(CosmeticId::TrailFlame))
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json,
        json!({ "error": "not_unlocked", "requires": { "price": 750 } })
    );
}

#[tokio::test]
async fn unknown_player_has_an_empty_profile() {
    let h = harness(None).await;
    let a = Uuid::new_v4();
    let profile = h.profile(a).await;
    assert_eq!(profile.uuid, a);
    assert_eq!(profile.coins, 0);
    assert_eq!(profile.rank, PlayerRank::Guest);
}

#[tokio::test]
async fn progression_routes_require_the_bearer_token() {
    let h = harness(Some("secret")).await;
    let a = Uuid::new_v4();
    let resp = h
        .client
        .get(h.url(&format!("/api/v1/players/{a}/profile")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = h
        .client
        .post(h.url("/api/v1/matches"))
        .json(&duel(Uuid::new_v4(), a, Uuid::new_v4()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = h
        .client
        .get(h.url(&format!("/api/v1/players/{a}/profile")))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn ledger_sum_equals_the_balance() {
    let h = harness(None).await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    h.heartbeat(&[a, b]).await;
    h.ack(&duel(Uuid::new_v4(), a, b)).await;
    h.ack(&duel(Uuid::new_v4(), a, b)).await;
    h.grant(a, 500).await;
    assert_eq!(
        h.purchase(a, CosmeticId::SheepTrailHearts).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        h.purchase(a, CosmeticId::TrailFlame).await.status(),
        StatusCode::PAYMENT_REQUIRED
    );
    let ledger = h.state.progression.repository().ledger(a).await.unwrap();
    let sum: i128 = ledger.iter().map(|m| m.delta.signed()).sum();
    let balance = h.profile(a).await.coins;
    assert_eq!(sum, i128::from(balance));
    assert_eq!(balance, 60);
}
