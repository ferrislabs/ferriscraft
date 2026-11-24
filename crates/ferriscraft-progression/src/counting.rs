use std::collections::HashSet;

use ferriscraft_protocol::{GameKind, MatchOutcome, MatchReport};
use uuid::Uuid;

pub const MIN_PARTICIPANTS: usize = 2;
pub const MIN_DURATION_SECS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotCounted {
    #[error("the round was force-started")]
    ForcedStart,
    #[error("fewer than {MIN_PARTICIPANTS} participants")]
    TooFewPlayers,
    #[error("the round lasted less than {MIN_DURATION_SECS} seconds")]
    TooShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountedParticipant {
    pub uuid: Uuid,
    pub outcome: MatchOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedMatch {
    pub match_id: Uuid,
    pub game: GameKind,
    pub rewarded: Vec<CountedParticipant>,
}

pub fn count_match(report: &MatchReport) -> Result<CountedMatch, NotCounted> {
    if report.forced_start {
        return Err(NotCounted::ForcedStart);
    }
    let mut seen = HashSet::with_capacity(report.participants.len());
    let distinct: Vec<_> = report
        .participants
        .iter()
        .filter(|participant| seen.insert(participant.uuid))
        .collect();
    if distinct.len() < MIN_PARTICIPANTS {
        return Err(NotCounted::TooFewPlayers);
    }
    if report.duration_secs < MIN_DURATION_SECS {
        return Err(NotCounted::TooShort);
    }
    let rewarded = distinct
        .into_iter()
        .filter(|participant| participant.completed)
        .map(|participant| CountedParticipant {
            uuid: participant.uuid,
            outcome: participant.outcome,
        })
        .collect();
    Ok(CountedMatch {
        match_id: report.match_id,
        game: report.game,
        rewarded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferriscraft_protocol::MatchParticipant;

    fn participant(n: u128, outcome: MatchOutcome, completed: bool) -> MatchParticipant {
        MatchParticipant {
            uuid: Uuid::from_u128(n),
            name: format!("player{n}"),
            outcome,
            completed,
        }
    }

    fn report(participants: Vec<MatchParticipant>) -> MatchReport {
        MatchReport {
            match_id: Uuid::from_u128(99),
            game: GameKind::SheepWars,
            server_id: "sheepwars-1".into(),
            forced_start: false,
            duration_secs: 300,
            participants,
        }
    }

    fn duel() -> MatchReport {
        report(vec![
            participant(1, MatchOutcome::Win, true),
            participant(2, MatchOutcome::Loss, true),
        ])
    }

    #[test]
    fn a_towerwars_duel_is_counted_for_its_own_game() {
        let mut towerwars = duel();
        towerwars.game = GameKind::TowerWars;
        towerwars.server_id = "towerwars-1".into();
        let counted = count_match(&towerwars).unwrap();
        assert_eq!(counted.game, GameKind::TowerWars);
        assert_eq!(counted.rewarded.len(), 2);
    }

    #[test]
    fn a_normal_duel_is_counted() {
        let counted = count_match(&duel()).unwrap();
        assert_eq!(counted.rewarded.len(), 2);
        assert_eq!(counted.match_id, Uuid::from_u128(99));
    }

    #[test]
    fn spec_prg_3_forced_start_is_not_counted() {
        let mut forced = duel();
        forced.forced_start = true;
        assert_eq!(count_match(&forced), Err(NotCounted::ForcedStart));
    }

    #[test]
    fn spec_prg_4_a_single_player_is_not_counted() {
        let solo = report(vec![participant(1, MatchOutcome::Win, true)]);
        assert_eq!(count_match(&solo), Err(NotCounted::TooFewPlayers));
    }

    #[test]
    fn a_duplicated_participant_counts_once() {
        let same = report(vec![
            participant(1, MatchOutcome::Win, true),
            participant(1, MatchOutcome::Win, true),
        ]);
        assert_eq!(count_match(&same), Err(NotCounted::TooFewPlayers));
    }

    #[test]
    fn spec_prg_4_a_short_round_is_not_counted() {
        let mut short = duel();
        short.duration_secs = MIN_DURATION_SECS - 1;
        assert_eq!(count_match(&short), Err(NotCounted::TooShort));
        short.duration_secs = MIN_DURATION_SECS;
        assert!(count_match(&short).is_ok());
    }

    #[test]
    fn spec_prg_5_a_player_who_left_is_not_rewarded() {
        let with_leaver = report(vec![
            participant(1, MatchOutcome::Win, true),
            participant(2, MatchOutcome::Loss, true),
            participant(3, MatchOutcome::Loss, false),
        ]);
        let counted = count_match(&with_leaver).unwrap();
        let rewarded: Vec<_> = counted.rewarded.iter().map(|p| p.uuid).collect();
        assert_eq!(rewarded, [Uuid::from_u128(1), Uuid::from_u128(2)]);
    }
}
