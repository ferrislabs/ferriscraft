use super::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TowerKind {
    Archer,
    Ice,
    Mage,
    Cannon,
    Tesla,
    Slime,
    Beacon,
}

impl TowerKind {
    pub const ALL: [Self; 7] = [
        Self::Archer,
        Self::Ice,
        Self::Mage,
        Self::Cannon,
        Self::Tesla,
        Self::Slime,
        Self::Beacon,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Branch {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    T1,
    T2,
    T3,
    T4(Branch),
}

impl Level {
    pub const fn rank(self) -> u8 {
        match self {
            Self::T1 => 1,
            Self::T2 => 2,
            Self::T3 => 3,
            Self::T4(_) => 4,
        }
    }

    pub const fn next(self, branch: Option<Branch>) -> Result<Self, SimError> {
        match (self, branch) {
            (Self::T1, None) => Ok(Self::T2),
            (Self::T2, None) => Ok(Self::T3),
            (Self::T3, Some(branch)) => Ok(Self::T4(branch)),
            (Self::T4(_), _) => Err(SimError::MaxLevel),
            _ => Err(SimError::BranchChoice),
        }
    }

    pub const fn history(self) -> &'static [Self] {
        match self {
            Self::T1 => &[Self::T1],
            Self::T2 => &[Self::T1, Self::T2],
            Self::T3 => &[Self::T1, Self::T2, Self::T3],
            Self::T4(Branch::A) => &[Self::T1, Self::T2, Self::T3, Self::T4(Branch::A)],
            Self::T4(Branch::B) => &[Self::T1, Self::T2, Self::T3, Self::T4(Branch::B)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    #[default]
    First,
    Last,
    Strongest,
    Weakest,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TowerEffect {
    None,
    Crit {
        chance: f32,
        multiplier: f32,
    },
    Slow {
        percent: f32,
        ticks: u16,
        vulnerability: f32,
    },
    Aura {
        dps: f32,
        freeze_chance: f32,
        freeze_ticks: u16,
        freeze_immunity_ticks: u16,
    },
    Burn {
        dps: f32,
        ticks: u16,
    },
    Pyro {
        radius: f32,
        dps: f32,
        ticks: u16,
    },
    Curse {
        burst_fraction: f32,
        radius: f32,
    },
    Splash {
        radius: f32,
    },
    Cluster {
        radius: f32,
        submunitions: u8,
        damage: f32,
        sub_radius: f32,
    },
    Chain {
        targets: u8,
        bounce: f32,
        stun_every: Option<(u8, u16)>,
    },
    Corrode {
        armor: f32,
        ticks: u16,
    },
    Pool {
        radius: f32,
        ticks: u16,
        dps: f32,
        armor: f32,
    },
    Mutator {
        fraction: f32,
    },
    Beacon {
        attack_speed: f32,
        range: f32,
        damage: f32,
        radius: f32,
    },
    GoldMine {
        income: u32,
        attack_speed: f32,
        radius: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TowerStats {
    pub cost: u32,
    pub damage: f32,
    pub cooldown_ticks: u16,
    pub range: f32,
    pub effect: TowerEffect,
}
