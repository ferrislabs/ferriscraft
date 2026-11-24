use std::time::Duration;

use ferriscraft_protocol::GameKind;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{NewQueueEntry, QueueEntryState, QueueRecord, QueueRepository, QueueStoreError};

#[derive(Debug, Clone)]
pub struct PgQueueRepository {
    pool: PgPool,
}

fn unavailable(err: sqlx::Error) -> QueueStoreError {
    QueueStoreError::Unavailable(err.to_string())
}

fn corrupted(what: &str, detail: impl std::fmt::Display) -> QueueStoreError {
    QueueStoreError::Corrupted(format!("{what} : {detail}"))
}

fn record(row: &PgRow) -> Result<QueueRecord, QueueStoreError> {
    let seq: i64 = row.try_get("seq").map_err(unavailable)?;
    let state: String = row.try_get("state").map_err(unavailable)?;
    let server_id: Option<String> = row.try_get("server_id").map_err(unavailable)?;
    let transfer_id: Option<Uuid> = row.try_get("transfer_id").map_err(unavailable)?;
    let state = match (state.as_str(), server_id, transfer_id) {
        ("waiting", _, _) => QueueEntryState::Waiting,
        ("dispatched", Some(server_id), Some(transfer_id)) => QueueEntryState::Dispatched {
            server_id,
            transfer_id,
        },
        (other, _, _) => return Err(corrupted("état illisible", other)),
    };
    let game: String = row.try_get("game").map_err(unavailable)?;
    let game = GameKind::from_slug(&game).ok_or_else(|| corrupted("jeu illisible", &game))?;
    let match_size: i16 = row.try_get("match_size").map_err(unavailable)?;
    Ok(QueueRecord {
        seq: u64::try_from(seq).map_err(|_| corrupted("seq négatif", seq))?,
        player_uuid: row.try_get("player_uuid").map_err(unavailable)?,
        name: row.try_get("name").map_err(unavailable)?,
        joined_at: row.try_get("joined_at").map_err(unavailable)?,
        state,
        game,
        match_size: u8::try_from(match_size)
            .map_err(|_| corrupted("match_size illisible", match_size))?,
    })
}

impl PgQueueRepository {
    pub async fn connect(url: &str) -> Result<Self, QueueStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(unavailable)?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl QueueRepository for PgQueueRepository {
    async fn load_all(&self) -> Result<Vec<QueueRecord>, QueueStoreError> {
        let rows = sqlx::query(
            "SELECT seq, player_uuid, name, joined_at, state, server_id, transfer_id, game, match_size \
             FROM queue_entries ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(unavailable)?;
        rows.iter().map(record).collect()
    }

    async fn insert(&self, entry: NewQueueEntry<'_>) -> Result<u64, QueueStoreError> {
        let explicit = entry
            .seq
            .map(|seq| i64::try_from(seq).map_err(|_| corrupted("seq trop grand", seq)))
            .transpose()?;
        let row = sqlx::query(
            "INSERT INTO queue_entries (seq, player_uuid, name, joined_at, game, match_size) \
             VALUES (COALESCE($1, nextval('queue_entries_seq_seq')), $2, $3, $4, $5, $6) \
             ON CONFLICT (player_uuid) DO UPDATE SET name = EXCLUDED.name \
             RETURNING seq",
        )
        .bind(explicit)
        .bind(entry.player_uuid)
        .bind(entry.name)
        .bind(entry.joined_at)
        .bind(entry.game.slug())
        .bind(i16::from(entry.match_size))
        .fetch_one(&self.pool)
        .await
        .map_err(unavailable)?;
        let seq: i64 = row.try_get("seq").map_err(unavailable)?;
        u64::try_from(seq).map_err(|_| corrupted("seq négatif", seq))
    }

    async fn set_state(
        &self,
        player_uuid: Uuid,
        state: &QueueEntryState,
    ) -> Result<(), QueueStoreError> {
        let (name, server_id, transfer_id) = match state {
            QueueEntryState::Waiting => ("waiting", None, None),
            QueueEntryState::Dispatched {
                server_id,
                transfer_id,
            } => ("dispatched", Some(server_id.as_str()), Some(*transfer_id)),
        };
        sqlx::query(
            "UPDATE queue_entries SET state = $2, server_id = $3, transfer_id = $4 \
             WHERE player_uuid = $1",
        )
        .bind(player_uuid)
        .bind(name)
        .bind(server_id)
        .bind(transfer_id)
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;
        Ok(())
    }

    async fn remove(&self, player_uuid: Uuid) -> Result<(), QueueStoreError> {
        sqlx::query("DELETE FROM queue_entries WHERE player_uuid = $1")
            .bind(player_uuid)
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}
