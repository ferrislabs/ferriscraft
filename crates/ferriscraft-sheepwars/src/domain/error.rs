use thiserror::Error;

use super::PlayerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RoundError {
    #[error("la partie a déjà commencé")]
    AlreadyStarted,
    #[error("la partie est complète")]
    Full,
    #[error("le joueur {0:?} est déjà dans la partie")]
    AlreadyJoined(PlayerId),
}
