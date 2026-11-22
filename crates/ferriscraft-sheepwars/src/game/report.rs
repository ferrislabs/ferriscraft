use std::future::Future;
use std::time::Duration;

use ferriscraft_protocol::{GameKind, MatchOutcome, MatchParticipant, MatchReport, PlayerReward};
use uuid::Uuid;

use crate::domain::{PlayerId, TeamId};
use crate::orchestrator::OrchestratorClientError;

pub const TICKS_PER_SECOND: u32 = 20;
pub const RETRY_FIRST_DELAY: Duration = Duration::from_secs(1);
pub const RETRY_MAX_DELAY: Duration = Duration::from_secs(10);
pub const RETRY_BUDGET: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantSummary {
    pub player: PlayerId,
    pub outcome: MatchOutcome,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundSummary {
    pub match_id: Uuid,
    pub forced_start: bool,
    pub duration_secs: u32,
    pub participants: Vec<ParticipantSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    player: PlayerId,
    team: TeamId,
    completed: bool,
    eliminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundLog {
    match_id: Uuid,
    forced_start: bool,
    entries: Vec<Entry>,
}

impl RoundLog {
    pub fn start(
        match_id: Uuid,
        forced_start: bool,
        roster: impl IntoIterator<Item = (PlayerId, TeamId)>,
    ) -> Self {
        Self {
            match_id,
            forced_start,
            entries: roster
                .into_iter()
                .map(|(player, team)| Entry {
                    player,
                    team,
                    completed: true,
                    eliminated: false,
                })
                .collect(),
        }
    }

    pub const fn match_id(&self) -> Uuid {
        self.match_id
    }

    pub fn eliminated(&mut self, player: PlayerId) {
        if let Some(entry) = self.entry_mut(player) {
            entry.eliminated = true;
        }
    }

    pub fn left(&mut self, player: PlayerId) {
        if let Some(entry) = self.entry_mut(player)
            && !entry.eliminated
        {
            entry.completed = false;
        }
    }

    pub fn finish(self, winner: Option<TeamId>, elapsed_ticks: u32) -> RoundSummary {
        RoundSummary {
            match_id: self.match_id,
            forced_start: self.forced_start,
            duration_secs: elapsed_ticks / TICKS_PER_SECOND,
            participants: self
                .entries
                .into_iter()
                .map(|entry| ParticipantSummary {
                    player: entry.player,
                    outcome: match winner {
                        Some(team) if team == entry.team => MatchOutcome::Win,
                        Some(_) => MatchOutcome::Loss,
                        None => MatchOutcome::Draw,
                    },
                    completed: entry.completed,
                })
                .collect(),
        }
    }

    fn entry_mut(&mut self, player: PlayerId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.player == player)
    }
}

pub fn build_report(
    summary: &RoundSummary,
    server_id: &str,
    name_of: impl Fn(PlayerId) -> Option<String>,
) -> MatchReport {
    MatchReport {
        match_id: summary.match_id,
        game: GameKind::SheepWars,
        server_id: server_id.to_owned(),
        forced_start: summary.forced_start,
        duration_secs: summary.duration_secs,
        participants: summary
            .participants
            .iter()
            .map(|participant| {
                let uuid = Uuid::from_u128(participant.player.0);
                MatchParticipant {
                    uuid,
                    name: name_of(participant.player).unwrap_or_else(|| uuid.to_string()),
                    outcome: participant.outcome,
                    completed: participant.completed,
                }
            })
            .collect(),
    }
}

pub fn retry_schedule() -> Vec<Duration> {
    let mut delays = Vec::new();
    let mut total = Duration::ZERO;
    let mut next = RETRY_FIRST_DELAY;
    while total + next <= RETRY_BUDGET {
        delays.push(next);
        total += next;
        next = (next * 2).min(RETRY_MAX_DELAY);
    }
    delays
}

pub fn is_retryable(error: &OrchestratorClientError) -> bool {
    match error {
        OrchestratorClientError::Transport(_) => true,
        OrchestratorClientError::HttpStatus(status) => *status >= 500,
    }
}

#[derive(Debug)]
pub enum DeliveryError {
    Rejected(OrchestratorClientError),
    Exhausted(OrchestratorClientError),
}

pub async fn deliver<T, F, Fut>(schedule: &[Duration], mut attempt: F) -> Result<T, DeliveryError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, OrchestratorClientError>>,
{
    let mut delays = schedule.iter();
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) if !is_retryable(&error) => return Err(DeliveryError::Rejected(error)),
            Err(error) => match delays.next() {
                Some(delay) => {
                    tracing::debug!("rapport de manche refusé temporairement : {error}");
                    tokio::time::sleep(*delay).await;
                }
                None => return Err(DeliveryError::Exhausted(error)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardDisplay {
    pub lines: Vec<String>,
    pub title: Option<String>,
}

pub fn reward_display(reward: &PlayerReward) -> RewardDisplay {
    let mut lines = vec![format!(
        "§a+{} XP SheepWars §8• §6+{} coins",
        reward.xp, reward.coins
    )];
    if reward.network_level_up {
        lines.push(format!(
            "§bNiveau réseau §f{} §b: bravo !",
            reward.network.level
        ));
    }
    RewardDisplay {
        lines,
        title: reward
            .game_level_up
            .then(|| format!("§dLevel {} SheepWars !", reward.game.level)),
    }
}

pub fn not_counted_notice() -> String {
    "§eCette manche n'est pas comptée (démarrage forcé, trop peu de joueurs ou manche trop courte) : aucune récompense.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferriscraft_protocol::LevelProgress;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const A: PlayerId = PlayerId(1);
    const B: PlayerId = PlayerId(2);
    const C: PlayerId = PlayerId(3);

    fn log(forced: bool) -> RoundLog {
        RoundLog::start(
            Uuid::nil(),
            forced,
            [(A, TeamId::Red), (B, TeamId::Blue), (C, TeamId::Red)],
        )
    }

    fn outcomes(summary: &RoundSummary) -> Vec<MatchOutcome> {
        summary.participants.iter().map(|p| p.outcome).collect()
    }

    fn progress(level: u32) -> LevelProgress {
        LevelProgress {
            level,
            xp_into_level: 0,
            xp_for_next: 100,
            total_xp: 0,
        }
    }

    fn reward(game_up: bool, network_up: bool) -> PlayerReward {
        PlayerReward {
            uuid: Uuid::nil(),
            xp: 150,
            coins: 30,
            game: progress(12),
            game_level_up: game_up,
            network: progress(5),
            network_level_up: network_up,
        }
    }

    #[test]
    fn the_winning_team_wins_and_the_other_loses() {
        let summary = log(false).finish(Some(TeamId::Red), 0);
        assert_eq!(
            outcomes(&summary),
            vec![MatchOutcome::Win, MatchOutcome::Loss, MatchOutcome::Win]
        );
    }

    #[test]
    fn no_winner_is_a_draw_for_everyone() {
        let summary = log(false).finish(None, 0);
        assert!(outcomes(&summary).iter().all(|o| *o == MatchOutcome::Draw));
    }

    #[test]
    fn a_player_who_stays_or_is_eliminated_completed() {
        let mut log = log(false);
        log.eliminated(C);
        let summary = log.finish(Some(TeamId::Blue), 0);
        assert!(summary.participants.iter().all(|p| p.completed));
    }

    #[test]
    fn a_player_who_leaves_before_elimination_did_not_complete() {
        let mut log = log(false);
        log.left(A);
        let summary = log.finish(Some(TeamId::Blue), 0);
        let completed: Vec<bool> = summary.participants.iter().map(|p| p.completed).collect();
        assert_eq!(completed, vec![false, true, true]);
    }

    #[test]
    fn leaving_after_elimination_stays_completed() {
        let mut log = log(false);
        log.eliminated(A);
        log.left(A);
        assert!(log.finish(None, 0).participants[0].completed);
    }

    #[test]
    fn unknown_players_are_ignored() {
        let mut log = log(false);
        log.left(PlayerId(99));
        log.eliminated(PlayerId(99));
        assert_eq!(log.finish(None, 0).participants.len(), 3);
    }

    #[test]
    fn forced_start_is_carried_into_the_summary() {
        assert!(log(true).finish(None, 0).forced_start);
        assert!(!log(false).finish(None, 0).forced_start);
    }

    #[test]
    fn duration_is_playing_ticks_over_twenty() {
        assert_eq!(log(false).finish(None, 20 * 90 + 19).duration_secs, 90);
        assert_eq!(log(false).finish(None, 19).duration_secs, 0);
    }

    #[test]
    fn the_report_carries_names_identity_and_flags() {
        let mut log = log(true);
        log.left(B);
        let summary = log.finish(Some(TeamId::Red), 20 * 61);
        let report = build_report(&summary, "sw-1", |id| (id == A).then(|| "Alex".to_owned()));
        assert_eq!(report.game, GameKind::SheepWars);
        assert_eq!(report.server_id, "sw-1");
        assert!(report.forced_start);
        assert_eq!(report.duration_secs, 61);
        assert_eq!(report.participants.len(), 3);
        assert_eq!(report.participants[0].name, "Alex");
        assert_eq!(report.participants[0].uuid, Uuid::from_u128(1));
        assert_eq!(report.participants[1].name, Uuid::from_u128(2).to_string());
        assert!(!report.participants[1].completed);
        assert_eq!(report.participants[1].outcome, MatchOutcome::Loss);
    }

    #[test]
    fn the_retry_schedule_backs_off_from_one_to_ten_seconds_within_two_minutes() {
        let schedule = retry_schedule();
        let secs: Vec<u64> = schedule.iter().map(Duration::as_secs).collect();
        assert_eq!(&secs[..6], &[1, 2, 4, 8, 10, 10]);
        assert!(secs.iter().skip(4).all(|s| *s == 10));
        let total: Duration = schedule.iter().sum();
        assert!(total <= RETRY_BUDGET);
        assert!(total + RETRY_MAX_DELAY > RETRY_BUDGET);
    }

    #[test]
    fn server_errors_are_retried_and_client_errors_are_final() {
        assert!(is_retryable(&OrchestratorClientError::HttpStatus(500)));
        assert!(is_retryable(&OrchestratorClientError::HttpStatus(503)));
        assert!(!is_retryable(&OrchestratorClientError::HttpStatus(422)));
        assert!(!is_retryable(&OrchestratorClientError::HttpStatus(401)));
    }

    #[tokio::test]
    async fn delivery_retries_then_succeeds() {
        let calls = AtomicUsize::new(0);
        let schedule = [Duration::from_millis(1); 5];
        let result = deliver(&schedule, || async {
            if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(OrchestratorClientError::HttpStatus(503))
            } else {
                Ok(7)
            }
        })
        .await;
        assert_eq!(result.ok(), Some(7));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn delivery_stops_on_a_client_error() {
        let calls = AtomicUsize::new(0);
        let schedule = [Duration::from_millis(1); 5];
        let result: Result<(), _> = deliver(&schedule, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(OrchestratorClientError::HttpStatus(422))
        })
        .await;
        assert!(matches!(result, Err(DeliveryError::Rejected(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn delivery_gives_up_when_the_budget_is_spent() {
        let calls = AtomicUsize::new(0);
        let schedule = [Duration::from_millis(1); 3];
        let result: Result<(), _> = deliver(&schedule, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(OrchestratorClientError::HttpStatus(500))
        })
        .await;
        assert!(matches!(result, Err(DeliveryError::Exhausted(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_reward_shows_xp_and_coins() {
        let display = reward_display(&reward(false, false));
        assert_eq!(display.lines, vec!["§a+150 XP SheepWars §8• §6+30 coins"]);
        assert_eq!(display.title, None);
    }

    #[test]
    fn level_ups_add_a_title_and_a_network_message() {
        let display = reward_display(&reward(true, true));
        assert_eq!(display.title.as_deref(), Some("§dLevel 12 SheepWars !"));
        assert_eq!(display.lines.len(), 2);
        assert!(display.lines[1].contains('5'));
    }

    #[test]
    fn the_not_counted_notice_is_in_french() {
        let notice = not_counted_notice();
        assert!(notice.contains("pas comptée") && notice.contains("démarrage forcé"));
    }
}
