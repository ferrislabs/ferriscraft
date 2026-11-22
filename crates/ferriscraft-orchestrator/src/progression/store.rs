use std::collections::BTreeMap;

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
    async fn load(&self, uuid: Uuid) -> Result<Option<PlayerRecord>, ProgressionStoreError> {
        match self {
            Self::Memory(repo) => repo.load(uuid).await,
            Self::Postgres(repo) => repo.load(uuid).await,
        }
    }

    async fn update<T, E, F>(
        &self,
        uuid: Uuid,
        apply: F,
    ) -> Result<Result<(PlayerRecord, T), E>, ProgressionStoreError>
    where
        T: Send,
        E: Send,
        F: FnOnce(&mut PlayerProgress) -> Result<(T, Vec<CoinMovement>), E> + Send,
    {
        match self {
            Self::Memory(repo) => repo.update(uuid, apply).await,
            Self::Postgres(repo) => repo.update(uuid, apply).await,
        }
    }

    async fn settle_match<F>(
        &self,
        settle: MatchToSettle<'_>,
        apply: F,
    ) -> Result<MatchReportAck, ProgressionStoreError>
    where
        F: FnOnce(&mut BTreeMap<Uuid, PlayerProgress>) -> Settlement + Send,
    {
        match self {
            Self::Memory(repo) => repo.settle_match(settle, apply).await,
            Self::Postgres(repo) => repo.settle_match(settle, apply).await,
        }
    }

    async fn ledger(&self, uuid: Uuid) -> Result<Vec<CoinMovement>, ProgressionStoreError> {
        match self {
            Self::Memory(repo) => repo.ledger(uuid).await,
            Self::Postgres(repo) => repo.ledger(uuid).await,
        }
    }
}
