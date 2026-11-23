mod abilities;
mod attacks;
mod damage;
mod effects;
mod statuses;

#[cfg(test)]
mod tests;

use crate::domain::LaneId;

use super::lane::Mob;
use super::world::World;

pub(crate) fn step_statuses(world: &mut World) {
    statuses::run(world);
}

pub(crate) fn step_towers(world: &mut World) {
    attacks::run(world);
}

pub(crate) fn step_abilities(world: &mut World) {
    abilities::run(world);
}

pub(crate) fn on_death(world: &mut World, lane: LaneId, mob: &Mob) {
    abilities::on_death(world, lane, mob);
}
