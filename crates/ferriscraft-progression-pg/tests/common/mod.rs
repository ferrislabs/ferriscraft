use std::sync::atomic::{AtomicBool, Ordering};

use ferriscraft_progression::{
    CoinDelta, CoinMovement, CoinReason, Coins, LedgerEntry, MatchToSettle, ProgressionRepository,
    Settlement, UnlockError, Xp,
};
use ferriscraft_protocol::{
    CosmeticId, CosmeticSlot, GameKind, MatchOutcome, MatchReportAck, PlayerRank,
};
use uuid::Uuid;

pub const ALEX: Uuid = Uuid::from_u128(0xA1);
pub const BOB: Uuid = Uuid::from_u128(0xB2);
pub const UNKNOWN: Uuid = Uuid::from_u128(0xFF);
pub const MATCH: Uuid = Uuid::from_u128(0x99);

pub async fn contract<R: ProgressionRepository>(repo: R) {
    assert_eq!(repo.load(UNKNOWN).await.unwrap(), None);

    let (record, ()) = repo
        .update(ALEX, |progress| {
            progress.set_identity("Alex", PlayerRank::Member);
            let movement = progress.grant(Coins(1_000), "console");
            Ok::<_, UnlockError>(((), vec![movement]))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.name, "Alex");
    assert_eq!(record.rank, PlayerRank::Member);
    assert_eq!(record.coins, Coins(1_000));
    assert_eq!(repo.load(ALEX).await.unwrap(), Some(record));

    let rejected = repo
        .update(ALEX, |progress| {
            let _ = progress.grant(Coins(5_000), "rollback");
            Err::<((), Vec<CoinMovement>), _>("refusé")
        })
        .await
        .unwrap();
    assert_eq!(rejected, Err("refusé"));
    let after_err = repo.load(ALEX).await.unwrap().unwrap();
    assert_eq!(after_err.coins, Coins(1_000));
    assert_eq!(repo.ledger(ALEX).await.unwrap().len(), 1);

    let (record, ()) = repo
        .update(ALEX, |progress| {
            let movement = progress.purchase(CosmeticId::GadgetWindCharge)?;
            progress
                .equip(
                    CosmeticSlot::LobbyGadget,
                    Some(CosmeticId::GadgetWindCharge),
                )
                .unwrap();
            Ok::<_, UnlockError>(((), vec![movement]))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.coins, Coins(500));
    assert!(record.purchased.contains(&CosmeticId::GadgetWindCharge));
    assert_eq!(
        record.equipped.get(&CosmeticSlot::LobbyGadget),
        Some(&CosmeticId::GadgetWindCharge)
    );
    assert_eq!(repo.load(ALEX).await.unwrap(), Some(record));

    let overdraw = repo
        .update(ALEX, |progress| {
            let movement = progress.purchase(CosmeticId::TrailFlame)?;
            Ok::<_, UnlockError>(((), vec![movement]))
        })
        .await
        .unwrap();
    assert!(matches!(
        overdraw,
        Err(UnlockError::InsufficientCoins { .. })
    ));
    let still = repo.load(ALEX).await.unwrap().unwrap();
    assert_eq!(still.coins, Coins(500));
    assert!(!still.purchased.contains(&CosmeticId::TrailFlame));

    let ledger = repo.ledger(ALEX).await.unwrap();
    assert_eq!(
        ledger,
        vec![
            CoinMovement {
                delta: CoinDelta::Credit(Coins(1_000)),
                reason: CoinReason::Admin("console".to_owned()),
            },
            CoinMovement {
                delta: CoinDelta::Debit(Coins(500)),
                reason: CoinReason::Cosmetic(CosmeticId::GadgetWindCharge),
            },
        ]
    );

    let players = [BOB, ALEX];
    let settle = MatchToSettle {
        match_id: MATCH,
        game: GameKind::SheepWars,
        server_id: "sheepwars-1",
        players: &players,
    };
    let applied = AtomicBool::new(false);
    let ack = repo
        .settle_match(settle, |map| {
            applied.store(true, Ordering::SeqCst);
            let mut ledger = Vec::new();
            for (uuid, progress) in map.iter_mut() {
                let outcome = if *uuid == ALEX {
                    MatchOutcome::Win
                } else {
                    MatchOutcome::Loss
                };
                let credit = progress.record_match(MATCH, GameKind::SheepWars, outcome);
                ledger.push(LedgerEntry {
                    uuid: *uuid,
                    movement: credit.movement,
                });
            }
            Settlement {
                ack: MatchReportAck {
                    match_id: MATCH,
                    counted: true,
                    duplicate: false,
                    rewards: Vec::new(),
                },
                ledger,
            }
        })
        .await
        .unwrap();
    assert!(applied.load(Ordering::SeqCst));
    assert!(ack.counted);
    assert!(!ack.duplicate);

    let alex = repo.load(ALEX).await.unwrap().unwrap();
    let bob = repo.load(BOB).await.unwrap().unwrap();
    assert_eq!(alex.games[&GameKind::SheepWars].xp, Xp(150));
    assert_eq!(alex.games[&GameKind::SheepWars].wins, 1);
    assert_eq!(alex.coins, Coins(530));
    assert_eq!(bob.games[&GameKind::SheepWars].xp, Xp(50));
    assert_eq!(bob.coins, Coins(10));
    assert_eq!(repo.ledger(ALEX).await.unwrap().len(), 3);
    assert_eq!(repo.ledger(BOB).await.unwrap().len(), 1);

    let replayed = AtomicBool::new(false);
    let replay = repo
        .settle_match(settle, |_| {
            replayed.store(true, Ordering::SeqCst);
            Settlement {
                ack: MatchReportAck {
                    match_id: MATCH,
                    counted: false,
                    duplicate: false,
                    rewards: Vec::new(),
                },
                ledger: Vec::new(),
            }
        })
        .await
        .unwrap();
    assert!(!replayed.load(Ordering::SeqCst));
    assert!(replay.duplicate);
    assert_eq!(replay.match_id, ack.match_id);
    assert_eq!(replay.counted, ack.counted);
    assert_eq!(repo.load(ALEX).await.unwrap().unwrap(), alex);
    assert_eq!(repo.load(BOB).await.unwrap().unwrap(), bob);
    assert_eq!(repo.ledger(ALEX).await.unwrap().len(), 3);
    assert_eq!(repo.ledger(BOB).await.unwrap().len(), 1);
}
