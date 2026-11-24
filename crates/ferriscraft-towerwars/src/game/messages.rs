use crate::domain::SimError;

pub fn message(error: &SimError) -> &'static str {
    match error {
        SimError::InvalidLane(_) => "§cVoie invalide.",
        SimError::UnknownPlayer => "§cVous ne jouez pas cette partie.",
        SimError::NotEnoughGold => "§cPas assez d'or.",
        SimError::LockedTier => "§cCe tier de mobs n'est pas encore débloqué.",
        SimError::NoCharge => "§cPlus de charge pour ce mob, patientez.",
        SimError::NotSendable => "§cCe mob ne peut pas être envoyé.",
        SimError::LaneFull => "§cVotre voie a atteint la limite de tours.",
        SimError::OccupiedCell => "§cCette case est déjà occupée.",
        SimError::OutOfBuildBounds => "§cOn ne peut pas construire ici.",
        SimError::NotOwner => "§cCette tour n'est pas à vous.",
        SimError::UnknownTower => "§cCette tour n'existe plus.",
        SimError::MaxLevel => "§cCette tour est déjà au niveau maximal.",
        SimError::BranchChoice => "§cChoisissez une spécialisation.",
        SimError::GoldMineLimit => "§cVous avez atteint la limite de mines d'or.",
        SimError::LaneEliminated => "§cVotre voie est éliminée.",
        SimError::PlayerLeft => "§cVous avez quitté la partie.",
        SimError::MatchEnded => "§cLa partie est terminée.",
        SimError::NotEnoughPlayers => "§cJoueurs insuffisants pour ce mode.",
        SimError::InvalidSeating => "§cRépartition des joueurs invalide.",
        SimError::LanesSaturated => "§cLes voies adverses sont pleines, patientez.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_a_short_red_french_sentence() {
        let errors = [
            SimError::InvalidLane(9),
            SimError::UnknownPlayer,
            SimError::NotEnoughGold,
            SimError::LockedTier,
            SimError::NoCharge,
            SimError::NotSendable,
            SimError::LaneFull,
            SimError::OccupiedCell,
            SimError::OutOfBuildBounds,
            SimError::NotOwner,
            SimError::UnknownTower,
            SimError::MaxLevel,
            SimError::BranchChoice,
            SimError::GoldMineLimit,
            SimError::LaneEliminated,
            SimError::PlayerLeft,
            SimError::MatchEnded,
            SimError::NotEnoughPlayers,
            SimError::InvalidSeating,
            SimError::LanesSaturated,
        ];
        for error in errors {
            let text = message(&error);
            assert!(text.starts_with("§c") && text.ends_with('.'), "{text}");
            assert!(text.chars().count() < 60, "{text}");
        }
    }

    #[test]
    fn gold_and_locked_tier_are_distinguishable() {
        assert_ne!(
            message(&SimError::NotEnoughGold),
            message(&SimError::LockedTier)
        );
        assert!(message(&SimError::NotEnoughGold).contains("or"));
    }
}
