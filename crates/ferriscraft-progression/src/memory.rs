use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{CoinMovement, PlayerProgress, PlayerRecord};
use ferriscraft_protocol::MatchReportAck;
use uuid::Uuid;

use crate::repository::{
    LedgerEntry, MatchToSettle, ProgressionRepository, ProgressionStoreError, Settlement,
};

#[derive(Debug, Default)]
struct State {
    players: BTreeMap<Uuid, PlayerRecord>,
    ledger: Vec<LedgerEntry>,
    settled: HashMap<Uuid, MatchReportAck>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryProgressionRepository {
    state: Arc<Mutex<State>>,
}

impl MemoryProgressionRepository {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn progress_of(state: &State, uuid: Uuid) -> PlayerProgress {
        state
            .players
            .get(&uuid)
            .cloned()
            .map_or_else(|| PlayerProgress::new(uuid), PlayerProgress::from)
    }
}

impl ProgressionRepository for MemoryProgressionRepository {
    fn load(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Option<PlayerRecord>, ProgressionStoreError>> + Send {
        ready(Ok(self.lock().players.get(&uuid).cloned()))
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
        let outcome = {
            let mut state = self.lock();
            let mut progress = Self::progress_of(&state, uuid);
            match apply(&mut progress) {
                Ok((value, movements)) => {
                    let record = progress.into_record();
                    state.players.insert(uuid, record.clone());
                    state.ledger.extend(
                        movements
                            .into_iter()
                            .map(|movement| LedgerEntry { uuid, movement }),
                    );
                    Ok((record, value))
                }
                Err(err) => Err(err),
            }
        };
        ready(Ok(outcome))
    }

    fn settle_match<F>(
        &self,
        settle: MatchToSettle<'_>,
        apply: F,
    ) -> impl Future<Output = Result<MatchReportAck, ProgressionStoreError>> + Send
    where
        F: FnOnce(&mut BTreeMap<Uuid, PlayerProgress>) -> Settlement + Send,
    {
        let ack = {
            let mut state = self.lock();
            if let Some(stored) = state.settled.get(&settle.match_id) {
                MatchReportAck {
                    duplicate: true,
                    ..stored.clone()
                }
            } else {
                let mut players: BTreeMap<Uuid, PlayerProgress> = settle
                    .players
                    .iter()
                    .map(|&uuid| (uuid, Self::progress_of(&state, uuid)))
                    .collect();
                let settlement = apply(&mut players);
                for (uuid, progress) in players {
                    state.players.insert(uuid, progress.into_record());
                }
                state.ledger.extend(settlement.ledger);
                let ack = MatchReportAck {
                    duplicate: false,
                    ..settlement.ack
                };
                state.settled.insert(settle.match_id, ack.clone());
                ack
            }
        };
        ready(Ok(ack))
    }

    fn ledger(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Vec<CoinMovement>, ProgressionStoreError>> + Send {
        let movements = self
            .lock()
            .ledger
            .iter()
            .filter(|entry| entry.uuid == uuid)
            .map(|entry| entry.movement.clone())
            .collect();
        ready(Ok(movements))
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use ferriscraft_protocol::{GameKind, MatchOutcome};

    use super::*;
    use crate::{Coins, UnlockError};

    const RED: Uuid = Uuid::from_u128(1);
    const BLUE: Uuid = Uuid::from_u128(2);
    const MATCH: Uuid = Uuid::from_u128(99);

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("memory repository futures never suspend"),
        }
    }

    fn grant(repo: &MemoryProgressionRepository, uuid: Uuid, amount: u64) {
        let _ = block_on(repo.update(uuid, |p| {
            Ok::<_, ()>(((), vec![p.grant(Coins(amount), "test")]))
        }))
        .unwrap();
    }

    fn settle_duel(repo: &MemoryProgressionRepository) -> MatchReportAck {
        let players = [RED, BLUE];
        block_on(repo.settle_match(
            MatchToSettle {
                match_id: MATCH,
                game: GameKind::SheepWars,
                server_id: "sheepwars-1",
                players: &players,
            },
            |map| {
                let mut rewards = Vec::new();
                let mut ledger = Vec::new();
                for (uuid, outcome) in [(RED, MatchOutcome::Win), (BLUE, MatchOutcome::Loss)] {
                    let credit = map.get_mut(&uuid).unwrap().record_match(
                        MATCH,
                        GameKind::SheepWars,
                        outcome,
                    );
                    rewards.push(credit.to_player_reward(uuid));
                    ledger.push(LedgerEntry {
                        uuid,
                        movement: credit.movement,
                    });
                }
                Settlement {
                    ack: MatchReportAck {
                        match_id: MATCH,
                        counted: true,
                        duplicate: false,
                        rewards,
                    },
                    ledger,
                }
            },
        ))
        .unwrap()
    }

    #[test]
    fn load_returns_none_for_an_unknown_player() {
        let repo = MemoryProgressionRepository::default();
        assert_eq!(block_on(repo.load(RED)).unwrap(), None);
    }

    #[test]
    fn update_persists_the_record_and_appends_the_ledger() {
        let repo = MemoryProgressionRepository::default();
        grant(&repo, RED, 100);
        let record = block_on(repo.load(RED)).unwrap().unwrap();
        assert_eq!(record.coins, Coins(100));
        assert_eq!(block_on(repo.ledger(RED)).unwrap().len(), 1);
    }

    #[test]
    fn update_error_persists_nothing() {
        let repo = MemoryProgressionRepository::default();
        grant(&repo, RED, 100);
        let result = block_on(repo.update(RED, |p| {
            let _ = p.grant(Coins(5), "lost");
            p.purchase(ferriscraft_protocol::CosmeticId::SheepTrailFlame)
                .map(|m| ((), vec![m]))
        }))
        .unwrap();
        assert!(matches!(result, Err(UnlockError::InsufficientCoins { .. })));
        let record = block_on(repo.load(RED)).unwrap().unwrap();
        assert_eq!(record.coins, Coins(100));
        assert_eq!(block_on(repo.ledger(RED)).unwrap().len(), 1);
    }

    #[test]
    fn update_error_on_an_unknown_player_creates_nothing() {
        let repo = MemoryProgressionRepository::default();
        let result = block_on(repo.update(RED, |p| {
            p.purchase(ferriscraft_protocol::CosmeticId::SheepTrailFlame)
                .map(|m| ((), vec![m]))
        }))
        .unwrap();
        assert!(result.is_err());
        assert_eq!(block_on(repo.load(RED)).unwrap(), None);
    }

    #[test]
    fn settle_match_credits_every_player_and_stores_the_ack() {
        let repo = MemoryProgressionRepository::default();
        let ack = settle_duel(&repo);
        assert!(ack.counted && !ack.duplicate);
        assert_eq!(block_on(repo.load(RED)).unwrap().unwrap().coins, Coins(30));
        assert_eq!(block_on(repo.load(BLUE)).unwrap().unwrap().coins, Coins(10));
        assert_eq!(block_on(repo.ledger(RED)).unwrap().len(), 1);
    }

    #[test]
    fn spec_prg_2_settling_twice_is_a_duplicate_and_does_not_apply() {
        let repo = MemoryProgressionRepository::default();
        let first = settle_duel(&repo);
        let players = [RED, BLUE];
        let second = block_on(repo.settle_match(
            MatchToSettle {
                match_id: MATCH,
                game: GameKind::SheepWars,
                server_id: "sheepwars-1",
                players: &players,
            },
            |_| panic!("apply must not run for a settled match"),
        ))
        .unwrap();
        assert!(second.duplicate);
        assert_eq!(second.rewards.len(), first.rewards.len());
        assert_eq!(block_on(repo.load(RED)).unwrap().unwrap().coins, Coins(30));
        assert_eq!(block_on(repo.ledger(RED)).unwrap().len(), 1);
    }

    #[test]
    fn ledger_keeps_insertion_order_per_player() {
        let repo = MemoryProgressionRepository::default();
        grant(&repo, RED, 1);
        grant(&repo, BLUE, 7);
        grant(&repo, RED, 2);
        let ledger = block_on(repo.ledger(RED)).unwrap();
        let signed: Vec<i128> = ledger.iter().map(|m| m.delta.signed()).collect();
        assert_eq!(signed, vec![1, 2]);
    }

    #[test]
    fn clones_share_the_same_state() {
        let repo = MemoryProgressionRepository::default();
        let other = repo.clone();
        grant(&repo, RED, 10);
        assert_eq!(block_on(other.load(RED)).unwrap().unwrap().coins, Coins(10));
    }
}
