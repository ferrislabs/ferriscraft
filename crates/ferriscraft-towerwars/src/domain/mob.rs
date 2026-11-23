#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MobKind {
    Silverfish,
    Zombie,
    Endermite,
    Skeleton,
    Spider,
    Creeper,
    Blaze,
    Witch,
    GiantSlime,
    PiglinBrute,
    Enderman,
    Ghast,
    IronGolem,
    Ravager,
    Warden,
    Wither,
    MediumSlime,
}

pub const SENDABLE_MOBS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobSpec {
    pub cost: u32,
    pub income: u32,
    pub hp: f32,
    pub speed: f32,
    pub damage: u16,
    pub tier: u8,
    pub armor: f32,
    pub slow_resist: f32,
    pub dodge: f32,
    pub fire_immune: bool,
    pub stun_immune: bool,
    pub flying: bool,
}

const BASE: MobSpec = MobSpec {
    cost: 0,
    income: 0,
    hp: 0.0,
    speed: 0.0,
    damage: 1,
    tier: 1,
    armor: 0.0,
    slow_resist: 0.0,
    dodge: 0.0,
    fire_immune: false,
    stun_immune: false,
    flying: false,
};

impl MobKind {
    pub const SENDABLE: [Self; SENDABLE_MOBS] = [
        Self::Silverfish,
        Self::Zombie,
        Self::Endermite,
        Self::Skeleton,
        Self::Spider,
        Self::Creeper,
        Self::Blaze,
        Self::Witch,
        Self::GiantSlime,
        Self::PiglinBrute,
        Self::Enderman,
        Self::Ghast,
        Self::IronGolem,
        Self::Ravager,
        Self::Warden,
        Self::Wither,
    ];

    pub const fn is_sendable(self) -> bool {
        !matches!(self, Self::MediumSlime)
    }

    pub fn stock_index(self) -> Option<usize> {
        Self::SENDABLE.iter().position(|kind| *kind == self)
    }

    pub const fn spec(self) -> MobSpec {
        match self {
            Self::Silverfish => MobSpec {
                cost: 15,
                income: 2,
                hp: 35.0,
                speed: 3.2,
                ..BASE
            },
            Self::Zombie => MobSpec {
                cost: 30,
                income: 4,
                hp: 95.0,
                speed: 2.4,
                armor: 0.10,
                ..BASE
            },
            Self::Endermite => MobSpec {
                cost: 50,
                income: 5,
                hp: 65.0,
                speed: 4.6,
                ..BASE
            },
            Self::Skeleton => MobSpec {
                cost: 80,
                income: 9,
                hp: 180.0,
                speed: 2.7,
                dodge: 0.15,
                ..BASE
            },
            Self::Spider => MobSpec {
                cost: 120,
                income: 13,
                hp: 220.0,
                speed: 3.8,
                tier: 2,
                slow_resist: 0.5,
                ..BASE
            },
            Self::Creeper => MobSpec {
                cost: 200,
                income: 21,
                hp: 420.0,
                speed: 2.5,
                damage: 2,
                tier: 2,
                ..BASE
            },
            Self::Blaze => MobSpec {
                cost: 320,
                income: 32,
                hp: 550.0,
                speed: 3.0,
                damage: 2,
                tier: 2,
                fire_immune: true,
                ..BASE
            },
            Self::Witch => MobSpec {
                cost: 450,
                income: 44,
                hp: 680.0,
                speed: 2.6,
                damage: 2,
                tier: 2,
                ..BASE
            },
            Self::GiantSlime => MobSpec {
                cost: 700,
                income: 65,
                hp: 1100.0,
                speed: 2.2,
                damage: 2,
                tier: 3,
                ..BASE
            },
            Self::PiglinBrute => MobSpec {
                cost: 1000,
                income: 90,
                hp: 1850.0,
                speed: 2.8,
                damage: 3,
                tier: 3,
                armor: 0.25,
                ..BASE
            },
            Self::Enderman => MobSpec {
                cost: 1450,
                income: 125,
                hp: 1600.0,
                speed: 3.5,
                damage: 3,
                tier: 3,
                ..BASE
            },
            Self::Ghast => MobSpec {
                cost: 2000,
                income: 170,
                hp: 2400.0,
                speed: 2.4,
                damage: 3,
                tier: 3,
                flying: true,
                ..BASE
            },
            Self::IronGolem => MobSpec {
                cost: 3000,
                income: 240,
                hp: 5500.0,
                speed: 1.9,
                damage: 4,
                tier: 4,
                armor: 0.40,
                stun_immune: true,
                ..BASE
            },
            Self::Ravager => MobSpec {
                cost: 4500,
                income: 340,
                hp: 7800.0,
                speed: 2.6,
                damage: 5,
                tier: 4,
                slow_resist: 0.5,
                ..BASE
            },
            Self::Warden => MobSpec {
                cost: 7000,
                income: 500,
                hp: 13500.0,
                speed: 2.1,
                damage: 8,
                tier: 4,
                ..BASE
            },
            Self::Wither => MobSpec {
                cost: 12000,
                income: 800,
                hp: 22000.0,
                speed: 2.5,
                damage: 15,
                tier: 4,
                ..BASE
            },
            Self::MediumSlime => MobSpec {
                hp: 300.0,
                speed: 2.8,
                damage: 2,
                tier: 3,
                ..BASE
            },
        }
    }
}
