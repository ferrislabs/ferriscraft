use crate::domain::{MAX_ARMOR, PlayerId};
use crate::sim::lane::Mob;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DamageKind {
    Physical,
    Magic,
}

pub(super) fn apply_damage(mob: &mut Mob, amount: f32, kind: DamageKind, source: Option<PlayerId>) {
    let dealt = amount * armor_factor(mob, kind) * vulnerability_factor(mob);
    mob.hp -= dealt;
    if source.is_some() {
        mob.last_hit_by = source;
    }
}

fn armor_factor(mob: &Mob, kind: DamageKind) -> f32 {
    match kind {
        DamageKind::Magic => 1.0,
        DamageKind::Physical => {
            let shred = mob.statuses.shred.map_or(0.0, |shred| shred.percent);
            1.0 - (mob.kind.spec().armor + shred).min(MAX_ARMOR)
        }
    }
}

fn vulnerability_factor(mob: &Mob) -> f32 {
    1.0 + mob
        .statuses
        .vulnerability
        .map_or(0.0, |vulnerability| vulnerability.percent)
}
