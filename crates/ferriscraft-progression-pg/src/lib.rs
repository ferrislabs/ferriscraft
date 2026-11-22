use std::collections::BTreeMap;
use std::future::Future;

use ferriscraft_progression::{CoinMovement, PlayerProgress, PlayerRecord};
use ferriscraft_protocol::MatchReportAck;
use uuid::Uuid;

use ferriscraft_progression::{
    MatchToSettle, ProgressionRepository, ProgressionStoreError, Settlement,
};

#[derive(Debug, Clone)]
pub struct PgProgressionRepository {
    pool: sqlx::PgPool,
}

impl PgProgressionRepository {
    pub async fn connect(_url: &str) -> Result<Self, ProgressionStoreError> {
        todo!()
    }

    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), ProgressionStoreError> {
        let _ = &self.pool;
        todo!()
    }
}

impl ProgressionRepository for PgProgressionRepository {
    fn load(
        &self,
        _uuid: Uuid,
    ) -> impl Future<Output = Result<Option<PlayerRecord>, ProgressionStoreError>> + Send {
        async { todo!() }
    }

    fn update<T, E, F>(
        &self,
        _uuid: Uuid,
        _apply: F,
    ) -> impl Future<Output = Result<Result<(PlayerRecord, T), E>, ProgressionStoreError>> + Send
    where
        T: Send,
        E: Send,
        F: FnOnce(&mut PlayerProgress) -> Result<(T, Vec<CoinMovement>), E> + Send,
    {
        async { todo!() }
    }

    fn settle_match<F>(
        &self,
        _settle: MatchToSettle<'_>,
        _apply: F,
    ) -> impl Future<Output = Result<MatchReportAck, ProgressionStoreError>> + Send
    where
        F: FnOnce(&mut BTreeMap<Uuid, PlayerProgress>) -> Settlement + Send,
    {
        async { todo!() }
    }

    fn ledger(
        &self,
        _uuid: Uuid,
    ) -> impl Future<Output = Result<Vec<CoinMovement>, ProgressionStoreError>> + Send {
        async { todo!() }
    }
}
