#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Gold(u32);

impl Gold {
    pub const fn new(amount: u32) -> Self {
        Self(amount)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn checked_sub(self, cost: u32) -> Option<Self> {
        match self.0.checked_sub(cost) {
            Some(rest) => Some(Self(rest)),
            None => None,
        }
    }

    pub const fn saturating_add(self, amount: u32) -> Self {
        Self(self.0.saturating_add(amount))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneHp {
    current: u16,
    cap: u16,
}

impl LaneHp {
    pub const fn full(cap: u16) -> Self {
        Self { current: cap, cap }
    }

    pub const fn current(self) -> u16 {
        self.current
    }

    pub const fn cap(self) -> u16 {
        self.cap
    }

    pub const fn is_dead(self) -> bool {
        self.current == 0
    }

    pub fn damage(&mut self, amount: u16) {
        self.current = self.current.saturating_sub(amount);
    }

    pub fn heal_to_cap(&mut self, amount: u16) {
        self.current = self.current.saturating_add(amount).min(self.cap);
    }
}
