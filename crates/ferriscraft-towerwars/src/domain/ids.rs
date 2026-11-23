use uuid::Uuid;

use super::SimError;

pub const LANE_COUNT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LaneId(u8);

impl LaneId {
    pub const ALL: [Self; LANE_COUNT] = [Self(0), Self(1), Self(2), Self(3), Self(4), Self(5)];

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<u8> for LaneId {
    type Error = SimError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if usize::from(value) < LANE_COUNT {
            Ok(Self(value))
        } else {
            Err(SimError::InvalidLane(value))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MobId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TowerId(pub u32);
