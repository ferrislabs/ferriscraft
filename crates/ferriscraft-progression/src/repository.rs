use std::collections::BTreeMap;
use std::future::Future;

use crate::{CoinMovement, PlayerProgress, PlayerRecord};
use ferriscraft_protocol::{GameKind, MatchReportAck};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgressionStoreError {
    #[error("stockage de progression indisponible : {0}")]
    Unavailable(String),
    #[error("conflit d'écriture sur la progression")]
    Conflict,
    #[error("données de progression corrompues : {0}")]
    Corrupted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub uuid: Uuid,
    pub movement: CoinMovement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchToSettle<'a> {
    pub match_id: Uuid,
    pub game: GameKind,
    pub server_id: &'a str,
    pub players: &'a [Uuid],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub ack: MatchReportAck,
    pub ledger: Vec<LedgerEntry>,
}

pub trait ProgressionRepository: Clone + Send + Sync + 'static {
    fn load(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Option<PlayerRecord>, ProgressionStoreError>> + Send;

    fn update<T, E, F>(
        &self,
        uuid: Uuid,
        apply: F,
    ) -> impl Future<Output = Result<Result<(PlayerRecord, T), E>, ProgressionStoreError>> + Send
    where
        T: Send,
        E: Send,
        F: FnOnce(&mut PlayerProgress) -> Result<(T, Vec<CoinMovement>), E> + Send;

    fn settle_match<F>(
        &self,
        settle: MatchToSettle<'_>,
        apply: F,
    ) -> impl Future<Output = Result<MatchReportAck, ProgressionStoreError>> + Send
    where
        F: FnOnce(&mut BTreeMap<Uuid, PlayerProgress>) -> Settlement + Send;

    fn ledger(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Vec<CoinMovement>, ProgressionStoreError>> + Send;
}
