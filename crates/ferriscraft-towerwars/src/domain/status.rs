#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slow {
    pub percent: f32,
    pub remaining: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burn {
    pub dps: f32,
    pub remaining: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorShred {
    pub percent: f32,
    pub remaining: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vulnerability {
    pub percent: f32,
    pub remaining: u16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statuses {
    pub slow: Option<Slow>,
    pub burns: Vec<Burn>,
    pub shred: Option<ArmorShred>,
    pub vulnerability: Option<Vulnerability>,
    pub stun_remaining: u16,
    pub freeze_immunity_remaining: u16,
    pub cursed: bool,
}

pub const MAX_BURN_STACKS: usize = 3;
