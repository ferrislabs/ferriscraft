use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use uuid::Uuid;

use super::{
    FIRST_SEQ, NewQueueEntry, QueueEntryState, QueueRecord, QueueRepository, QueueStoreError,
};

#[derive(Debug)]
struct Inner {
    next_seq: u64,
    entries: BTreeMap<Uuid, QueueRecord>,
}

#[derive(Debug, Clone)]
pub struct MemoryQueueRepository {
    inner: Arc<Mutex<Inner>>,
}

impl Default for MemoryQueueRepository {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_seq: FIRST_SEQ,
                entries: BTreeMap::new(),
            })),
        }
    }
}

impl MemoryQueueRepository {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl QueueRepository for MemoryQueueRepository {
    async fn load_all(&self) -> Result<Vec<QueueRecord>, QueueStoreError> {
        let mut records: Vec<QueueRecord> = self.lock().entries.values().cloned().collect();
        records.sort_by_key(|record| record.seq);
        Ok(records)
    }

    async fn insert(&self, entry: NewQueueEntry<'_>) -> Result<u64, QueueStoreError> {
        let mut inner = self.lock();
        if let Some(existing) = inner.entries.get_mut(&entry.player_uuid) {
            entry.name.clone_into(&mut existing.name);
            return Ok(existing.seq);
        }
        let seq = match entry.seq {
            Some(seq) => seq,
            None => {
                let seq = inner.next_seq;
                inner.next_seq += 1;
                seq
            }
        };
        if inner.entries.values().any(|record| record.seq == seq) {
            return Err(QueueStoreError::Corrupted(format!(
                "seq {seq} déjà utilisé"
            )));
        }
        inner.entries.insert(
            entry.player_uuid,
            QueueRecord {
                seq,
                player_uuid: entry.player_uuid,
                name: entry.name.to_owned(),
                joined_at: entry.joined_at,
                state: QueueEntryState::Waiting,
                game: entry.game,
                match_size: entry.match_size,
            },
        );
        Ok(seq)
    }

    async fn set_state(
        &self,
        player_uuid: Uuid,
        state: &QueueEntryState,
    ) -> Result<(), QueueStoreError> {
        if let Some(record) = self.lock().entries.get_mut(&player_uuid) {
            record.state = state.clone();
        }
        Ok(())
    }

    async fn remove(&self, player_uuid: Uuid) -> Result<(), QueueStoreError> {
        self.lock().entries.remove(&player_uuid);
        Ok(())
    }
}
