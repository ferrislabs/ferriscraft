mod memory;
mod postgres;
mod store;

use std::future::Future;
use std::time::Instant;

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{FleetEngine, FleetPolicy};

pub use memory::MemoryQueueRepository;
pub use postgres::PgQueueRepository;
pub use store::QueueStore;

pub const FIRST_SEQ: u64 = 1 << 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEntryState {
    Waiting,
    Dispatched {
        server_id: String,
        transfer_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRecord {
    pub seq: u64,
    pub player_uuid: Uuid,
    pub name: String,
    pub joined_at: DateTime<Utc>,
    pub state: QueueEntryState,
}

#[derive(Debug, Clone, Copy)]
pub struct NewQueueEntry<'a> {
    pub player_uuid: Uuid,
    pub name: &'a str,
    pub joined_at: DateTime<Utc>,
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueMutation {
    Insert {
        seq: u64,
        player_uuid: Uuid,
        name: String,
    },
    SetState {
        player_uuid: Uuid,
        state: QueueEntryState,
    },
    Remove {
        player_uuid: Uuid,
    },
}

#[derive(Debug, Error)]
pub enum QueueStoreError {
    #[error("file d'attente indisponible : {0}")]
    Unavailable(String),
    #[error("file d'attente corrompue : {0}")]
    Corrupted(String),
}

pub trait QueueRepository {
    fn load_all(&self) -> impl Future<Output = Result<Vec<QueueRecord>, QueueStoreError>> + Send;

    fn insert(
        &self,
        entry: NewQueueEntry<'_>,
    ) -> impl Future<Output = Result<u64, QueueStoreError>> + Send;

    fn set_state(
        &self,
        player_uuid: Uuid,
        state: &QueueEntryState,
    ) -> impl Future<Output = Result<(), QueueStoreError>> + Send;

    fn remove(&self, player_uuid: Uuid)
    -> impl Future<Output = Result<(), QueueStoreError>> + Send;
}

pub async fn persist<R: QueueRepository>(
    repo: &R,
    mutation: &QueueMutation,
) -> Result<(), QueueStoreError> {
    match mutation {
        QueueMutation::Insert {
            seq,
            player_uuid,
            name,
        } => repo
            .insert(NewQueueEntry {
                player_uuid: *player_uuid,
                name,
                joined_at: Utc::now(),
                seq: Some(*seq),
            })
            .await
            .map(drop),
        QueueMutation::SetState { player_uuid, state } => repo.set_state(*player_uuid, state).await,
        QueueMutation::Remove { player_uuid } => repo.remove(*player_uuid).await,
    }
}

pub async fn restore_engine<R: QueueRepository>(
    policy: FleetPolicy,
    repo: &R,
    now: Instant,
) -> Result<FleetEngine, QueueStoreError> {
    let records = repo.load_all().await?;
    let mut engine = FleetEngine::new(policy);
    engine.restore_queue(now, records);
    Ok(engine)
}
