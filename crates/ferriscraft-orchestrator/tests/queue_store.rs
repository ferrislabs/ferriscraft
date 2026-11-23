use std::time::{Duration, Instant};

use chrono::Utc;
use ferriscraft_orchestrator::api::{AppState, build_router};
use ferriscraft_orchestrator::engine::FleetPolicy;
use ferriscraft_orchestrator::queue::{
    FIRST_SEQ, MemoryQueueRepository, NewQueueEntry, QueueEntryState, QueueRepository, QueueStore,
    restore_engine, restore_games,
};
use ferriscraft_protocol::{
    GameKind, HeartbeatRequest, JoinQueueRequest, PlayerQueueState, PlayerQueueStatus,
    ProxyStateSnapshot, ServerPhase, TransferAckRequest,
};
use tokio::net::TcpListener;
use uuid::Uuid;

fn policy() -> FleetPolicy {
    FleetPolicy {
        game: GameKind::SheepWars,
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

fn tower_policy() -> FleetPolicy {
    FleetPolicy {
        game: GameKind::TowerWars,
        ..policy()
    }
}

fn player(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

async fn contract<R: QueueRepository>(repo: R) {
    assert!(repo.load_all().await.unwrap().is_empty());

    let a = repo
        .insert(NewQueueEntry {
            player_uuid: player(1),
            name: "Alice",
            joined_at: Utc::now(),
            seq: None,
            game: GameKind::SheepWars,
            match_size: 0,
        })
        .await
        .unwrap();
    let b = repo
        .insert(NewQueueEntry {
            player_uuid: player(2),
            name: "Bob",
            joined_at: Utc::now(),
            seq: None,
            game: GameKind::SheepWars,
            match_size: 0,
        })
        .await
        .unwrap();
    assert!(a >= FIRST_SEQ);
    assert!(b > a);

    let again = repo
        .insert(NewQueueEntry {
            player_uuid: player(1),
            name: "Alice2",
            joined_at: Utc::now(),
            seq: None,
            game: GameKind::SheepWars,
            match_size: 0,
        })
        .await
        .unwrap();
    assert_eq!(again, a);

    let head = repo
        .insert(NewQueueEntry {
            player_uuid: player(3),
            name: "Carol",
            joined_at: Utc::now(),
            seq: Some(a - 1),
            game: GameKind::SheepWars,
            match_size: 0,
        })
        .await
        .unwrap();
    assert_eq!(head, a - 1);

    let loaded = repo.load_all().await.unwrap();
    let order: Vec<(Uuid, &str)> = loaded
        .iter()
        .map(|r| (r.player_uuid, r.name.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (player(3), "Carol"),
            (player(1), "Alice2"),
            (player(2), "Bob")
        ]
    );
    assert!(loaded.iter().all(|r| r.state == QueueEntryState::Waiting));

    let transfer_id = Uuid::from_u128(77);
    let dispatched = QueueEntryState::Dispatched {
        server_id: "sheepwars-1".to_owned(),
        transfer_id,
    };
    repo.set_state(player(1), &dispatched).await.unwrap();
    let loaded = repo.load_all().await.unwrap();
    assert_eq!(loaded[1].state, dispatched);
    assert_eq!(loaded[1].seq, a);

    repo.set_state(player(1), &QueueEntryState::Waiting)
        .await
        .unwrap();
    assert_eq!(
        repo.load_all().await.unwrap()[1].state,
        QueueEntryState::Waiting
    );

    repo.set_state(player(99), &QueueEntryState::Waiting)
        .await
        .unwrap();
    repo.remove(player(2)).await.unwrap();
    repo.remove(player(99)).await.unwrap();
    let left: Vec<Uuid> = repo
        .load_all()
        .await
        .unwrap()
        .iter()
        .map(|r| r.player_uuid)
        .collect();
    assert_eq!(left, [player(3), player(1)]);
}

async fn boot_games(store: &QueueStore) -> String {
    let games = restore_games(policy(), vec![tower_policy()], store, Instant::now())
        .await
        .unwrap();
    let state = AppState::from_games(games, None).with_queue(store.clone());
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn join_game(base: &str, n: u128, game: GameKind) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/queue/join"))
        .json(&JoinQueueRequest {
            player_uuid: player(n),
            player_name: format!("p{n}"),
            game,
            match_size: 0,
        })
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

async fn status_game(base: &str, n: u128, game: &str) -> PlayerQueueStatus {
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

async fn game_column_round_trip<R: QueueRepository>(repo: R) {
    for (n, game) in [
        (1, GameKind::SheepWars),
        (2, GameKind::TowerWars),
        (3, GameKind::TowerWars),
    ] {
        repo.insert(NewQueueEntry {
            player_uuid: player(n),
            name: "p",
            joined_at: Utc::now(),
            seq: None,
            game,
            match_size: 0,
        })
        .await
        .unwrap();
    }
    let loaded = repo.load_all().await.unwrap();
    let games: Vec<(Uuid, GameKind)> = loaded.iter().map(|r| (r.player_uuid, r.game)).collect();
    assert_eq!(
        games,
        [
            (player(1), GameKind::SheepWars),
            (player(2), GameKind::TowerWars),
            (player(3), GameKind::TowerWars)
        ]
    );
    repo.insert(NewQueueEntry {
        player_uuid: player(2),
        name: "renamed",
        joined_at: Utc::now(),
        seq: None,
        game: GameKind::TowerWars,
        match_size: 0,
    })
    .await
    .unwrap();
    let again = repo.load_all().await.unwrap();
    assert_eq!(again[1].game, GameKind::TowerWars);
    assert_eq!(again[1].name, "renamed");
}

async fn match_size_round_trip<R: QueueRepository>(repo: R) {
    for (n, game, match_size) in [
        (1, GameKind::SheepWars, 0),
        (2, GameKind::TowerWars, 1),
        (3, GameKind::TowerWars, 4),
    ] {
        repo.insert(NewQueueEntry {
            player_uuid: player(n),
            name: "p",
            joined_at: Utc::now(),
            seq: None,
            game,
            match_size,
        })
        .await
        .unwrap();
    }
    let sizes: Vec<(Uuid, u8)> = repo
        .load_all()
        .await
        .unwrap()
        .iter()
        .map(|r| (r.player_uuid, r.match_size))
        .collect();
    assert_eq!(sizes, [(player(1), 0), (player(2), 1), (player(3), 4)]);
}

async fn spec_tw_26_each_game_queue_survives_a_restart(store: QueueStore) {
    let first = boot_games(&store).await;
    join_game(&first, 1, GameKind::SheepWars).await;
    join_game(&first, 2, GameKind::TowerWars).await;
    join_game(&first, 3, GameKind::TowerWars).await;
    join_game(&first, 2, GameKind::SheepWars).await;

    let second = boot_games(&store).await;
    let sheep = status_game(&second, 1, "sheep_wars").await;
    assert_eq!(
        (sheep.state, sheep.position),
        (PlayerQueueState::Queued, Some(1))
    );
    assert_eq!(sheep.total_queued, 1);
    let tower_two = status_game(&second, 2, "tower_wars").await;
    let tower_three = status_game(&second, 3, "tower_wars").await;
    assert_eq!(tower_two.position, Some(1));
    assert_eq!(tower_three.position, Some(2));
    assert_eq!(tower_three.total_queued, 2);

    let sheep_only = boot(&store).await;
    assert_eq!(
        status(&sheep_only, 2).await.state,
        PlayerQueueState::NotQueued
    );
    assert_eq!(status(&sheep_only, 1).await.position, Some(1));
}

async fn match_size_survives_a_restart(store: QueueStore) {
    let first = boot_games(&store).await;
    for (n, size) in [(1, 0), (2, 3), (3, 9)] {
        let resp = reqwest::Client::new()
            .post(format!("{first}/api/v1/queue/join"))
            .json(&JoinQueueRequest {
                player_uuid: player(n),
                player_name: format!("p{n}"),
                game: GameKind::TowerWars,
                match_size: size,
            })
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
    }
    let sizes: Vec<u8> = store
        .load_all()
        .await
        .unwrap()
        .iter()
        .map(|r| r.match_size)
        .collect();
    assert_eq!(sizes, [1, 3, 1]);

    let second = boot_games(&store).await;
    assert_eq!(
        reqwest::get(format!(
            "{second}/api/v1/queue/player/{}?game=tower_wars",
            player(2)
        ))
        .await
        .unwrap()
        .json::<PlayerQueueStatus>()
        .await
        .unwrap()
        .min_players,
        3
    );
}

async fn boot(store: &QueueStore) -> String {
    let engine = restore_engine(policy(), store, Instant::now())
        .await
        .unwrap();
    let state = AppState::new(engine, None).with_queue(store.clone());
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn join(base: &str, n: u128) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/queue/join"))
        .json(&JoinQueueRequest {
            player_uuid: player(n),
            player_name: format!("p{n}"),
            game: GameKind::SheepWars,
            match_size: 0,
        })
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

async fn status(base: &str, n: u128) -> PlayerQueueStatus {
    reqwest::get(format!("{base}/api/v1/queue/player/{}", player(n)))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn heartbeat(base: &str, server: &str, players: &[u128]) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/servers/heartbeat"))
        .json(&HeartbeatRequest {
            server_id: server.to_owned(),
            address: Some("10.0.0.5:25565".to_owned()),
            phase: ServerPhase::Waiting,
            player_count: players.len() as u32,
            max_players: 8,
            player_uuids: players.iter().map(|n| player(*n)).collect(),
            version: None,
            kind: Default::default(),
        })
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

async fn plc_13_queue_order_survives_a_restart(store: QueueStore) {
    let first = boot(&store).await;
    for n in 1..=5 {
        join(&first, n).await;
    }
    let second = boot(&store).await;
    for n in 1..=5u128 {
        let status = status(&second, n).await;
        assert_eq!(status.state, PlayerQueueState::Queued);
        assert_eq!(status.position, Some(n as usize));
        assert_eq!(status.total_queued, 5);
    }
    let third = boot(&store).await;
    join(&third, 6).await;
    assert_eq!(status(&third, 6).await.position, Some(6));
    let loaded = store.load_all().await.unwrap();
    let seqs: Vec<u64> = loaded.iter().map(|r| r.seq).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
}

async fn leave_and_disconnect_are_persisted(store: QueueStore) {
    let base = boot(&store).await;
    for n in 1..=3 {
        join(&base, n).await;
    }
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/api/v1/queue/leave"))
        .json(&serde_json::json!({ "player_uuid": player(1), "reason": "user_toggle" }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/api/v1/proxy/player-disconnected"))
        .json(&serde_json::json!({ "player_uuid": player(2) }))
        .send()
        .await
        .unwrap();
    let rebooted = boot(&store).await;
    assert_eq!(
        status(&rebooted, 1).await.state,
        PlayerQueueState::NotQueued
    );
    assert_eq!(
        status(&rebooted, 2).await.state,
        PlayerQueueState::NotQueued
    );
    assert_eq!(status(&rebooted, 3).await.position, Some(1));
}

async fn plc_14_and_15_dispatched_entries_survive_and_fail_back(store: QueueStore) {
    let first = boot(&store).await;
    heartbeat(&first, "sheepwars-1", &[]).await;
    join(&first, 1).await;
    join(&first, 2).await;
    join(&first, 3).await;
    assert_eq!(
        status(&first, 1).await.state,
        PlayerQueueState::Transferring
    );

    let second = boot(&store).await;
    let after = status(&second, 1).await;
    assert_eq!(after.state, PlayerQueueState::Transferring);
    assert_eq!(after.target_server.as_deref(), Some("sheepwars-1"));
    assert_eq!(
        status(&second, 2).await.state,
        PlayerQueueState::Transferring
    );

    heartbeat(&second, "sheepwars-1", &[1]).await;
    assert_eq!(status(&second, 1).await.state, PlayerQueueState::NotQueued);

    let third = boot(&store).await;
    assert_eq!(status(&third, 1).await.state, PlayerQueueState::NotQueued);
    assert_eq!(
        status(&third, 2).await.state,
        PlayerQueueState::Transferring
    );

    let proxy: ProxyStateSnapshot = reqwest::get(format!("{third}/api/v1/proxy/state"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let transfer = proxy
        .pending_transfers
        .iter()
        .find(|t| t.player_uuid == player(2))
        .unwrap();
    reqwest::Client::new()
        .post(format!(
            "{third}/api/v1/proxy/transfers/{}/ack",
            transfer.transfer_id
        ))
        .json(&TransferAckRequest {
            success: false,
            message: None,
        })
        .send()
        .await
        .unwrap();

    let records = store.load_all().await.unwrap();
    let waiting: Vec<(Uuid, &QueueEntryState)> =
        records.iter().map(|r| (r.player_uuid, &r.state)).collect();
    assert_eq!(waiting.len(), 2);
    assert_eq!(waiting[0].0, player(2));
    assert_eq!(waiting[0].1, &QueueEntryState::Waiting);
    assert_eq!(waiting[1].0, player(3));
}

#[tokio::test]
async fn memory_passes_the_contract() {
    contract(MemoryQueueRepository::default()).await;
}

#[tokio::test]
async fn memory_plc_13_queue_order_survives_a_restart() {
    plc_13_queue_order_survives_a_restart(QueueStore::default()).await;
}

#[tokio::test]
async fn memory_persists_leave_and_proxy_disconnect() {
    leave_and_disconnect_are_persisted(QueueStore::default()).await;
}

#[tokio::test]
async fn memory_plc_14_and_15_dispatched_entries_survive_and_fail_back() {
    plc_14_and_15_dispatched_entries_survive_and_fail_back(QueueStore::default()).await;
}

#[tokio::test]
async fn memory_game_column_round_trip() {
    game_column_round_trip(MemoryQueueRepository::default()).await;
}

#[tokio::test]
async fn memory_match_size_round_trip() {
    match_size_round_trip(MemoryQueueRepository::default()).await;
}

#[tokio::test]
async fn memory_match_size_chosen_at_join_survives_a_restart() {
    match_size_survives_a_restart(QueueStore::default()).await;
}

#[tokio::test]
async fn memory_spec_tw_26_each_game_queue_survives_a_restart() {
    spec_tw_26_each_game_queue_survives_a_restart(QueueStore::default()).await;
}

#[cfg(feature = "postgres-tests")]
mod postgres {
    use ferriscraft_orchestrator::queue::PgQueueRepository;
    use ferriscraft_progression_pg::PgProgressionRepository;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::ContainerAsync;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    async fn start() -> (ContainerAsync<Postgres>, PgQueueRepository) {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        PgProgressionRepository::connect(&url)
            .await
            .unwrap()
            .migrate()
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let repo = PgQueueRepository::from_pool(pool);
        (container, repo)
    }

    #[tokio::test]
    async fn postgres_game_column_round_trip() {
        let (_container, repo) = start().await;
        game_column_round_trip(repo).await;
    }

    #[tokio::test]
    async fn postgres_match_size_round_trip() {
        let (_container, repo) = start().await;
        match_size_round_trip(repo).await;
    }

    #[tokio::test]
    async fn postgres_match_size_chosen_at_join_survives_a_restart() {
        let (_container, repo) = start().await;
        match_size_survives_a_restart(QueueStore::Postgres(repo)).await;
    }

    #[tokio::test]
    async fn postgres_migration_is_idempotent_and_old_rows_load_with_size_zero() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let progression = PgProgressionRepository::connect(&url).await.unwrap();
        progression.migrate().await.unwrap();
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query(
            "INSERT INTO queue_entries (player_uuid, name, game) VALUES ($1, 'old', 'towerwars')",
        )
        .bind(player(1))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../ferriscraft-progression-pg/migrations/0004_queue_match_size.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        progression.migrate().await.unwrap();
        let loaded = PgQueueRepository::from_pool(pool).load_all().await.unwrap();
        assert_eq!(loaded[0].match_size, 0);
    }

    #[tokio::test]
    async fn postgres_spec_tw_26_each_game_queue_survives_a_restart() {
        let (_container, repo) = start().await;
        spec_tw_26_each_game_queue_survives_a_restart(QueueStore::Postgres(repo)).await;
    }

    #[tokio::test]
    async fn postgres_rows_written_without_a_game_load_as_sheepwars() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        PgProgressionRepository::connect(&url)
            .await
            .unwrap()
            .migrate()
            .await
            .unwrap();
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("INSERT INTO queue_entries (player_uuid, name) VALUES ($1, 'legacy')")
            .bind(player(1))
            .execute(&pool)
            .await
            .unwrap();
        let repo = PgQueueRepository::from_pool(pool);
        let loaded = repo.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].game, GameKind::SheepWars);
    }

    #[tokio::test]
    async fn postgres_passes_the_contract() {
        let (_container, repo) = start().await;
        contract(repo).await;
    }

    #[tokio::test]
    async fn postgres_plc_13_queue_order_survives_a_restart() {
        let (_container, repo) = start().await;
        plc_13_queue_order_survives_a_restart(QueueStore::Postgres(repo)).await;
    }

    #[tokio::test]
    async fn postgres_persists_leave_and_proxy_disconnect() {
        let (_container, repo) = start().await;
        leave_and_disconnect_are_persisted(QueueStore::Postgres(repo)).await;
    }

    #[tokio::test]
    async fn postgres_plc_14_and_15_dispatched_entries_survive_and_fail_back() {
        let (_container, repo) = start().await;
        plc_14_and_15_dispatched_entries_survive_and_fail_back(QueueStore::Postgres(repo)).await;
    }
}
