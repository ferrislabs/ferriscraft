use uuid::Uuid;

use super::{
    MemoryQueueRepository, NewQueueEntry, PgQueueRepository, QueueEntryState, QueueRecord,
    QueueRepository, QueueStoreError,
};

#[derive(Debug, Clone)]
pub enum QueueStore {
    Memory(MemoryQueueRepository),
    Postgres(PgQueueRepository),
}

impl Default for QueueStore {
    fn default() -> Self {
        Self::Memory(MemoryQueueRepository::default())
    }
}

impl QueueRepository for QueueStore {
    async fn load_all(&self) -> Result<Vec<QueueRecord>, QueueStoreError> {
        match self {
            Self::Memory(repo) => repo.load_all().await,
            Self::Postgres(repo) => repo.load_all().await,
        }
    }

    async fn insert(&self, entry: NewQueueEntry<'_>) -> Result<u64, QueueStoreError> {
        match self {
            Self::Memory(repo) => repo.insert(entry).await,
            Self::Postgres(repo) => repo.insert(entry).await,
        }
    }

    async fn set_state(
        &self,
        player_uuid: Uuid,
        state: &QueueEntryState,
    ) -> Result<(), QueueStoreError> {
        match self {
            Self::Memory(repo) => repo.set_state(player_uuid, state).await,
            Self::Postgres(repo) => repo.set_state(player_uuid, state).await,
        }
    }

    async fn remove(&self, player_uuid: Uuid) -> Result<(), QueueStoreError> {
        match self {
            Self::Memory(repo) => repo.remove(player_uuid).await,
            Self::Postgres(repo) => repo.remove(player_uuid).await,
        }
    }
}
