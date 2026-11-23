use super::{Branch, Level, TowerEffect, TowerKind, TowerStats};

const fn stats(
    cost: u32,
    damage: f32,
    cooldown_ticks: u16,
    range: f32,
    effect: TowerEffect,
) -> TowerStats {
    TowerStats {
        cost,
        damage,
        cooldown_ticks,
        range,
        effect,
    }
}

impl TowerKind {
    pub fn stats(self, level: Level) -> TowerStats {
        match self {
            Self::Archer => archer(level),
            Self::Ice => ice(level),
            Self::Mage => mage(level),
            Self::Cannon => cannon(level),
            Self::Tesla => tesla(level),
            Self::Slime => slime(level),
            Self::Beacon => beacon(level),
        }
    }
}

const fn archer(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(50, 12.0, 20, 8.0, TowerEffect::None),
        Level::T2 => stats(100, 26.0, 18, 9.0, TowerEffect::None),
        Level::T3 => stats(250, 60.0, 16, 10.0, TowerEffect::None),
        Level::T4(Branch::A) => stats(
            600,
            180.0,
            24,
            14.0,
            TowerEffect::Crit {
                chance: 0.25,
                multiplier: 2.5,
            },
        ),
        Level::T4(Branch::B) => stats(600, 45.0, 5, 9.0, TowerEffect::None),
    }
}

const fn slow(percent: f32, ticks: u16, vulnerability: f32) -> TowerEffect {
    TowerEffect::Slow {
        percent,
        ticks,
        vulnerability,
    }
}

const fn ice(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(75, 5.0, 30, 3.5, slow(0.25, 50, 0.0)),
        Level::T2 => stats(150, 12.0, 28, 4.0, slow(0.35, 60, 0.0)),
        Level::T3 => stats(350, 25.0, 26, 4.5, slow(0.45, 70, 0.0)),
        Level::T4(Branch::A) => stats(800, 25.0, 26, 4.5, slow(0.60, 80, 0.15)),
        Level::T4(Branch::B) => stats(
            800,
            0.0,
            20,
            4.5,
            TowerEffect::Aura {
                dps: 65.0,
                freeze_chance: 0.10,
                freeze_ticks: 15,
                freeze_immunity_ticks: 80,
            },
        ),
    }
}

const fn burn(dps: f32, ticks: u16) -> TowerEffect {
    TowerEffect::Burn { dps, ticks }
}

const fn mage(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(100, 15.0, 24, 8.0, burn(5.0, 60)),
        Level::T2 => stats(220, 35.0, 22, 9.0, burn(12.0, 60)),
        Level::T3 => stats(480, 80.0, 20, 10.0, burn(25.0, 80)),
        Level::T4(Branch::A) => stats(
            1100,
            80.0,
            20,
            10.0,
            TowerEffect::Pyro {
                radius: 2.5,
                dps: 45.0,
                ticks: 80,
            },
        ),
        Level::T4(Branch::B) => stats(
            1100,
            80.0,
            20,
            10.0,
            TowerEffect::Curse {
                burst_fraction: 0.30,
                radius: 3.0,
            },
        ),
    }
}

const fn splash(radius: f32) -> TowerEffect {
    TowerEffect::Splash { radius }
}

const fn cannon(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(125, 28.0, 40, 9.0, splash(2.5)),
        Level::T2 => stats(275, 65.0, 38, 10.0, splash(3.0)),
        Level::T3 => stats(600, 150.0, 36, 11.0, splash(3.5)),
        Level::T4(Branch::A) => stats(1400, 480.0, 50, 15.0, splash(4.5)),
        Level::T4(Branch::B) => stats(
            1400,
            150.0,
            36,
            11.0,
            TowerEffect::Cluster {
                radius: 3.5,
                submunitions: 4,
                damage: 110.0,
                sub_radius: 2.0,
            },
        ),
    }
}

const fn chain(targets: u8, bounce: f32, stun_every: Option<(u8, u16)>) -> TowerEffect {
    TowerEffect::Chain {
        targets,
        bounce,
        stun_every,
    }
}

const fn tesla(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(150, 20.0, 22, 7.5, chain(3, 3.5, None)),
        Level::T2 => stats(320, 45.0, 20, 8.5, chain(4, 4.0, None)),
        Level::T3 => stats(700, 100.0, 18, 9.5, chain(5, 4.5, None)),
        Level::T4(Branch::A) => stats(1600, 190.0, 18, 9.5, chain(10, 4.5, None)),
        Level::T4(Branch::B) => stats(1600, 320.0, 18, 9.5, chain(4, 4.5, Some((3, 8)))),
    }
}

const fn corrode(armor: f32, ticks: u16) -> TowerEffect {
    TowerEffect::Corrode { armor, ticks }
}

const fn slime(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(120, 14.0, 26, 7.5, corrode(0.15, 80)),
        Level::T2 => stats(260, 32.0, 24, 8.5, corrode(0.25, 80)),
        Level::T3 => stats(550, 75.0, 22, 9.5, corrode(0.35, 100)),
        Level::T4(Branch::A) => stats(
            1250,
            75.0,
            22,
            9.5,
            TowerEffect::Pool {
                radius: 3.0,
                ticks: 80,
                dps: 60.0,
                armor: 0.50,
            },
        ),
        Level::T4(Branch::B) => stats(
            1250,
            75.0,
            22,
            9.5,
            TowerEffect::Mutator { fraction: 0.025 },
        ),
    }
}

const fn aura(attack_speed: f32, range: f32, damage: f32, radius: f32) -> TowerEffect {
    TowerEffect::Beacon {
        attack_speed,
        range,
        damage,
        radius,
    }
}

const fn beacon(level: Level) -> TowerStats {
    match level {
        Level::T1 => stats(200, 0.0, 20, 0.0, aura(0.10, 0.0, 0.0, 4.5)),
        Level::T2 => stats(450, 0.0, 20, 0.0, aura(0.18, 0.10, 0.0, 5.0)),
        Level::T3 => stats(950, 0.0, 20, 0.0, aura(0.25, 0.15, 0.0, 5.5)),
        Level::T4(Branch::A) => stats(2000, 0.0, 20, 0.0, aura(0.40, 0.0, 0.20, 6.0)),
        Level::T4(Branch::B) => stats(
            2000,
            0.0,
            20,
            0.0,
            TowerEffect::GoldMine {
                income: 15,
                attack_speed: 0.15,
                radius: 5.5,
            },
        ),
    }
}
