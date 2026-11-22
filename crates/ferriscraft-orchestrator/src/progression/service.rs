use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;

use ferriscraft_progression::{
    Coins, EquipError, LedgerEntry, MatchToSettle, PlayerProgress, PlayerRecord,
    ProgressionRepository, ProgressionStoreError, Settlement, UnlockError, count_match,
};
use ferriscraft_protocol::{
    AdminCoinGrant, CosmeticId, EquipRequest, MatchReport, MatchReportAck, PlayerIdentityUpdate,
    PlayerProfile,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ProgressionError {
    #[error(transparent)]
    Store(#[from] ProgressionStoreError),
    #[error("rapport de partie sans participant")]
    EmptyMatch,
    #[error(transparent)]
    Unlock(#[from] UnlockError),
    #[error(transparent)]
    Equip(#[from] EquipError),
}

#[derive(Clone)]
pub struct ProgressionService<R> {
    repo: R,
}

fn profile_of(record: PlayerRecord) -> PlayerProfile {
    PlayerProfile::from(&PlayerProgress::from(record))
}

impl<R: ProgressionRepository> ProgressionService<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub fn repository(&self) -> &R {
        &self.repo
    }

    pub async fn report_match(
        &self,
        report: &MatchReport,
        seen: &HashSet<Uuid>,
    ) -> Result<MatchReportAck, ProgressionError> {
        if report.participants.is_empty() {
            return Err(ProgressionError::EmptyMatch);
        }
        let participants = report
            .participants
            .iter()
            .filter(|participant| {
                let known = seen.contains(&participant.uuid);
                if !known {
                    tracing::warn!(
                        server_id = %report.server_id,
                        uuid = %participant.uuid,
                        "Participant jamais vu dans un heartbeat de ce serveur, ignoré"
                    );
                }
                known
            })
            .cloned()
            .collect();
        let filtered = MatchReport {
            match_id: report.match_id,
            game: report.game,
            server_id: report.server_id.clone(),
            forced_start: report.forced_start,
            duration_secs: report.duration_secs,
            participants,
        };
        let counted = match count_match(&filtered) {
            Ok(counted) => counted,
            Err(reason) => {
                tracing::info!(match_id = %report.match_id, %reason, "Partie non comptée");
                return Ok(MatchReportAck {
                    match_id: report.match_id,
                    counted: false,
                    duplicate: false,
                    rewards: Vec::new(),
                });
            }
        };
        let names: HashMap<Uuid, &str> = filtered
            .participants
            .iter()
            .map(|participant| (participant.uuid, participant.name.as_str()))
            .collect();
        let players: Vec<Uuid> = counted
            .rewarded
            .iter()
            .map(|participant| participant.uuid)
            .collect();
        let settle = MatchToSettle {
            match_id: counted.match_id,
            game: counted.game,
            server_id: &report.server_id,
            players: &players,
        };
        let ack = self
            .repo
            .settle_match(settle, |map: &mut BTreeMap<Uuid, PlayerProgress>| {
                let mut rewards = Vec::with_capacity(counted.rewarded.len());
                let mut ledger = Vec::with_capacity(counted.rewarded.len());
                for participant in &counted.rewarded {
                    let Some(player) = map.get_mut(&participant.uuid) else {
                        continue;
                    };
                    if player.record().name.is_empty()
                        && let Some(name) = names.get(&participant.uuid)
                    {
                        let rank = player.rank();
                        player.set_identity(name, rank);
                    }
                    let credit =
                        player.record_match(counted.match_id, counted.game, participant.outcome);
                    rewards.push(credit.to_player_reward(participant.uuid));
                    ledger.push(LedgerEntry {
                        uuid: participant.uuid,
                        movement: credit.movement,
                    });
                }
                Settlement {
                    ack: MatchReportAck {
                        match_id: counted.match_id,
                        counted: true,
                        duplicate: false,
                        rewards,
                    },
                    ledger,
                }
            })
            .await?;
        Ok(ack)
    }

    pub async fn profile(&self, uuid: Uuid) -> Result<PlayerProfile, ProgressionError> {
        let progress = self
            .repo
            .load(uuid)
            .await?
            .map_or_else(|| PlayerProgress::new(uuid), PlayerProgress::from);
        Ok(PlayerProfile::from(&progress))
    }

    pub async fn set_identity(
        &self,
        uuid: Uuid,
        identity: &PlayerIdentityUpdate,
    ) -> Result<(), ProgressionError> {
        let _ = self
            .repo
            .update(uuid, |player| {
                player.set_identity(&identity.name, identity.rank);
                Ok::<_, Infallible>(((), Vec::new()))
            })
            .await?;
        Ok(())
    }

    pub async fn purchase(
        &self,
        uuid: Uuid,
        cosmetic: CosmeticId,
    ) -> Result<PlayerProfile, ProgressionError> {
        let (record, ()) = self
            .repo
            .update(uuid, |player| {
                player
                    .purchase(cosmetic)
                    .map(|movement| ((), vec![movement]))
            })
            .await??;
        Ok(profile_of(record))
    }

    pub async fn equip(
        &self,
        uuid: Uuid,
        request: &EquipRequest,
    ) -> Result<PlayerProfile, ProgressionError> {
        let (record, ()) = self
            .repo
            .update(uuid, |player| {
                player
                    .equip(request.slot, request.cosmetic)
                    .map(|()| ((), Vec::new()))
            })
            .await??;
        Ok(profile_of(record))
    }

    pub async fn grant(
        &self,
        uuid: Uuid,
        grant: &AdminCoinGrant,
    ) -> Result<PlayerProfile, ProgressionError> {
        let (record, ()) = self
            .repo
            .update(uuid, |player| {
                let movement = player.grant(Coins(grant.amount), &grant.admin);
                Ok::<_, Infallible>(((), vec![movement]))
            })
            .await?
            .unwrap_or_else(|never| match never {});
        Ok(profile_of(record))
    }
}
