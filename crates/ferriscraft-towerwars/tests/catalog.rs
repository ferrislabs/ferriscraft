use ferriscraft_towerwars::domain::{
    Branch, Level, MobKind, SENDABLE_MOBS, TowerEffect, TowerKind,
};

struct MobRow {
    kind: MobKind,
    cost: u32,
    income: u32,
    hp: f32,
    speed: f32,
    damage: u16,
    tier: u8,
    armor: f32,
    slow_resist: f32,
    dodge: f32,
    flags: [bool; 3],
}

struct TowerRow {
    kind: TowerKind,
    level: Level,
    cost: u32,
    damage: f32,
    cooldown: u16,
    range: f32,
    effect: TowerEffect,
}

const MOBS: [MobRow; 17] = [
    MobRow {
        kind: MobKind::Silverfish,
        cost: 15,
        income: 2,
        hp: 35.0,
        speed: 3.2,
        damage: 1,
        tier: 1,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Zombie,
        cost: 30,
        income: 4,
        hp: 95.0,
        speed: 2.4,
        damage: 1,
        tier: 1,
        armor: 0.1,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Endermite,
        cost: 50,
        income: 5,
        hp: 65.0,
        speed: 4.6,
        damage: 1,
        tier: 1,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Skeleton,
        cost: 80,
        income: 9,
        hp: 180.0,
        speed: 2.7,
        damage: 1,
        tier: 1,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.15,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Spider,
        cost: 120,
        income: 13,
        hp: 220.0,
        speed: 3.8,
        damage: 1,
        tier: 2,
        armor: 0.0,
        slow_resist: 0.5,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Creeper,
        cost: 200,
        income: 21,
        hp: 420.0,
        speed: 2.5,
        damage: 2,
        tier: 2,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Blaze,
        cost: 320,
        income: 32,
        hp: 550.0,
        speed: 3.0,
        damage: 2,
        tier: 2,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [true, false, false],
    },
    MobRow {
        kind: MobKind::Witch,
        cost: 450,
        income: 44,
        hp: 680.0,
        speed: 2.6,
        damage: 2,
        tier: 2,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::GiantSlime,
        cost: 700,
        income: 65,
        hp: 1100.0,
        speed: 2.2,
        damage: 2,
        tier: 3,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::PiglinBrute,
        cost: 1000,
        income: 90,
        hp: 1850.0,
        speed: 2.8,
        damage: 3,
        tier: 3,
        armor: 0.25,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Enderman,
        cost: 1450,
        income: 125,
        hp: 1600.0,
        speed: 3.5,
        damage: 3,
        tier: 3,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Ghast,
        cost: 2000,
        income: 170,
        hp: 2400.0,
        speed: 2.4,
        damage: 3,
        tier: 3,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, true],
    },
    MobRow {
        kind: MobKind::IronGolem,
        cost: 3000,
        income: 240,
        hp: 5500.0,
        speed: 1.9,
        damage: 4,
        tier: 4,
        armor: 0.4,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, true, false],
    },
    MobRow {
        kind: MobKind::Ravager,
        cost: 4500,
        income: 340,
        hp: 7800.0,
        speed: 2.6,
        damage: 5,
        tier: 4,
        armor: 0.0,
        slow_resist: 0.5,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Warden,
        cost: 7000,
        income: 500,
        hp: 13500.0,
        speed: 2.1,
        damage: 8,
        tier: 4,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::Wither,
        cost: 12000,
        income: 800,
        hp: 22000.0,
        speed: 2.5,
        damage: 15,
        tier: 4,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
    MobRow {
        kind: MobKind::MediumSlime,
        cost: 0,
        income: 0,
        hp: 300.0,
        speed: 2.8,
        damage: 2,
        tier: 3,
        armor: 0.0,
        slow_resist: 0.0,
        dodge: 0.0,
        flags: [false, false, false],
    },
];

const TOWERS: [TowerRow; 35] = [
    TowerRow {
        kind: TowerKind::Archer,
        level: Level::T1,
        cost: 50,
        damage: 12.0,
        cooldown: 20,
        range: 8.0,
        effect: TowerEffect::None,
    },
    TowerRow {
        kind: TowerKind::Archer,
        level: Level::T2,
        cost: 100,
        damage: 26.0,
        cooldown: 18,
        range: 9.0,
        effect: TowerEffect::None,
    },
    TowerRow {
        kind: TowerKind::Archer,
        level: Level::T3,
        cost: 250,
        damage: 60.0,
        cooldown: 16,
        range: 10.0,
        effect: TowerEffect::None,
    },
    TowerRow {
        kind: TowerKind::Archer,
        level: Level::T4(Branch::A),
        cost: 600,
        damage: 180.0,
        cooldown: 24,
        range: 14.0,
        effect: TowerEffect::Crit {
            chance: 0.25,
            multiplier: 2.5,
        },
    },
    TowerRow {
        kind: TowerKind::Archer,
        level: Level::T4(Branch::B),
        cost: 600,
        damage: 45.0,
        cooldown: 5,
        range: 9.0,
        effect: TowerEffect::None,
    },
    TowerRow {
        kind: TowerKind::Ice,
        level: Level::T1,
        cost: 75,
        damage: 5.0,
        cooldown: 30,
        range: 3.5,
        effect: TowerEffect::Slow {
            percent: 0.25,
            ticks: 50,
            vulnerability: 0.0,
        },
    },
    TowerRow {
        kind: TowerKind::Ice,
        level: Level::T2,
        cost: 150,
        damage: 12.0,
        cooldown: 28,
        range: 4.0,
        effect: TowerEffect::Slow {
            percent: 0.35,
            ticks: 60,
            vulnerability: 0.0,
        },
    },
    TowerRow {
        kind: TowerKind::Ice,
        level: Level::T3,
        cost: 350,
        damage: 25.0,
        cooldown: 26,
        range: 4.5,
        effect: TowerEffect::Slow {
            percent: 0.45,
            ticks: 70,
            vulnerability: 0.0,
        },
    },
    TowerRow {
        kind: TowerKind::Ice,
        level: Level::T4(Branch::A),
        cost: 800,
        damage: 25.0,
        cooldown: 26,
        range: 4.5,
        effect: TowerEffect::Slow {
            percent: 0.6,
            ticks: 80,
            vulnerability: 0.15,
        },
    },
    TowerRow {
        kind: TowerKind::Ice,
        level: Level::T4(Branch::B),
        cost: 800,
        damage: 0.0,
        cooldown: 20,
        range: 4.5,
        effect: TowerEffect::Aura {
            dps: 65.0,
            freeze_chance: 0.10,
            freeze_ticks: 15,
            freeze_immunity_ticks: 80,
        },
    },
    TowerRow {
        kind: TowerKind::Mage,
        level: Level::T1,
        cost: 100,
        damage: 15.0,
        cooldown: 24,
        range: 8.0,
        effect: TowerEffect::Burn {
            dps: 5.0,
            ticks: 60,
        },
    },
    TowerRow {
        kind: TowerKind::Mage,
        level: Level::T2,
        cost: 220,
        damage: 35.0,
        cooldown: 22,
        range: 9.0,
        effect: TowerEffect::Burn {
            dps: 12.0,
            ticks: 60,
        },
    },
    TowerRow {
        kind: TowerKind::Mage,
        level: Level::T3,
        cost: 480,
        damage: 80.0,
        cooldown: 20,
        range: 10.0,
        effect: TowerEffect::Burn {
            dps: 25.0,
            ticks: 80,
        },
    },
    TowerRow {
        kind: TowerKind::Mage,
        level: Level::T4(Branch::A),
        cost: 1100,
        damage: 80.0,
        cooldown: 20,
        range: 10.0,
        effect: TowerEffect::Pyro {
            radius: 2.5,
            dps: 45.0,
            ticks: 80,
        },
    },
    TowerRow {
        kind: TowerKind::Mage,
        level: Level::T4(Branch::B),
        cost: 1100,
        damage: 80.0,
        cooldown: 20,
        range: 10.0,
        effect: TowerEffect::Curse {
            burst_fraction: 0.30,
            radius: 3.0,
        },
    },
    TowerRow {
        kind: TowerKind::Cannon,
        level: Level::T1,
        cost: 125,
        damage: 28.0,
        cooldown: 40,
        range: 9.0,
        effect: TowerEffect::Splash { radius: 2.5 },
    },
    TowerRow {
        kind: TowerKind::Cannon,
        level: Level::T2,
        cost: 275,
        damage: 65.0,
        cooldown: 38,
        range: 10.0,
        effect: TowerEffect::Splash { radius: 3.0 },
    },
    TowerRow {
        kind: TowerKind::Cannon,
        level: Level::T3,
        cost: 600,
        damage: 150.0,
        cooldown: 36,
        range: 11.0,
        effect: TowerEffect::Splash { radius: 3.5 },
    },
    TowerRow {
        kind: TowerKind::Cannon,
        level: Level::T4(Branch::A),
        cost: 1400,
        damage: 480.0,
        cooldown: 50,
        range: 15.0,
        effect: TowerEffect::Splash { radius: 4.5 },
    },
    TowerRow {
        kind: TowerKind::Cannon,
        level: Level::T4(Branch::B),
        cost: 1400,
        damage: 150.0,
        cooldown: 36,
        range: 11.0,
        effect: TowerEffect::Cluster {
            radius: 3.5,
            submunitions: 4,
            damage: 110.0,
            sub_radius: 2.0,
        },
    },
    TowerRow {
        kind: TowerKind::Tesla,
        level: Level::T1,
        cost: 150,
        damage: 20.0,
        cooldown: 22,
        range: 7.5,
        effect: TowerEffect::Chain {
            targets: 3,
            bounce: 3.5,
            stun_every: None,
        },
    },
    TowerRow {
        kind: TowerKind::Tesla,
        level: Level::T2,
        cost: 320,
        damage: 45.0,
        cooldown: 20,
        range: 8.5,
        effect: TowerEffect::Chain {
            targets: 4,
            bounce: 4.0,
            stun_every: None,
        },
    },
    TowerRow {
        kind: TowerKind::Tesla,
        level: Level::T3,
        cost: 700,
        damage: 100.0,
        cooldown: 18,
        range: 9.5,
        effect: TowerEffect::Chain {
            targets: 5,
            bounce: 4.5,
            stun_every: None,
        },
    },
    TowerRow {
        kind: TowerKind::Tesla,
        level: Level::T4(Branch::A),
        cost: 1600,
        damage: 190.0,
        cooldown: 18,
        range: 9.5,
        effect: TowerEffect::Chain {
            targets: 10,
            bounce: 4.5,
            stun_every: None,
        },
    },
    TowerRow {
        kind: TowerKind::Tesla,
        level: Level::T4(Branch::B),
        cost: 1600,
        damage: 320.0,
        cooldown: 18,
        range: 9.5,
        effect: TowerEffect::Chain {
            targets: 4,
            bounce: 4.5,
            stun_every: Some((3, 8)),
        },
    },
    TowerRow {
        kind: TowerKind::Slime,
        level: Level::T1,
        cost: 120,
        damage: 14.0,
        cooldown: 26,
        range: 7.5,
        effect: TowerEffect::Corrode {
            armor: 0.15,
            ticks: 80,
        },
    },
    TowerRow {
        kind: TowerKind::Slime,
        level: Level::T2,
        cost: 260,
        damage: 32.0,
        cooldown: 24,
        range: 8.5,
        effect: TowerEffect::Corrode {
            armor: 0.25,
            ticks: 80,
        },
    },
    TowerRow {
        kind: TowerKind::Slime,
        level: Level::T3,
        cost: 550,
        damage: 75.0,
        cooldown: 22,
        range: 9.5,
        effect: TowerEffect::Corrode {
            armor: 0.35,
            ticks: 100,
        },
    },
    TowerRow {
        kind: TowerKind::Slime,
        level: Level::T4(Branch::A),
        cost: 1250,
        damage: 75.0,
        cooldown: 22,
        range: 9.5,
        effect: TowerEffect::Pool {
            radius: 3.0,
            ticks: 80,
            dps: 60.0,
            armor: 0.50,
        },
    },
    TowerRow {
        kind: TowerKind::Slime,
        level: Level::T4(Branch::B),
        cost: 1250,
        damage: 75.0,
        cooldown: 22,
        range: 9.5,
        effect: TowerEffect::Mutator { fraction: 0.025 },
    },
    TowerRow {
        kind: TowerKind::Beacon,
        level: Level::T1,
        cost: 200,
        damage: 0.0,
        cooldown: 20,
        range: 0.0,
        effect: TowerEffect::Beacon {
            attack_speed: 0.1,
            range: 0.0,
            damage: 0.0,
            radius: 4.5,
        },
    },
    TowerRow {
        kind: TowerKind::Beacon,
        level: Level::T2,
        cost: 450,
        damage: 0.0,
        cooldown: 20,
        range: 0.0,
        effect: TowerEffect::Beacon {
            attack_speed: 0.18,
            range: 0.1,
            damage: 0.0,
            radius: 5.0,
        },
    },
    TowerRow {
        kind: TowerKind::Beacon,
        level: Level::T3,
        cost: 950,
        damage: 0.0,
        cooldown: 20,
        range: 0.0,
        effect: TowerEffect::Beacon {
            attack_speed: 0.25,
            range: 0.15,
            damage: 0.0,
            radius: 5.5,
        },
    },
    TowerRow {
        kind: TowerKind::Beacon,
        level: Level::T4(Branch::A),
        cost: 2000,
        damage: 0.0,
        cooldown: 20,
        range: 0.0,
        effect: TowerEffect::Beacon {
            attack_speed: 0.4,
            range: 0.0,
            damage: 0.2,
            radius: 6.0,
        },
    },
    TowerRow {
        kind: TowerKind::Beacon,
        level: Level::T4(Branch::B),
        cost: 2000,
        damage: 0.0,
        cooldown: 20,
        range: 0.0,
        effect: TowerEffect::GoldMine {
            income: 15,
            attack_speed: 0.15,
            radius: 5.5,
        },
    },
];

#[test]
fn every_mob_matches_the_design_document() {
    for row in &MOBS {
        let spec = row.kind.spec();
        let label = format!("{:?}", row.kind);
        assert_eq!(spec.cost, row.cost, "{label} cost");
        assert_eq!(spec.income, row.income, "{label} income");
        assert_eq!(spec.hp, row.hp, "{label} hp");
        assert_eq!(spec.speed, row.speed, "{label} speed");
        assert_eq!(spec.damage, row.damage, "{label} damage");
        assert_eq!(spec.tier, row.tier, "{label} tier");
        assert_eq!(spec.armor, row.armor, "{label} armor");
        assert_eq!(spec.slow_resist, row.slow_resist, "{label} slow_resist");
        assert_eq!(spec.dodge, row.dodge, "{label} dodge");
        let flags = [spec.fire_immune, spec.stun_immune, spec.flying];
        assert_eq!(flags, row.flags, "{label} flags");
    }
}

#[test]
fn the_mob_table_covers_all_sixteen_sendable_kinds_and_the_medium_slime() {
    assert_eq!(MOBS.len(), SENDABLE_MOBS + 1);
    for kind in MobKind::SENDABLE {
        assert!(MOBS.iter().any(|row| row.kind == kind), "{kind:?}");
    }
    assert!(MOBS.iter().any(|row| row.kind == MobKind::MediumSlime));
}

#[test]
fn every_tower_kind_and_level_matches_the_design_document() {
    for row in &TOWERS {
        let stats = row.kind.stats(row.level);
        let label = format!("{:?} {:?}", row.kind, row.level);
        assert_eq!(stats.cost, row.cost, "{label} cost");
        assert_eq!(stats.damage, row.damage, "{label} damage");
        assert_eq!(stats.cooldown_ticks, row.cooldown, "{label} cooldown");
        assert_eq!(stats.range, row.range, "{label} range");
        assert_eq!(stats.effect, row.effect, "{label} effect");
    }
}

#[test]
fn the_tower_table_has_every_kind_at_every_level_exactly_once() {
    let levels = [
        Level::T1,
        Level::T2,
        Level::T3,
        Level::T4(Branch::A),
        Level::T4(Branch::B),
    ];
    assert_eq!(TOWERS.len(), TowerKind::ALL.len() * levels.len());
    for kind in TowerKind::ALL {
        for level in levels {
            let found = TOWERS
                .iter()
                .filter(|row| row.kind == kind && row.level == level)
                .count();
            assert_eq!(found, 1, "{kind:?} {level:?}");
        }
    }
}
