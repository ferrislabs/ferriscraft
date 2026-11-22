#![cfg(feature = "postgres-tests")]

mod common;

use ferriscraft_progression::{Coins, ProgressionRepository, ProgressionStoreError, UnlockError};
use ferriscraft_progression_pg::PgProgressionRepository;
use ferriscraft_protocol::CosmeticId;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

async fn start() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, url)
}

async fn repository(url: &str) -> PgProgressionRepository {
    let repo = PgProgressionRepository::connect(url).await.unwrap();
    repo.migrate().await.unwrap();
    repo
}

#[tokio::test]
async fn postgres_passes_the_contract() {
    let (_container, url) = start().await;
    common::contract(repository(&url).await).await;
}

#[tokio::test]
async fn migrate_is_idempotent_and_balance_check_rejects_negatives() {
    let (_container, url) = start().await;
    let repo = repository(&url).await;
    repo.migrate().await.unwrap();

    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let err = sqlx::query("INSERT INTO coin_balance (uuid, coins) VALUES ($1, -1)")
        .bind(Uuid::from_u128(1))
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error()
            .and_then(|db| db.code().map(|c| c.into_owned())),
        Some("23514".to_owned())
    );
}

#[tokio::test]
async fn unreachable_database_is_unavailable() {
    let err = PgProgressionRepository::connect("postgres://postgres:x@127.0.0.1:1/postgres")
        .await
        .unwrap_err();
    assert!(matches!(err, ProgressionStoreError::Unavailable(_)));
}

#[tokio::test]
async fn spec_prg_9_concurrent_purchases_never_overdraw() {
    let (_container, url) = start().await;
    let repo = repository(&url).await;
    let player = common::ALEX;
    repo.update(player, |progress| {
        let movement = progress.grant(Coins(600), "console");
        Ok::<_, UnlockError>(((), vec![movement]))
    })
    .await
    .unwrap()
    .unwrap();

    let buy = |id: CosmeticId| {
        let repo = repo.clone();
        async move {
            repo.update(player, move |progress| {
                let movement = progress.purchase(id)?;
                Ok::<_, UnlockError>(((), vec![movement]))
            })
            .await
            .unwrap()
        }
    };
    let (first, second) = tokio::join!(
        buy(CosmeticId::GadgetWindCharge),
        buy(CosmeticId::SheepTrailFlame)
    );

    let successes = [&first, &second].iter().filter(|r| r.is_ok()).count();
    assert_eq!(successes, 1);
    let failure = [first, second].into_iter().find_map(Result::err).unwrap();
    assert!(matches!(failure, UnlockError::InsufficientCoins { .. }));
    let record = repo.load(player).await.unwrap().unwrap();
    assert_eq!(record.coins, Coins(100));
    assert_eq!(record.purchased.len(), 1);
}

#[tokio::test]
async fn spec_prg_15_progression_survives_reconnect() {
    let (_container, url) = start().await;
    let repo = repository(&url).await;
    let (written, ()) = repo
        .update(common::ALEX, |progress| {
            progress.set_identity("Alex", ferriscraft_protocol::PlayerRank::Vip);
            let movement = progress.grant(Coins(800), "console");
            Ok::<_, UnlockError>(((), vec![movement]))
        })
        .await
        .unwrap()
        .unwrap();
    drop(repo);

    let reconnected = PgProgressionRepository::connect(&url).await.unwrap();
    assert_eq!(reconnected.load(common::ALEX).await.unwrap(), Some(written));
}
