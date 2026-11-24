use ferriscraft_protocol::ServerPhase;
use thiserror::Error;
use uuid::Uuid;

use super::bot::{BOT_TURN_TICKS, Bot};
use super::intent::Intent;
use super::lanes::assign_seats;
use super::report::MatchLog;
use crate::config::TowerWarsConfig;
use crate::domain::{GameMode, LaneId, PlayerId, TICKS_PER_SECOND};
use crate::map::Map;
use crate::sim::{Event, Match};

pub const RESET_DELAY_TICKS: u32 = 20 * 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    pub mode: GameMode,
    pub min_players: usize,
    pub max_players: usize,
    pub countdown_ticks: u32,
    pub end_delay_ticks: u32,
    pub seed: Option<u64>,
    pub needs_expected: bool,
}

impl From<&TowerWarsConfig> for Rules {
    fn from(config: &TowerWarsConfig) -> Self {
        Self {
            mode: config.mode,
            min_players: config.min_players,
            max_players: config.max_players,
            countdown_ticks: config.countdown_ticks,
            end_delay_ticks: config.end_delay_ticks,
            seed: config.seed,
            needs_expected: config.orchestrator_url.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndBehavior {
    EphemeralPod,
    ResetRound { return_to_lobby: bool },
}

impl Default for EndBehavior {
    fn default() -> Self {
        Self::ResetRound {
            return_to_lobby: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ForceStartError {
    #[error("la partie a déjà commencé")]
    AlreadyStarted,
    #[error("personne n'est dans la partie")]
    NoPlayers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Waiting,
    Countdown { remaining: u32 },
    Playing,
    Ended { winner: Option<LaneId> },
}

#[derive(Debug, Default)]
pub struct Tick {
    pub intents: Vec<Intent>,
    pub events: Vec<Event>,
}

#[derive(Debug)]
pub struct Session {
    map: Map,
    rules: Rules,
    stage: Stage,
    roster: Vec<PlayerId>,
    game: Option<Match>,
    reset_in: Option<u32>,
    end_behavior: EndBehavior,
    server_ending: bool,
    log: Option<MatchLog>,
    forced_start: bool,
    pending_report: Option<Uuid>,
    delay_elapsed: bool,
    bots: Vec<PlayerId>,
    brains: Vec<Bot>,
    expected: Option<u32>,
}

fn bots_to_fill(mode: GameMode, humans: usize) -> Vec<PlayerId> {
    let missing = mode.min_players().saturating_sub(humans);
    (0..missing)
        .map(|index| PlayerId(Uuid::from_u128(u128::MAX - index as u128)))
        .collect()
}

fn random_seed() -> u64 {
    Uuid::new_v4().as_u64_pair().0
}

impl Session {
    pub fn new(map: Map, rules: Rules, end_behavior: EndBehavior) -> Self {
        Self {
            map,
            rules: Rules {
                end_delay_ticks: rules.end_delay_ticks.max(1),
                ..rules
            },
            stage: Stage::Waiting,
            roster: Vec::new(),
            game: None,
            reset_in: None,
            end_behavior,
            server_ending: false,
            log: None,
            forced_start: false,
            pending_report: None,
            delay_elapsed: false,
            bots: Vec::new(),
            brains: Vec::new(),
            expected: None,
        }
    }

    pub fn is_bot(&self, id: PlayerId) -> bool {
        self.bots.contains(&id)
    }

    pub const fn rules(&self) -> &Rules {
        &self.rules
    }

    pub const fn map(&self) -> &Map {
        &self.map
    }

    pub const fn stage(&self) -> Stage {
        self.stage
    }

    pub const fn expected(&self) -> Option<u32> {
        self.expected
    }

    pub fn threshold(&self) -> Option<usize> {
        if self.rules.needs_expected {
            self.expected.map(|expected| (expected as usize).max(1))
        } else {
            Some(self.rules.min_players.max(1))
        }
    }

    fn reached(&self) -> bool {
        self.threshold()
            .is_some_and(|threshold| self.roster.len() >= threshold)
    }

    pub fn set_expected(&mut self, expected: Option<u32>) -> Vec<Intent> {
        if expected.is_some() {
            self.expected = expected;
        }
        if self.server_ending {
            return Vec::new();
        }
        match self.stage {
            Stage::Waiting => self.open(),
            Stage::Countdown { .. } => self.cancel_countdown(),
            Stage::Playing | Stage::Ended { .. } => Vec::new(),
        }
    }

    fn open(&mut self) -> Vec<Intent> {
        if self.stage != Stage::Waiting || self.roster.is_empty() || !self.reached() {
            return Vec::new();
        }
        if self.threshold() == Some(1) {
            return self.start();
        }
        self.stage = Stage::Countdown {
            remaining: self.rules.countdown_ticks.max(1),
        };
        vec![Intent::Broadcast(format!(
            "§aLa partie commence dans §f{} §asecondes.",
            self.rules.countdown_ticks.div_ceil(TICKS_PER_SECOND)
        ))]
    }

    fn cancel_countdown(&mut self) -> Vec<Intent> {
        if self.reached() {
            return Vec::new();
        }
        self.stage = Stage::Waiting;
        vec![Intent::Broadcast(
            "§eCompte à rebours annulé : joueurs insuffisants.".to_owned(),
        )]
    }

    pub fn roster(&self) -> &[PlayerId] {
        &self.roster
    }

    pub fn game(&self) -> Option<&Match> {
        self.game.as_ref()
    }

    pub fn playing(&mut self) -> Option<&mut Match> {
        if self.is_playing() {
            self.game.as_mut()
        } else {
            None
        }
    }

    pub fn is_playing(&self) -> bool {
        !self.server_ending && self.stage == Stage::Playing
    }

    pub const fn is_server_ending(&self) -> bool {
        self.server_ending
    }

    pub fn server_phase(&self) -> ServerPhase {
        if self.server_ending {
            return ServerPhase::Ending;
        }
        match self.stage {
            Stage::Waiting => ServerPhase::Waiting,
            Stage::Countdown { .. } => ServerPhase::Countdown,
            Stage::Playing | Stage::Ended { .. } => ServerPhase::InGame,
        }
    }

    pub fn lane_of(&self, player: PlayerId) -> Option<LaneId> {
        self.game
            .as_ref()
            .and_then(|game| game.world().player(player))
            .map(|state| state.lane)
    }

    pub fn join(&mut self, player: PlayerId) -> Vec<Intent> {
        if self.server_ending {
            let mut intents = self.sideline(
                player,
                "§eLa partie est terminée, retour au Hub FerrisCraft...",
            );
            intents.push(Intent::SendToLobby {
                player,
                reason: "partie terminée".to_owned(),
            });
            return intents;
        }
        if self.roster.contains(&player) {
            return vec![Intent::Tell {
                player,
                text: "§cVous êtes déjà dans la partie.".to_owned(),
            }];
        }
        match self.stage {
            Stage::Waiting | Stage::Countdown { .. } => {}
            Stage::Playing | Stage::Ended { .. } => {
                return self.sideline(
                    player,
                    "§eLa partie a déjà commencé, vous regardez en spectateur.",
                );
            }
        }
        if self.roster.len() >= self.rules.max_players {
            return self.sideline(
                player,
                "§cLa partie est complète, vous regardez en spectateur.",
            );
        }
        self.roster.push(player);
        let mut intents = vec![Intent::Enroll { player }, Intent::Lobby { player }];
        intents.extend(self.open());
        intents
    }

    pub fn watch(&mut self, player: PlayerId) -> Vec<Intent> {
        self.sideline(
            player,
            "§eVous regardez cette partie en spectateur. §d/hub §epour revenir au Hub.",
        )
    }

    pub fn leave(&mut self, player: PlayerId) -> Vec<Intent> {
        let Some(index) = self.roster.iter().position(|member| *member == player) else {
            return Vec::new();
        };
        self.roster.remove(index);
        let mut intents = vec![Intent::Unenroll { player }];
        match self.stage {
            Stage::Waiting => {}
            Stage::Countdown { .. } => {
                intents.extend(self.cancel_countdown());
            }
            Stage::Playing | Stage::Ended { .. } => {
                if let Some(log) = self.log.as_mut() {
                    log.left(player);
                }
                if let Some(game) = self.game.as_mut() {
                    game.forfeit(player);
                }
            }
        }
        intents
    }

    pub fn quit(&mut self, player: PlayerId) -> Vec<Intent> {
        if !self.roster.contains(&player) {
            return vec![Intent::Tell {
                player,
                text: "§cVous n'êtes pas dans la partie.".to_owned(),
            }];
        }
        let mut intents = self.leave(player);
        intents.push(Intent::Tell {
            player,
            text: "§eVous avez quitté la partie.".to_owned(),
        });
        intents.push(Intent::Enroll { player });
        intents.push(Intent::Lobby { player });
        intents
    }

    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        if self.server_ending {
            return tick;
        }
        let counting = self.reset_in.is_some();
        match self.stage {
            Stage::Countdown { remaining } if remaining <= 1 => {
                tick.intents.extend(self.start());
            }
            Stage::Countdown { remaining } => {
                self.stage = Stage::Countdown {
                    remaining: remaining - 1,
                };
            }
            Stage::Playing => self.advance(&mut tick),
            Stage::Waiting | Stage::Ended { .. } => {}
        }
        if counting && let Some(remaining) = self.reset_in {
            if remaining <= 1 {
                self.reset_in = None;
                self.delay_elapsed = true;
                if self.pending_report.is_none() {
                    tick.intents.extend(self.finish_end_delay());
                }
            } else {
                self.reset_in = Some(remaining - 1);
            }
        }
        tick
    }

    pub fn force_start(&mut self) -> Result<Vec<Intent>, ForceStartError> {
        if self.server_ending {
            return Err(ForceStartError::AlreadyStarted);
        }
        match self.stage {
            Stage::Waiting => {
                if self.roster.is_empty() {
                    return Err(ForceStartError::NoPlayers);
                }
            }
            Stage::Countdown { .. } => {}
            Stage::Playing | Stage::Ended { .. } => return Err(ForceStartError::AlreadyStarted),
        }
        self.forced_start = true;
        Ok(self.start())
    }

    pub fn report_settled(&mut self, match_id: Uuid) -> Vec<Intent> {
        if self.pending_report != Some(match_id) {
            return Vec::new();
        }
        self.pending_report = None;
        if self.delay_elapsed {
            self.finish_end_delay()
        } else {
            Vec::new()
        }
    }

    pub fn stop(&mut self) -> Vec<Intent> {
        self.reset("§cLa partie a été arrêtée.")
    }

    fn start(&mut self) -> Vec<Intent> {
        let mut seated = self.roster.clone();
        self.bots = bots_to_fill(self.rules.mode, seated.len());
        seated.extend(self.bots.iter().copied());
        let seats = assign_seats(self.rules.mode, &seated);
        let seed = self.rules.seed.unwrap_or_else(random_seed);
        match Match::new(self.rules.mode, seed, &self.map, &seats) {
            Ok(game) => {
                self.log = Some(MatchLog::start(
                    Uuid::new_v4(),
                    self.forced_start,
                    seats
                        .iter()
                        .filter(|seat| !self.bots.contains(&seat.player))
                        .map(|seat| (seat.player, seat.lane)),
                ));
                self.brains = seats
                    .iter()
                    .filter(|seat| self.bots.contains(&seat.player))
                    .map(|seat| Bot::new(&game, seat.player, seat.lane))
                    .collect();
                self.game = Some(game);
                self.stage = Stage::Playing;
                vec![Intent::Begin { seats }]
            }
            Err(error) => {
                self.stage = Stage::Waiting;
                self.forced_start = false;
                vec![Intent::Broadcast(format!(
                    "§cImpossible de démarrer la partie : {error}"
                ))]
            }
        }
    }

    fn advance(&mut self, tick: &mut Tick) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        game.tick();
        if game.elapsed() % BOT_TURN_TICKS == 0 {
            for bot in &mut self.brains {
                bot.act(game);
            }
        }
        let events = game.drain_events();
        let elapsed = game.elapsed();
        self.handle_events(&events, elapsed, tick);
        tick.events = events;
    }

    fn handle_events(&mut self, events: &[Event], elapsed: u64, tick: &mut Tick) {
        for event in events {
            match event {
                Event::LaneEliminated { lane } => {
                    if let Some(log) = self.log.as_mut() {
                        log.eliminated(*lane);
                    }
                }
                Event::Won { lane } => {
                    self.stage = Stage::Ended { winner: *lane };
                    self.reset_in = Some(self.rules.end_delay_ticks);
                    if lane.is_none() {
                        tick.intents.push(Intent::Broadcast(
                            "§eMatch nul : personne ne gagne.".to_owned(),
                        ));
                    }
                    tick.intents.push(Intent::Won { winner: *lane });
                    if let Some(log) = self.log.take() {
                        let summary = log.finish(*lane, elapsed);
                        if self.end_behavior == EndBehavior::EphemeralPod {
                            self.pending_report = Some(summary.match_id);
                        }
                        tick.intents.push(Intent::SubmitReport(summary));
                    }
                }
                _ => {}
            }
        }
    }

    fn finish_end_delay(&mut self) -> Vec<Intent> {
        match self.end_behavior {
            EndBehavior::EphemeralPod => {
                self.server_ending = true;
                self.reset_in = None;
                vec![
                    Intent::Broadcast("§eFin de partie — retour au Hub FerrisCraft...".to_owned()),
                    Intent::ReturnAllToLobby {
                        reason: "fin de partie".to_owned(),
                    },
                    Intent::EndServer,
                ]
            }
            EndBehavior::ResetRound { return_to_lobby } => {
                let mut intents = Vec::new();
                if return_to_lobby {
                    intents.push(Intent::ReturnAllToLobby {
                        reason: "fin de partie".to_owned(),
                    });
                }
                intents.extend(self.reset("§eUne nouvelle partie se prépare."));
                intents
            }
        }
    }

    fn reset(&mut self, text: &str) -> Vec<Intent> {
        self.stage = Stage::Waiting;
        self.roster.clear();
        self.game = None;
        self.reset_in = None;
        self.server_ending = false;
        self.log = None;
        self.forced_start = false;
        self.pending_report = None;
        self.delay_elapsed = false;
        self.bots.clear();
        self.brains.clear();
        self.expected = None;
        vec![Intent::Broadcast(text.to_owned()), Intent::Reset]
    }

    fn sideline(&self, player: PlayerId, text: &str) -> Vec<Intent> {
        vec![
            Intent::Tell {
                player,
                text: text.to_owned(),
            },
            Intent::Watch { player },
        ]
    }
}

#[cfg(test)]
mod tests {
    use ferriscraft_protocol::MatchOutcome;

    use super::*;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn rules() -> Rules {
        Rules {
            mode: GameMode::Solo,
            min_players: 4,
            max_players: 5,
            countdown_ticks: 3,
            end_delay_ticks: 3,
            seed: Some(11),
            needs_expected: false,
        }
    }

    fn session(behavior: EndBehavior) -> Session {
        Session::new(Map::test_map(), rules(), behavior)
    }

    fn joined(count: u128, behavior: EndBehavior) -> Session {
        let mut session = session(behavior);
        for index in 0..count {
            session.join(pid(index));
        }
        session
    }

    fn started(behavior: EndBehavior) -> Session {
        let mut session = joined(4, behavior);
        for _ in 0..3 {
            session.tick();
        }
        session
    }

    fn summaries(intents: &[Intent]) -> Vec<&super::super::report::MatchSummary> {
        intents
            .iter()
            .filter_map(|intent| match intent {
                Intent::SubmitReport(summary) => Some(summary),
                _ => None,
            })
            .collect()
    }

    fn end_by_forfeit(session: &mut Session) -> Vec<Intent> {
        for index in 1..4 {
            session.leave(pid(index));
        }
        session.tick().intents
    }

    #[test]
    fn the_countdown_starts_when_the_minimum_is_reached() {
        let mut session = session(EndBehavior::default());
        for index in 0..3 {
            session.join(pid(index));
            assert_eq!(session.server_phase(), ServerPhase::Waiting);
        }
        let intents = session.join(pid(3));
        assert_eq!(session.server_phase(), ServerPhase::Countdown);
        assert!(intents.contains(&Intent::Enroll { player: pid(3) }));
        assert!(intents.contains(&Intent::Lobby { player: pid(3) }));
    }

    #[test]
    fn leaving_during_the_countdown_cancels_it() {
        let mut session = joined(4, EndBehavior::default());
        session.leave(pid(0));
        assert_eq!(session.server_phase(), ServerPhase::Waiting);
        assert_eq!(session.stage(), Stage::Waiting);
    }

    #[test]
    fn joining_twice_is_refused() {
        let mut session = joined(1, EndBehavior::default());
        let intents = session.join(pid(0));
        assert!(matches!(intents.as_slice(), [Intent::Tell { .. }]));
        assert_eq!(session.roster().len(), 1);
    }

    #[test]
    fn a_full_session_sends_the_extra_player_to_watch() {
        let mut session = joined(5, EndBehavior::default());
        let intents = session.join(pid(5));
        assert!(intents.contains(&Intent::Watch { player: pid(5) }));
        assert_eq!(session.roster().len(), 5);
    }

    #[test]
    fn the_countdown_ends_with_a_seated_match() {
        let mut session = joined(4, EndBehavior::default());
        let mut begun = None;
        for _ in 0..3 {
            let tick = session.tick();
            for intent in tick.intents {
                if let Intent::Begin { seats } = intent {
                    begun = Some(seats);
                }
            }
        }
        let seats = begun.expect("match began");
        assert_eq!(seats.len(), 4);
        assert_eq!(session.server_phase(), ServerPhase::InGame);
        assert!(session.is_playing());
        assert_eq!(session.lane_of(pid(2)), Some(seats[2].lane));
        assert_eq!(session.game().map(Match::elapsed), Some(0));
    }

    #[test]
    fn ticking_advances_the_match_and_returns_its_events() {
        let mut session = started(EndBehavior::default());
        let mut paid = false;
        for _ in 0..120 {
            let tick = session.tick();
            paid |= tick
                .events
                .iter()
                .any(|event| matches!(event, Event::IncomePaid { .. }));
        }
        assert!(paid);
        assert_eq!(session.game().map(Match::elapsed), Some(120));
    }

    #[test]
    fn late_players_watch_instead_of_joining() {
        let mut session = started(EndBehavior::default());
        let intents = session.join(pid(9));
        assert!(intents.contains(&Intent::Watch { player: pid(9) }));
        assert!(!session.roster().contains(&pid(9)));
    }

    #[test]
    fn a_forced_start_needs_players_and_marks_the_report() {
        let mut session = session(EndBehavior::default());
        assert_eq!(session.force_start(), Err(ForceStartError::NoPlayers));
        session.join(pid(0));
        assert!(matches!(
            session.force_start().expect("forced").as_slice(),
            [Intent::Begin { seats }] if seats.len() == GameMode::Solo.min_players()
        ));
    }

    #[test]
    fn a_forced_start_is_reported_as_forced() {
        let mut session = joined(4, EndBehavior::default());
        let intents = session.force_start().expect("forced");
        assert!(matches!(intents.as_slice(), [Intent::Begin { .. }]));
        let ended = end_by_forfeit(&mut session);
        let summaries = summaries(&ended);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].forced_start);
    }

    #[test]
    fn a_forced_start_cannot_happen_twice() {
        let mut session = started(EndBehavior::default());
        assert_eq!(session.force_start(), Err(ForceStartError::AlreadyStarted));
    }

    #[test]
    fn a_normal_start_is_not_forced() {
        let mut session = started(EndBehavior::default());
        let ended = end_by_forfeit(&mut session);
        assert!(!summaries(&ended)[0].forced_start);
    }

    #[test]
    fn the_last_lane_standing_wins_and_the_report_says_who_completed() {
        let mut session = started(EndBehavior::default());
        let ended = end_by_forfeit(&mut session);
        assert!(ended.contains(&Intent::Won {
            winner: session.lane_of(pid(0))
        }));
        let report = summaries(&ended)[0].clone();
        assert_eq!(report.participants.len(), 4);
        let by_player = |player: PlayerId| {
            report
                .participants
                .iter()
                .find(|participant| participant.player == player)
                .map(|participant| (participant.outcome, participant.completed))
        };
        assert_eq!(by_player(pid(0)), Some((MatchOutcome::Win, true)));
        assert_eq!(by_player(pid(1)), Some((MatchOutcome::Loss, false)));
        assert_eq!(session.server_phase(), ServerPhase::InGame);
    }

    #[test]
    fn the_report_is_sent_once() {
        let mut session = started(EndBehavior::default());
        let first = summaries(&end_by_forfeit(&mut session)).len();
        let mut later = 0;
        for _ in 0..10 {
            later += summaries(&session.tick().intents).len();
        }
        assert_eq!((first, later), (1, 0));
    }

    #[test]
    fn after_the_end_delay_the_round_resets() {
        let mut session = started(EndBehavior::default());
        end_by_forfeit(&mut session);
        let mut reset = false;
        for _ in 0..5 {
            reset |= session.tick().intents.contains(&Intent::Reset);
        }
        assert!(reset);
        assert_eq!(session.server_phase(), ServerPhase::Waiting);
        assert!(session.roster().is_empty());
        assert!(session.game().is_none());
    }

    fn started_with_delay(delay: u32) -> Session {
        let rules = Rules {
            end_delay_ticks: delay,
            ..rules()
        };
        let mut session = Session::new(Map::test_map(), rules, EndBehavior::default());
        for index in 0..4 {
            session.join(pid(index));
        }
        for _ in 0..3 {
            session.tick();
        }
        session
    }

    fn ticks_to_reset(delay: u32) -> (bool, u32) {
        let mut session = started_with_delay(delay);
        let won = end_by_forfeit(&mut session);
        assert!(
            won.iter()
                .any(|intent| matches!(intent, Intent::Won { .. }))
        );
        let reset_on_won_tick = won.contains(&Intent::Reset);
        for after in 1..=delay + 5 {
            if session.tick().intents.contains(&Intent::Reset) {
                return (reset_on_won_tick, after);
            }
        }
        (reset_on_won_tick, 0)
    }

    #[test]
    fn the_end_delay_is_exact_for_one_tick() {
        assert_eq!(ticks_to_reset(1), (false, 1));
    }

    #[test]
    fn the_end_delay_is_exact_for_two_ticks() {
        assert_eq!(ticks_to_reset(2), (false, 2));
    }

    #[test]
    fn the_end_delay_is_exact_for_two_hundred_ticks() {
        assert_eq!(ticks_to_reset(200), (false, 200));
    }

    #[test]
    fn a_zero_end_delay_is_clamped_to_one_tick() {
        assert_eq!(ticks_to_reset(0), (false, 1));
    }

    #[test]
    fn a_draw_is_announced_and_reported_as_a_draw_for_everyone_who_stayed() {
        let mut session = started(EndBehavior::default());
        session.leave(pid(3));
        let mut tick = Tick::default();
        session.handle_events(&[Event::Won { lane: None }], 40, &mut tick);
        assert_eq!(session.stage(), Stage::Ended { winner: None });
        assert!(tick.intents.contains(&Intent::Won { winner: None }));
        assert!(
            tick.intents.iter().any(
                |intent| matches!(intent, Intent::Broadcast(text) if text.contains("Match nul"))
            )
        );
        let report = summaries(&tick.intents)[0].clone();
        assert_eq!(report.participants.len(), 4);
        assert!(
            report
                .participants
                .iter()
                .all(|participant| participant.outcome == MatchOutcome::Draw)
        );
        let left = report
            .participants
            .iter()
            .find(|participant| participant.player == pid(3))
            .map(|participant| participant.completed);
        assert_eq!(left, Some(false));
    }

    #[test]
    fn quitting_sends_the_player_to_the_lobby() {
        let mut session = started(EndBehavior::default());
        let intents = session.quit(pid(1));
        assert!(intents.contains(&Intent::Enroll { player: pid(1) }));
        assert!(intents.contains(&Intent::Lobby { player: pid(1) }));
        assert!(!intents.contains(&Intent::Watch { player: pid(1) }));
        assert!(!session.roster().contains(&pid(1)));
    }

    #[test]
    fn a_teammate_keeps_playing_when_their_partner_leaves() {
        let rules = Rules {
            mode: GameMode::Teams,
            ..rules()
        };
        let mut session = Session::new(Map::test_map(), rules, EndBehavior::default());
        for index in 0..4 {
            session.join(pid(index));
        }
        for _ in 0..3 {
            session.tick();
        }
        session.quit(pid(1));
        let ticks: Vec<Tick> = (0..5).map(|_| session.tick()).collect();
        assert!(ticks.iter().all(|tick| {
            !tick
                .intents
                .iter()
                .any(|intent| matches!(intent, Intent::Won { .. }))
        }));
        assert!(session.is_playing());
        let lane = session.lane_of(pid(0)).expect("seated");
        let alive = session.game().map(|game| game.lane(lane).hp.current() > 0);
        assert_eq!(alive, Some(true));
    }

    #[test]
    fn a_resetting_server_returns_everyone_to_the_lobby_when_asked() {
        let mut session = started(EndBehavior::ResetRound {
            return_to_lobby: true,
        });
        end_by_forfeit(&mut session);
        let mut sent = false;
        for _ in 0..5 {
            sent |= session
                .tick()
                .intents
                .iter()
                .any(|intent| matches!(intent, Intent::ReturnAllToLobby { .. }));
        }
        assert!(sent);
    }

    #[test]
    fn an_ephemeral_pod_waits_for_the_report_before_ending() {
        let mut session = started(EndBehavior::EphemeralPod);
        let ended = end_by_forfeit(&mut session);
        let match_id = summaries(&ended)[0].match_id;
        for _ in 0..10 {
            let tick = session.tick();
            assert!(!tick.intents.contains(&Intent::EndServer));
        }
        assert_eq!(session.server_phase(), ServerPhase::InGame);
        let intents = session.report_settled(match_id);
        assert!(intents.contains(&Intent::EndServer));
        assert_eq!(session.server_phase(), ServerPhase::Ending);
    }

    #[test]
    fn an_ephemeral_pod_ends_at_the_delay_when_the_report_settled_early() {
        let mut session = started(EndBehavior::EphemeralPod);
        let ended = end_by_forfeit(&mut session);
        let match_id = summaries(&ended)[0].match_id;
        assert!(session.report_settled(match_id).is_empty());
        let mut ending = false;
        for _ in 0..5 {
            ending |= session.tick().intents.contains(&Intent::EndServer);
        }
        assert!(ending);
    }

    #[test]
    fn an_unrelated_report_acknowledgement_changes_nothing() {
        let mut session = started(EndBehavior::EphemeralPod);
        end_by_forfeit(&mut session);
        assert!(session.report_settled(Uuid::nil()).is_empty());
    }

    #[test]
    fn a_player_joining_an_ending_server_is_sent_back_to_the_lobby() {
        let mut session = started(EndBehavior::EphemeralPod);
        let ended = end_by_forfeit(&mut session);
        let match_id = summaries(&ended)[0].match_id;
        for _ in 0..5 {
            session.tick();
        }
        session.report_settled(match_id);
        let intents = session.join(pid(8));
        assert!(
            intents
                .iter()
                .any(|intent| matches!(intent, Intent::SendToLobby { .. }))
        );
    }

    #[test]
    fn stopping_resets_without_a_report() {
        let mut session = started(EndBehavior::default());
        let intents = session.stop();
        assert!(intents.contains(&Intent::Reset));
        assert!(summaries(&intents).is_empty());
        assert_eq!(session.server_phase(), ServerPhase::Waiting);
    }

    #[test]
    fn a_teams_session_pairs_players_on_lanes() {
        let rules = Rules {
            mode: GameMode::Teams,
            ..rules()
        };
        let mut session = Session::new(Map::test_map(), rules, EndBehavior::default());
        for index in 0..4 {
            session.join(pid(index));
        }
        for _ in 0..3 {
            session.tick();
        }
        assert_eq!(session.lane_of(pid(0)), session.lane_of(pid(1)));
        assert_ne!(session.lane_of(pid(1)), session.lane_of(pid(2)));
    }

    fn orchestrated() -> Session {
        let rules = Rules {
            min_players: 1,
            needs_expected: true,
            ..rules()
        };
        Session::new(Map::test_map(), rules, EndBehavior::default())
    }

    fn begun(intents: &[Intent]) -> Option<&Vec<crate::sim::Seat>> {
        intents.iter().find_map(|intent| match intent {
            Intent::Begin { seats } => Some(seats),
            _ => None,
        })
    }

    #[test]
    fn solo_expected_starts_at_once_and_fills_the_other_lanes_with_bots() {
        let mut session = orchestrated();
        session.set_expected(Some(1));
        let intents = session.join(pid(0));
        let seats = begun(&intents).expect("begin");
        assert_eq!(seats.len(), GameMode::Solo.min_players());
        assert_eq!(seats[0].player, pid(0));
        assert_eq!(session.stage(), Stage::Playing);
        assert!(session.game().is_some());
    }

    #[test]
    fn expected_arriving_after_the_player_starts_at_once() {
        let mut session = orchestrated();
        assert!(begun(&session.join(pid(0))).is_none());
        assert_eq!(session.stage(), Stage::Waiting);
        assert!(begun(&session.set_expected(Some(1))).is_some());
        assert_eq!(session.stage(), Stage::Playing);
    }

    #[test]
    fn a_duo_waits_for_the_second_player_then_counts_down() {
        let mut session = orchestrated();
        session.set_expected(Some(2));
        session.join(pid(0));
        assert_eq!(session.stage(), Stage::Waiting);
        session.join(pid(1));
        assert!(matches!(session.stage(), Stage::Countdown { .. }));
        let mut seats = 0;
        for _ in 0..3 {
            seats += begun(&session.tick().intents).map_or(0, Vec::len);
        }
        assert_eq!(seats, GameMode::Solo.min_players());
        assert_eq!(session.stage(), Stage::Playing);
    }

    #[test]
    fn the_countdown_cancels_when_the_roster_drops_below_expected() {
        let mut session = orchestrated();
        session.set_expected(Some(2));
        session.join(pid(0));
        session.join(pid(1));
        session.leave(pid(1));
        assert_eq!(session.stage(), Stage::Waiting);
    }

    #[test]
    fn without_a_known_expected_an_orchestrated_session_never_starts() {
        let mut session = orchestrated();
        session.join(pid(0));
        session.set_expected(None);
        for _ in 0..5 {
            session.tick();
        }
        assert_eq!(session.stage(), Stage::Waiting);
        assert!(session.game().is_none());
    }

    #[test]
    fn a_none_response_keeps_the_previous_expected_and_a_reset_clears_it() {
        let mut session = orchestrated();
        session.set_expected(Some(3));
        session.set_expected(None);
        assert_eq!(session.expected(), Some(3));
        session.stop();
        assert_eq!(session.expected(), None);
    }

    #[test]
    fn without_an_orchestrator_the_threshold_is_min_players() {
        let rules = Rules {
            min_players: 1,
            ..rules()
        };
        let mut session = Session::new(Map::test_map(), rules, EndBehavior::default());
        assert!(begun(&session.join(pid(0))).is_some());
    }

    #[test]
    fn bots_send_mobs_at_the_human_so_a_solo_match_is_not_empty() {
        let mut session = orchestrated();
        session.set_expected(Some(1));
        session.join(pid(0));
        let mut sent_by_bot = false;
        let mut reached_human_lane = false;
        for _ in 0..(BOT_TURN_TICKS * 2) {
            for event in session.tick().events {
                match event {
                    Event::MobSent { sender, .. } if sender != pid(0) => sent_by_bot = true,
                    Event::MobSpawned { lane, .. } if lane.get() == 0 => reached_human_lane = true,
                    _ => {}
                }
            }
        }
        assert!(sent_by_bot);
        assert!(reached_human_lane);
    }

    #[test]
    fn bots_only_fill_the_seats_missing_to_the_mode_minimum() {
        let min = GameMode::Solo.min_players();
        assert_eq!(bots_to_fill(GameMode::Solo, 1).len(), min - 1);
        assert!(bots_to_fill(GameMode::Solo, min).is_empty());
    }

    #[test]
    fn a_single_player_below_min_players_never_starts() {
        let mut session = session(EndBehavior::default());
        session.join(pid(0));
        for _ in 0..5 {
            session.tick();
        }
        assert!(session.game().is_none());
    }
}
