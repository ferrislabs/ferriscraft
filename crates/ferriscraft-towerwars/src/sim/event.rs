use crate::domain::{LaneId, Level, MobId, MobKind, PlayerId, TowerId, TowerKind, Vec3};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    IncomePaid {
        player: PlayerId,
        amount: u32,
    },
    TierUnlocked {
        tier: u8,
    },
    SuddenDeath {
        level: u32,
    },
    MobSent {
        sender: PlayerId,
        kind: MobKind,
        count: u8,
    },
    MobSpawned {
        lane: LaneId,
        mob: MobId,
        kind: MobKind,
    },
    MobKilled {
        lane: LaneId,
        mob: MobId,
        kind: MobKind,
        bounty: u32,
    },
    MobLeaked {
        lane: LaneId,
        mob: MobId,
        kind: MobKind,
        damage: u16,
    },
    MobSplit {
        lane: LaneId,
        parent: MobId,
        children: [MobId; 2],
    },
    MobBlinked {
        lane: LaneId,
        mob: MobId,
    },
    LaneDamaged {
        lane: LaneId,
        hp: u16,
    },
    LaneHealed {
        lane: LaneId,
        hp: u16,
    },
    LaneEliminated {
        lane: LaneId,
    },
    TowerPlaced {
        lane: LaneId,
        tower: TowerId,
        kind: TowerKind,
        cell: (i32, i32),
    },
    TowerUpgraded {
        lane: LaneId,
        tower: TowerId,
        level: Level,
    },
    TowerSold {
        lane: LaneId,
        tower: TowerId,
        refund: u32,
    },
    TowerFired {
        lane: LaneId,
        tower: TowerId,
        from: Vec3,
        to: Vec3,
    },
    TowerStunned {
        lane: LaneId,
        tower: TowerId,
        ticks: u16,
    },
    Explosion {
        lane: LaneId,
        at: Vec3,
        radius: f32,
    },
    Chain {
        lane: LaneId,
        points: Vec<Vec3>,
    },
    Won {
        lane: Option<LaneId>,
    },
}
