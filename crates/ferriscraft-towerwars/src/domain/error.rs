use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimError {
    #[error("voie invalide : {0}")]
    InvalidLane(u8),
    #[error("joueur inconnu")]
    UnknownPlayer,
    #[error("pas assez d'or")]
    NotEnoughGold,
    #[error("tier de mob pas encore débloqué")]
    LockedTier,
    #[error("plus de charge pour ce mob")]
    NoCharge,
    #[error("ce mob ne peut pas être envoyé")]
    NotSendable,
    #[error("limite de tours atteinte sur la voie")]
    LaneFull,
    #[error("case déjà occupée")]
    OccupiedCell,
    #[error("case hors de la zone constructible")]
    OutOfBuildBounds,
    #[error("cette tour n'appartient pas à votre voie")]
    NotOwner,
    #[error("tour inconnue")]
    UnknownTower,
    #[error("niveau maximal atteint")]
    MaxLevel,
    #[error("choix de spécialisation requis ou invalide")]
    BranchChoice,
    #[error("limite de mines d'or atteinte")]
    GoldMineLimit,
    #[error("voie éliminée")]
    LaneEliminated,
    #[error("joueur parti de la partie")]
    PlayerLeft,
    #[error("partie terminée")]
    MatchEnded,
    #[error("joueurs insuffisants pour ce mode")]
    NotEnoughPlayers,
    #[error("répartition des joueurs invalide")]
    InvalidSeating,
    #[error("toutes les voies adverses sont saturées")]
    LanesSaturated,
}
