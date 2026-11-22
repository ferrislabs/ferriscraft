use ferriscraft_protocol::CosmeticId;
use uuid::Uuid;

use crate::amount::Coins;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoinReason {
    Match(Uuid),
    Cosmetic(CosmeticId),
    Admin(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoinDelta {
    Credit(Coins),
    Debit(Coins),
}

impl CoinDelta {
    #[must_use]
    pub const fn signed(self) -> i128 {
        match self {
            Self::Credit(coins) => coins.get() as i128,
            Self::Debit(coins) => -(coins.get() as i128),
        }
    }
}

#[must_use = "a coin movement must be written to the ledger"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoinMovement {
    pub delta: CoinDelta,
    pub reason: CoinReason,
}
