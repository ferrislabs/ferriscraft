use crate::domain::{Level, MobKind, Priority, TICKS_PER_SECOND, TowerEffect, TowerKind};

pub const fn mob_name(kind: MobKind) -> &'static str {
    match kind {
        MobKind::Silverfish => "Poisson d'argent",
        MobKind::Zombie => "Zombie",
        MobKind::Endermite => "Endermite",
        MobKind::Skeleton => "Squelette",
        MobKind::Spider => "Araignée",
        MobKind::Creeper => "Creeper",
        MobKind::Blaze => "Blaze",
        MobKind::Witch => "Sorcière",
        MobKind::GiantSlime => "Slime géant",
        MobKind::PiglinBrute => "Piglin brute",
        MobKind::Enderman => "Enderman",
        MobKind::Ghast => "Ghast",
        MobKind::IronGolem => "Golem de fer",
        MobKind::Ravager => "Ravageur",
        MobKind::Warden => "Warden",
        MobKind::Wither => "Wither",
        MobKind::MediumSlime => "Slime moyen",
    }
}

pub const fn tower_name(kind: TowerKind) -> &'static str {
    match kind {
        TowerKind::Archer => "Archer",
        TowerKind::Ice => "Glace",
        TowerKind::Mage => "Mage",
        TowerKind::Cannon => "Canon",
        TowerKind::Tesla => "Tesla",
        TowerKind::Slime => "Slime",
        TowerKind::Beacon => "Balise",
    }
}

pub const fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::First => "Premier",
        Priority::Last => "Dernier",
        Priority::Strongest => "Plus fort",
        Priority::Weakest => "Plus faible",
    }
}

pub const fn next_priority(priority: Priority) -> Priority {
    match priority {
        Priority::First => Priority::Last,
        Priority::Last => Priority::Strongest,
        Priority::Strongest => Priority::Weakest,
        Priority::Weakest => Priority::First,
    }
}

pub fn level_name(level: Level) -> String {
    match level {
        Level::T4(branch) => format!("T4-{branch:?}"),
        other => format!("T{}", other.rank()),
    }
}

fn seconds(ticks: u16) -> f32 {
    f32::from(ticks) / TICKS_PER_SECOND as f32
}

pub fn effect_text(effect: TowerEffect) -> Option<String> {
    Some(match effect {
        TowerEffect::None => return None,
        TowerEffect::Crit { chance, multiplier } => {
            format!("Critique {:.0}% (x{multiplier:.1})", chance * 100.0)
        }
        TowerEffect::Slow {
            percent,
            ticks,
            vulnerability,
        } => {
            let base = format!(
                "Ralentit de {:.0}% ({:.1}s)",
                percent * 100.0,
                seconds(ticks)
            );
            if vulnerability > 0.0 {
                format!("{base}, +{:.0}% de dégâts subis", vulnerability * 100.0)
            } else {
                base
            }
        }
        TowerEffect::Aura {
            dps,
            freeze_chance,
            freeze_ticks,
            ..
        } => format!(
            "Aura {dps:.0} dégâts/s, gel {:.0}% ({:.1}s)",
            freeze_chance * 100.0,
            seconds(freeze_ticks)
        ),
        TowerEffect::Burn { dps, ticks } => {
            format!("Brûlure {dps:.0}/s pendant {:.1}s", seconds(ticks))
        }
        TowerEffect::Pyro { radius, dps, ticks } => format!(
            "Brûlure de zone {dps:.0}/s ({:.1}s, rayon {radius:.1})",
            seconds(ticks)
        ),
        TowerEffect::Curse {
            burst_fraction,
            radius,
        } => format!(
            "Malédiction : explosion de {:.0}% des PV (rayon {radius:.1})",
            burst_fraction * 100.0
        ),
        TowerEffect::Splash { radius } => format!("Dégâts de zone (rayon {radius:.1})"),
        TowerEffect::Cluster {
            submunitions,
            radius,
            ..
        } => format!("{submunitions} sous-munitions (rayon {radius:.1})"),
        TowerEffect::Chain {
            targets, bounce, ..
        } => format!("Chaîne sur {targets} cibles (rebond {bounce:.1})"),
        TowerEffect::Corrode { armor, ticks } => format!(
            "Réduit l'armure de {:.0}% ({:.1}s)",
            armor * 100.0,
            seconds(ticks)
        ),
        TowerEffect::Pool {
            radius, ticks, dps, ..
        } => format!(
            "Flaque {dps:.0}/s ({:.1}s, rayon {radius:.1})",
            seconds(ticks)
        ),
        TowerEffect::Mutator { fraction } => format!("Mutation de {:.0}%", fraction * 100.0),
        TowerEffect::Beacon {
            attack_speed,
            range,
            ..
        } => format!(
            "+{:.0}% de vitesse d'attaque alliée (portée {range:.1})",
            attack_speed * 100.0
        ),
        TowerEffect::GoldMine { income, .. } => format!("Mine d'or : +{income} or par cycle"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Branch;

    #[test]
    fn priorities_cycle_through_all_four() {
        let mut priority = Priority::First;
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(priority);
            priority = next_priority(priority);
        }
        assert_eq!(priority, Priority::First);
        seen.sort_by_key(|p| *p as u8);
        seen.dedup();
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn levels_are_named_by_rank_and_branch() {
        assert_eq!(level_name(Level::T2), "T2");
        assert_eq!(level_name(Level::T4(Branch::B)), "T4-B");
    }

    #[test]
    fn every_catalogue_effect_has_a_description_or_none() {
        for kind in TowerKind::ALL {
            for level in [
                Level::T1,
                Level::T2,
                Level::T3,
                Level::T4(Branch::A),
                Level::T4(Branch::B),
            ] {
                let effect = kind.stats(level).effect;
                assert_eq!(
                    effect_text(effect).is_none(),
                    matches!(effect, TowerEffect::None)
                );
            }
        }
    }

    #[test]
    fn ice_describes_its_slow() {
        let text = effect_text(TowerKind::Ice.stats(Level::T1).effect).expect("text");
        assert!(text.contains("25%") && text.contains("2.5s"), "{text}");
    }

    #[test]
    fn every_mob_and_tower_has_a_name() {
        for kind in MobKind::SENDABLE {
            assert!(!mob_name(kind).is_empty());
        }
        for kind in TowerKind::ALL {
            assert!(!tower_name(kind).is_empty());
        }
    }
}
