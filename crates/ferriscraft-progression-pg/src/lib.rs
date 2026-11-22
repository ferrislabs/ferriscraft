use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use ferriscraft_progression::{
    CoinDelta, CoinMovement, CoinReason, Coins, GameRecord, MatchToSettle, PlayerProgress,
    PlayerRecord, ProgressionRepository, ProgressionStoreError, Settlement, Xp,
};
use ferriscraft_protocol::{CosmeticId, CosmeticSlot, GameKind, MatchReportAck, PlayerRank};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PgProgressionRepository {
    pool: sqlx::PgPool,
}

fn store(err: sqlx::Error) -> ProgressionStoreError {
    if let Some(db) = err.as_database_error()
        && matches!(db.code().as_deref(), Some("23514" | "40001"))
    {
        return ProgressionStoreError::Conflict;
    }
    ProgressionStoreError::Unavailable(err.to_string())
}

fn corrupted(what: &str, detail: impl std::fmt::Display) -> ProgressionStoreError {
    ProgressionStoreError::Corrupted(format!("{what} : {detail}"))
}

fn encode<T: Serialize>(value: &T) -> Result<String, ProgressionStoreError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => Ok(text),
        Ok(other) => Err(corrupted("valeur non textuelle", other)),
        Err(err) => Err(corrupted("encodage impossible", err)),
    }
}

fn decode<T: DeserializeOwned>(text: &str) -> Result<T, ProgressionStoreError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|err| corrupted(&format!("valeur illisible `{text}`"), err))
}

fn to_i64(value: u64, what: &str) -> Result<i64, ProgressionStoreError> {
    i64::try_from(value).map_err(|_| corrupted(what, "valeur supérieure à i64::MAX"))
}

fn to_i32(value: u32, what: &str) -> Result<i32, ProgressionStoreError> {
    i32::try_from(value).map_err(|_| corrupted(what, "valeur supérieure à i32::MAX"))
}

fn from_i64(value: i64, what: &str) -> Result<u64, ProgressionStoreError> {
    u64::try_from(value).map_err(|_| corrupted(what, "valeur négative"))
}

fn from_i32(value: i32, what: &str) -> Result<u32, ProgressionStoreError> {
    u32::try_from(value).map_err(|_| corrupted(what, "valeur négative"))
}

async fn ensure_row(conn: &mut PgConnection, uuid: Uuid) -> Result<(), ProgressionStoreError> {
    sqlx::query(
        "INSERT INTO players (uuid, name, rank) VALUES ($1, '', 'guest') ON CONFLICT DO NOTHING",
    )
    .bind(uuid)
    .execute(conn)
    .await
    .map_err(store)?;
    Ok(())
}

async fn load_record(
    conn: &mut PgConnection,
    uuid: Uuid,
    lock: bool,
) -> Result<Option<PlayerRecord>, ProgressionStoreError> {
    let sql = if lock {
        "SELECT name, rank FROM players WHERE uuid = $1 FOR UPDATE"
    } else {
        "SELECT name, rank FROM players WHERE uuid = $1"
    };
    let Some(row) = sqlx::query(sql)
        .bind(uuid)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store)?
    else {
        return Ok(None);
    };
    let name: String = row.try_get("name").map_err(store)?;
    let rank: PlayerRank = decode(&row.try_get::<String, _>("rank").map_err(store)?)?;

    let coins = sqlx::query("SELECT coins FROM coin_balance WHERE uuid = $1")
        .bind(uuid)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store)?
        .map(|row| row.try_get::<i64, _>("coins").map_err(store))
        .transpose()?
        .map_or(Ok(0), |coins| from_i64(coins, "solde"))?;

    let mut games = BTreeMap::new();
    for row in sqlx::query(
        "SELECT game, xp, played, wins, losses, draws FROM game_progress WHERE uuid = $1",
    )
    .bind(uuid)
    .fetch_all(&mut *conn)
    .await
    .map_err(store)?
    {
        let game: GameKind = decode(&row.try_get::<String, _>("game").map_err(store)?)?;
        games.insert(
            game,
            GameRecord {
                xp: Xp(from_i64(row.try_get("xp").map_err(store)?, "xp")?),
                played: from_i32(row.try_get("played").map_err(store)?, "parties")?,
                wins: from_i32(row.try_get("wins").map_err(store)?, "victoires")?,
                losses: from_i32(row.try_get("losses").map_err(store)?, "défaites")?,
                draws: from_i32(row.try_get("draws").map_err(store)?, "égalités")?,
            },
        );
    }

    let mut purchased = BTreeSet::new();
    for row in sqlx::query("SELECT cosmetic FROM unlocks WHERE uuid = $1")
        .bind(uuid)
        .fetch_all(&mut *conn)
        .await
        .map_err(store)?
    {
        let id: CosmeticId = decode(&row.try_get::<String, _>("cosmetic").map_err(store)?)?;
        purchased.insert(id);
    }

    let mut equipped = BTreeMap::new();
    for row in sqlx::query("SELECT slot, cosmetic FROM equipped WHERE uuid = $1")
        .bind(uuid)
        .fetch_all(&mut *conn)
        .await
        .map_err(store)?
    {
        let slot: CosmeticSlot = decode(&row.try_get::<String, _>("slot").map_err(store)?)?;
        let id: CosmeticId = decode(&row.try_get::<String, _>("cosmetic").map_err(store)?)?;
        equipped.insert(slot, id);
    }

    Ok(Some(PlayerRecord {
        uuid,
        name,
        rank,
        coins: Coins(coins),
        games,
        purchased,
        equipped,
    }))
}

async fn persist(
    conn: &mut PgConnection,
    record: &PlayerRecord,
) -> Result<(), ProgressionStoreError> {
    let uuid = record.uuid;
    let balance = to_i64(record.coins.get(), "solde")?;

    sqlx::query("UPDATE players SET name = $2, rank = $3, rank_seen_at = now() WHERE uuid = $1")
        .bind(uuid)
        .bind(&record.name)
        .bind(encode(&record.rank)?)
        .execute(&mut *conn)
        .await
        .map_err(store)?;

    for (game, stats) in &record.games {
        sqlx::query(
            "INSERT INTO game_progress (uuid, game, xp, played, wins, losses, draws) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (uuid, game) DO UPDATE SET xp = EXCLUDED.xp, played = EXCLUDED.played, \
             wins = EXCLUDED.wins, losses = EXCLUDED.losses, draws = EXCLUDED.draws",
        )
        .bind(uuid)
        .bind(encode(game)?)
        .bind(to_i64(stats.xp.get(), "xp")?)
        .bind(to_i32(stats.played, "parties")?)
        .bind(to_i32(stats.wins, "victoires")?)
        .bind(to_i32(stats.losses, "défaites")?)
        .bind(to_i32(stats.draws, "égalités")?)
        .execute(&mut *conn)
        .await
        .map_err(store)?;
    }

    for id in &record.purchased {
        sqlx::query(
            "INSERT INTO unlocks (uuid, cosmetic, source) VALUES ($1, $2, 'purchase') \
             ON CONFLICT DO NOTHING",
        )
        .bind(uuid)
        .bind(encode(id)?)
        .execute(&mut *conn)
        .await
        .map_err(store)?;
    }

    sqlx::query("DELETE FROM equipped WHERE uuid = $1")
        .bind(uuid)
        .execute(&mut *conn)
        .await
        .map_err(store)?;
    for (slot, id) in &record.equipped {
        sqlx::query("INSERT INTO equipped (uuid, slot, cosmetic) VALUES ($1, $2, $3)")
            .bind(uuid)
            .bind(encode(slot)?)
            .bind(encode(id)?)
            .execute(&mut *conn)
            .await
            .map_err(store)?;
    }

    sqlx::query(
        "INSERT INTO coin_balance (uuid, coins) VALUES ($1, $2) \
         ON CONFLICT (uuid) DO UPDATE SET coins = EXCLUDED.coins",
    )
    .bind(uuid)
    .bind(balance)
    .execute(&mut *conn)
    .await
    .map_err(store)?;
    Ok(())
}

async fn insert_movement(
    conn: &mut PgConnection,
    uuid: Uuid,
    movement: &CoinMovement,
) -> Result<(), ProgressionStoreError> {
    let delta = match movement.delta {
        CoinDelta::Credit(coins) => to_i64(coins.get(), "mouvement")?,
        CoinDelta::Debit(coins) => -to_i64(coins.get(), "mouvement")?,
    };
    let (reason, match_id, cosmetic, admin) = match &movement.reason {
        CoinReason::Match(id) => ("match", Some(*id), None, None),
        CoinReason::Cosmetic(id) => ("cosmetic", None, Some(encode(id)?), None),
        CoinReason::Admin(who) => ("admin", None, None, Some(who.as_str())),
    };
    sqlx::query(
        "INSERT INTO coin_ledger (uuid, delta, reason, match_id, cosmetic, admin) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(uuid)
    .bind(delta)
    .bind(reason)
    .bind(match_id)
    .bind(cosmetic)
    .bind(admin)
    .execute(conn)
    .await
    .map_err(store)?;
    Ok(())
}

impl PgProgressionRepository {
    pub async fn connect(url: &str) -> Result<Self, ProgressionStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(store)?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), ProgressionStoreError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|err| ProgressionStoreError::Unavailable(err.to_string()))
    }
}

impl ProgressionRepository for PgProgressionRepository {
    async fn load(&self, uuid: Uuid) -> Result<Option<PlayerRecord>, ProgressionStoreError> {
        let mut conn = self.pool.acquire().await.map_err(store)?;
        load_record(&mut conn, uuid, false).await
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
        let mut tx = self.pool.begin().await.map_err(store)?;
        ensure_row(&mut tx, uuid).await?;
        let record = load_record(&mut tx, uuid, true)
            .await?
            .ok_or_else(|| corrupted("joueur introuvable", uuid))?;
        let mut progress = PlayerProgress::from(record);
        match apply(&mut progress) {
            Ok((value, movements)) => {
                persist(&mut tx, progress.record()).await?;
                for movement in &movements {
                    insert_movement(&mut tx, uuid, movement).await?;
                }
                tx.commit().await.map_err(store)?;
                Ok(Ok((progress.into_record(), value)))
            }
            Err(err) => {
                tx.rollback().await.map_err(store)?;
                Ok(Err(err))
            }
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
        let mut tx = self.pool.begin().await.map_err(store)?;
        let inserted = sqlx::query(
            "INSERT INTO matches (match_id, game, server_id, reported_at, ack) \
             VALUES ($1, $2, $3, now(), 'null'::jsonb) ON CONFLICT DO NOTHING",
        )
        .bind(settle.match_id)
        .bind(encode(&settle.game)?)
        .bind(settle.server_id)
        .execute(&mut *tx)
        .await
        .map_err(store)?
        .rows_affected();

        if inserted == 0 {
            let stored: serde_json::Value =
                sqlx::query("SELECT ack FROM matches WHERE match_id = $1")
                    .bind(settle.match_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(store)?
                    .try_get("ack")
                    .map_err(store)?;
            tx.rollback().await.map_err(store)?;
            let mut ack: MatchReportAck = serde_json::from_value(stored)
                .map_err(|err| corrupted("acquittement illisible", err))?;
            ack.duplicate = true;
            return Ok(ack);
        }

        let ids: BTreeSet<Uuid> = settle.players.iter().copied().collect();
        let mut players = BTreeMap::new();
        for uuid in ids {
            ensure_row(&mut tx, uuid).await?;
            let record = load_record(&mut tx, uuid, true)
                .await?
                .ok_or_else(|| corrupted("joueur introuvable", uuid))?;
            players.insert(uuid, PlayerProgress::from(record));
        }

        let settlement = apply(&mut players);

        for progress in players.values() {
            persist(&mut tx, progress.record()).await?;
        }
        for entry in &settlement.ledger {
            insert_movement(&mut tx, entry.uuid, &entry.movement).await?;
        }

        let mut ack = settlement.ack;
        ack.duplicate = false;
        let stored =
            serde_json::to_value(&ack).map_err(|err| corrupted("acquittement illisible", err))?;
        sqlx::query("UPDATE matches SET ack = $2 WHERE match_id = $1")
            .bind(settle.match_id)
            .bind(stored)
            .execute(&mut *tx)
            .await
            .map_err(store)?;
        tx.commit().await.map_err(store)?;
        Ok(ack)
    }

    async fn ledger(&self, uuid: Uuid) -> Result<Vec<CoinMovement>, ProgressionStoreError> {
        let rows = sqlx::query(
            "SELECT delta, reason, match_id, cosmetic, admin FROM coin_ledger \
             WHERE uuid = $1 ORDER BY id",
        )
        .bind(uuid)
        .fetch_all(&self.pool)
        .await
        .map_err(store)?;

        let mut movements = Vec::with_capacity(rows.len());
        for row in rows {
            let delta: i64 = row.try_get("delta").map_err(store)?;
            let magnitude = Coins(delta.unsigned_abs());
            let delta = if delta < 0 {
                CoinDelta::Debit(magnitude)
            } else {
                CoinDelta::Credit(magnitude)
            };
            let reason: String = row.try_get("reason").map_err(store)?;
            let reason = match reason.as_str() {
                "match" => CoinReason::Match(
                    row.try_get::<Option<Uuid>, _>("match_id")
                        .map_err(store)?
                        .ok_or_else(|| corrupted("ledger", "match_id absent"))?,
                ),
                "cosmetic" => CoinReason::Cosmetic(decode(
                    &row.try_get::<Option<String>, _>("cosmetic")
                        .map_err(store)?
                        .ok_or_else(|| corrupted("ledger", "cosmetic absent"))?,
                )?),
                "admin" => CoinReason::Admin(
                    row.try_get::<Option<String>, _>("admin")
                        .map_err(store)?
                        .ok_or_else(|| corrupted("ledger", "admin absent"))?,
                ),
                other => return Err(corrupted("raison de ledger inconnue", other)),
            };
            movements.push(CoinMovement { delta, reason });
        }
        Ok(movements)
    }
}
