use super::report::MatchSummary;
use crate::domain::{LaneId, PlayerId};
use crate::sim::Seat;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Broadcast(String),
    Tell { player: PlayerId, text: String },
    Enroll { player: PlayerId },
    Unenroll { player: PlayerId },
    Lobby { player: PlayerId },
    Watch { player: PlayerId },
    Begin { seats: Vec<Seat> },
    Won { winner: Option<LaneId> },
    ReturnAllToLobby { reason: String },
    SendToLobby { player: PlayerId, reason: String },
    EndServer,
    SubmitReport(MatchSummary),
    Reset,
}
