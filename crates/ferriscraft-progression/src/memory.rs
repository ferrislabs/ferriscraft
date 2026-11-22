use std::collections::BTreeMap;
use std::future::Future;

use crate::{CoinMovement, PlayerProgress, PlayerRecord};
use ferriscraft_protocol::MatchReportAck;
use uuid::Uuid;

use crate::repository::{MatchToSettle, ProgressionRepository, ProgressionStoreError, Settlement};

#[derive(Debug, Clone, Default)]
pub struct MemoryProgressionRepository;

impl ProgressionRepository for MemoryProgressionRepository {
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
