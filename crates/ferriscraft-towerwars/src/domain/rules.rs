pub const TICKS_PER_SECOND: u32 = 20;
pub const STARTING_GOLD: u32 = 100;
pub const STARTING_INCOME: u32 = 10;
pub const INCOME_CYCLE_TICKS: u64 = 120;
pub const TOWER_LIMIT: usize = 80;
pub const SELL_REFUND_PERCENT: u32 = 80;
pub const BOUNTY_PERCENT: u32 = 12;
pub const STOCK_MAX: u8 = 10;
pub const STOCK_RECHARGE_TICKS: u16 = 30;
pub const SEND_STAGGER_TICKS: u64 = 2;
pub const SUDDEN_DEATH_TICKS: u64 = 18_000;
pub const SUDDEN_DEATH_STEP_TICKS: u64 = 1_200;
pub const SUDDEN_DEATH_SPEED_STEP: f32 = 0.25;
pub const SUDDEN_DEATH_HP_STEP: f32 = 0.5;
pub const MAX_BEACONS_PER_TOWER: usize = 2;
pub const MAX_GOLD_MINES_PER_PLAYER: usize = 4;
pub const MAX_ARMOR: f32 = 0.9;
pub const TIER_UNLOCK_TICKS: [u64; 4] = [0, 3_000, 6_600, 10_800];
pub const MAX_INCOME: u32 = 10_000;
pub const MAX_MOBS_PER_LANE: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Solo,
    Teams,
}

impl GameMode {
    pub const fn starting_hp(self) -> u16 {
        match self {
            Self::Solo => 20,
            Self::Teams => 25,
        }
    }

    pub const fn owners_per_lane(self) -> usize {
        match self {
            Self::Solo => 1,
            Self::Teams => 2,
        }
    }

    pub const fn min_players(self) -> usize {
        match self {
            Self::Solo => 4,
            Self::Teams => 4,
        }
    }

    pub const fn max_players(self) -> usize {
        match self {
            Self::Solo => 6,
            Self::Teams => 12,
        }
    }
}
