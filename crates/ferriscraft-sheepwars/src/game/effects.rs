use crate::domain::{SheepKind, TeamId};
use crate::sheep::{Action, Vec3};

#[derive(Debug, Default)]
pub struct Effects;

impl Effects {
    pub fn new() -> Self {
        Self
    }

    pub fn start(&mut self, _kind: SheepKind, _at: Vec3, _team: TeamId) {}

    pub fn tick(&mut self) -> Vec<Action> {
        Vec::new()
    }

    pub fn is_idle(&self) -> bool {
        true
    }
}
