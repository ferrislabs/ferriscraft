use std::collections::BTreeMap;
use std::future::Future;

use ferriscraft_progression::{
    CoinMovement, MatchToSettle, MemoryProgressionRepository, PlayerProgress, PlayerRecord,
    ProgressionRepository, ProgressionStoreError, Settlement,
};
use ferriscraft_progression_pg::PgProgressionRepository;
use ferriscraft_protocol::MatchReportAck;
use uuid::Uuid;

#[derive(Clone)]
pub enum ProgressionStore {
    Memory(MemoryProgressionRepository),
    Postgres(PgProgressionRepository),
}

impl ProgressionRepository for ProgressionStore {
    fn load(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Option<PlayerRecord>, ProgressionStoreError>> + Send {
        async move {
            match self {
                Self::Memory(repo) => repo.load(uuid).await,
                Self::Postgres(repo) => repo.load(uuid).await,
            }
        }
    }

    fn update<T, E, F>(
        &self,
        uuid: Uuid,
        apply: F,
    ) -> impl Future<Output = Result<Result<(PlayerRecord, T), E>, ProgressionStoreError>> + Send
    where
        T: Send,
        E: Send,
        F: FnOnce(&mut PlayerProgress) -> Result<(T, Vec<CoinMovement>), E> + Send,
    {
        async move {
            match self {
                Self::Memory(repo) => repo.update(uuid, apply).await,
                Self::Postgres(repo) => repo.update(uuid, apply).await,
            }
        }
    }

    fn settle_match<F>(
        &self,
        settle: MatchToSettle<'_>,
        apply: F,
    ) -> impl Future<Output = Result<MatchReportAck, ProgressionStoreError>> + Send
    where
        F: FnOnce(&mut BTreeMap<Uuid, PlayerProgress>) -> Settlement + Send,
    {
        async move {
            match self {
                Self::Memory(repo) => repo.settle_match(settle, apply).await,
                Self::Postgres(repo) => repo.settle_match(settle, apply).await,
            }
        }
    }

    fn ledger(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Vec<CoinMovement>, ProgressionStoreError>> + Send {
        async move {
            match self {
                Self::Memory(repo) => repo.ledger(uuid).await,
                Self::Postgres(repo) => repo.ledger(uuid).await,
            }
        }
    }
}
