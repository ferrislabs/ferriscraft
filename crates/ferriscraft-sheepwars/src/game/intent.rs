use super::report::RoundSummary;
use crate::arena::Point;
use crate::domain::{PlayerId, SheepKind, TeamId};

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Broadcast(String),
    Tell { player: PlayerId, text: String },
    Title { text: String, subtitle: String },
    Celebrate { team: TeamId },
    Enroll { player: PlayerId, team: TeamId },
    Unenroll { player: PlayerId, team: TeamId },
    SyncTeams,
    Teleport { player: PlayerId, to: Point },
    ClearInventory { player: PlayerId },
    GiveStarterKit { player: PlayerId },
    Give { player: PlayerId, kind: SheepKind },
    Respawn { player: PlayerId },
    Spectate { player: PlayerId },
    ReturnAllToLobby { reason: String },
    SendToLobby { player: PlayerId, reason: String },
    EndServer,
    SubmitReport(RoundSummary),
    Reset,
}
