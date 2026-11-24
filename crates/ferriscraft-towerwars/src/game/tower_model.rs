use rspigot_plugin::{BlockType, Transformation};

use crate::domain::{Branch, Level, TowerKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    pub block: BlockType,
    pub from: [f32; 3],
    pub size: [f32; 3],
    pub turret: bool,
}

impl Part {
    const fn fixed(block: BlockType, from: [f32; 3], size: [f32; 3]) -> Self {
        Self {
            block,
            from,
            size,
            turret: false,
        }
    }

    const fn turret(block: BlockType, from: [f32; 3], size: [f32; 3]) -> Self {
        Self {
            block,
            from,
            size,
            turret: true,
        }
    }

    pub fn top(&self) -> f32 {
        self.from[1] + self.size[1]
    }

    pub fn transformation(&self, yaw: f32) -> Transformation {
        let offset = [self.from[0] - 0.5, self.from[1], self.from[2] - 0.5];
        let yaw = if self.turret { yaw } else { 0.0 };
        let [qx, qy, qz, qw] = yaw_rotation(yaw);
        let [tx, ty, tz] = rotate(offset, yaw);
        Transformation::default()
            .translation(tx, ty, tz)
            .left_rotation(qx, qy, qz, qw)
            .scale(self.size[0], self.size[1], self.size[2])
    }
}

pub fn yaw_rotation(yaw: f32) -> [f32; 4] {
    let half = (-yaw).to_radians() / 2.0;
    [0.0, half.sin(), 0.0, half.cos()]
}

fn rotate(point: [f32; 3], yaw: f32) -> [f32; 3] {
    let (sin, cos) = (-yaw).to_radians().sin_cos();
    [
        point[0] * cos + point[2] * sin,
        point[1],
        -point[0] * sin + point[2] * cos,
    ]
}

const fn rank(level: Level) -> f32 {
    match level {
        Level::T1 => 1.0,
        Level::T2 => 2.0,
        Level::T3 => 3.0,
        Level::T4(_) => 4.0,
    }
}

const fn branch(level: Level) -> Option<Branch> {
    match level {
        Level::T4(branch) => Some(branch),
        _ => None,
    }
}

pub fn tower_model(kind: TowerKind, level: Level) -> Vec<Part> {
    match kind {
        TowerKind::Archer => archer(level),
        TowerKind::Ice => ice(level),
        TowerKind::Mage => mage(level),
        TowerKind::Cannon => cannon(level),
        TowerKind::Tesla => tesla(level),
        TowerKind::Slime => slime(level),
        TowerKind::Beacon => beacon(level),
    }
}

pub fn model_height(kind: TowerKind, level: Level) -> f32 {
    tower_model(kind, level)
        .iter()
        .map(Part::top)
        .fold(0.0, f32::max)
}

fn corners(block: BlockType, y: f32, side: f32, height: f32, inset: f32) -> [Part; 4] {
    let far = 1.0 - inset - side;
    [
        Part::fixed(block, [inset, y, inset], [side, height, side]),
        Part::fixed(block, [far, y, inset], [side, height, side]),
        Part::fixed(block, [inset, y, far], [side, height, side]),
        Part::fixed(block, [far, y, far], [side, height, side]),
    ]
}

fn archer(level: Level) -> Vec<Part> {
    let n = rank(level);
    let legs = 0.3 + 0.15 * n;
    let deck = 0.2 + legs;
    let mut parts = vec![Part::fixed(
        BlockType::StoneBricks,
        [0.05, 0.0, 0.05],
        [0.9, 0.2, 0.9],
    )];
    parts.extend(corners(BlockType::SpruceLog, 0.2, 0.14, legs, 0.12));
    parts.push(Part::fixed(
        BlockType::SprucePlanks,
        [0.05, deck, 0.05],
        [0.9, 0.1, 0.9],
    ));
    if n >= 3.0 {
        parts.extend(corners(
            BlockType::StoneBricks,
            deck + 0.1,
            0.16,
            0.16,
            0.05,
        ));
    }
    let head = match branch(level) {
        Some(Branch::A) => BlockType::Target,
        Some(Branch::B) => BlockType::Observer,
        None => BlockType::Dispenser,
    };
    parts.push(Part::turret(head, [0.3, deck + 0.1, 0.3], [0.4, 0.4, 0.4]));
    if branch(level) == Some(Branch::B) {
        parts.push(Part::turret(
            BlockType::CoalBlock,
            [0.43, deck + 0.24, 0.7],
            [0.14, 0.12, 0.3],
        ));
    }
    parts
}

fn ice(level: Level) -> Vec<Part> {
    let n = rank(level);
    let spire = 0.4 + 0.2 * n;
    let mut parts = vec![
        Part::fixed(BlockType::PackedIce, [0.08, 0.0, 0.08], [0.84, 0.2, 0.84]),
        Part::fixed(BlockType::BlueIce, [0.32, 0.2, 0.32], [0.36, spire, 0.36]),
    ];
    let shard = 0.25 + 0.1 * n;
    if n >= 2.0 {
        parts.push(Part::fixed(
            BlockType::Ice,
            [0.12, 0.2, 0.4],
            [0.16, shard, 0.16],
        ));
        parts.push(Part::fixed(
            BlockType::Ice,
            [0.72, 0.2, 0.44],
            [0.16, shard * 0.8, 0.16],
        ));
    }
    if n >= 3.0 {
        parts.push(Part::fixed(
            BlockType::Ice,
            [0.42, 0.2, 0.12],
            [0.16, shard * 0.7, 0.16],
        ));
        parts.push(Part::fixed(
            BlockType::Ice,
            [0.4, 0.2, 0.72],
            [0.16, shard * 0.9, 0.16],
        ));
    }
    let crown = match branch(level) {
        Some(Branch::A) => BlockType::SnowBlock,
        Some(Branch::B) => BlockType::WhiteStainedGlass,
        None => BlockType::LightBlueStainedGlass,
    };
    parts.push(Part::turret(
        crown,
        [0.3, 0.3 + spire, 0.3],
        [0.4, 0.3, 0.4],
    ));
    if branch(level) == Some(Branch::B) {
        parts.extend(corners(
            BlockType::LightBlueStainedGlass,
            0.5 + spire,
            0.12,
            0.12,
            0.0,
        ));
    }
    parts
}

fn mage(level: Level) -> Vec<Part> {
    let n = rank(level);
    let pillar = 0.4 + 0.2 * n;
    let mut parts = vec![
        Part::fixed(
            BlockType::PolishedBlackstoneBricks,
            [0.1, 0.0, 0.1],
            [0.8, 0.2, 0.8],
        ),
        Part::fixed(
            BlockType::PurpurPillar,
            [0.32, 0.2, 0.32],
            [0.36, pillar, 0.36],
        ),
    ];
    if n >= 2.0 {
        parts.extend(corners(BlockType::BuddingAmethyst, 0.2, 0.14, 0.2, 0.12));
    }
    if n >= 3.0 {
        parts.push(Part::fixed(
            BlockType::CryingObsidian,
            [0.25, 0.2 + pillar, 0.25],
            [0.5, 0.08, 0.5],
        ));
    }
    let orb = match branch(level) {
        Some(Branch::A) => BlockType::MagmaBlock,
        Some(Branch::B) => BlockType::SoulLantern,
        None => BlockType::AmethystBlock,
    };
    parts.push(Part::turret(
        orb,
        [0.33, 0.4 + pillar, 0.33],
        [0.34, 0.34, 0.34],
    ));
    parts
}

fn cannon(level: Level) -> Vec<Part> {
    let n = rank(level);
    let big = branch(level) == Some(Branch::A);
    let mut parts = vec![
        Part::fixed(
            BlockType::PolishedDeepslate,
            [0.05, 0.0, 0.05],
            [0.9, 0.3, 0.9],
        ),
        Part::turret(
            BlockType::PolishedBlackstone,
            [0.22, 0.3, 0.22],
            [0.56, 0.32, 0.56],
        ),
    ];
    let bore = if big { 0.3 } else { 0.2 };
    let length = 0.35 + 0.08 * n;
    parts.push(Part::turret(
        BlockType::CoalBlock,
        [0.5 - bore / 2.0, 0.42, 0.5],
        [bore, bore, length],
    ));
    if n >= 2.0 {
        parts.push(Part::turret(
            BlockType::IronBlock,
            [0.5 - bore / 2.0 - 0.03, 0.39, 0.5 + length - 0.12],
            [bore + 0.06, bore + 0.06, 0.08],
        ));
    }
    if n >= 3.0 {
        parts.push(Part::fixed(
            BlockType::DarkOakLog,
            [0.0, 0.08, 0.3],
            [0.08, 0.4, 0.4],
        ));
        parts.push(Part::fixed(
            BlockType::DarkOakLog,
            [0.92, 0.08, 0.3],
            [0.08, 0.4, 0.4],
        ));
    }
    if branch(level) == Some(Branch::B) {
        parts.push(Part::turret(
            BlockType::Tnt,
            [0.36, 0.62, 0.18],
            [0.28, 0.28, 0.28],
        ));
    }
    parts
}

fn tesla(level: Level) -> Vec<Part> {
    let n = rank(level);
    let mut parts = vec![Part::fixed(
        BlockType::CutCopper,
        [0.1, 0.0, 0.1],
        [0.8, 0.25, 0.8],
    )];
    let mut y = 0.25;
    for ring in 0..(n as usize + 1) {
        let block = if ring % 2 == 0 {
            BlockType::CopperBlock
        } else {
            BlockType::ExposedCopper
        };
        parts.push(Part::fixed(block, [0.3, y, 0.3], [0.4, 0.14, 0.4]));
        y += 0.14;
        parts.push(Part::fixed(
            BlockType::CutCopper,
            [0.38, y, 0.38],
            [0.24, 0.06, 0.24],
        ));
        y += 0.06;
    }
    parts.push(Part::fixed(
        BlockType::LightningRod,
        [0.25, y, 0.25],
        [0.5, 0.6, 0.5],
    ));
    let orb = match branch(level) {
        Some(Branch::A) => Some((BlockType::SeaLantern, 0.3)),
        Some(Branch::B) => Some((BlockType::RedstoneBlock, 0.24)),
        None => Some((BlockType::SeaLantern, 0.1 + 0.03 * n)),
    };
    if let Some((block, side)) = orb {
        let from = 0.5 - side / 2.0;
        parts.push(Part::turret(
            block,
            [from, y + 0.55, from],
            [side, side, side],
        ));
    }
    parts
}

fn slime(level: Level) -> Vec<Part> {
    let n = rank(level);
    let vat = 0.3 + 0.15 * n;
    let shell = match branch(level) {
        Some(Branch::B) => BlockType::LimeStainedGlass,
        _ => BlockType::Glass,
    };
    let mut parts = vec![
        Part::fixed(
            BlockType::MossyCobblestone,
            [0.05, 0.0, 0.05],
            [0.9, 0.22, 0.9],
        ),
        Part::fixed(
            BlockType::SlimeBlock,
            [0.2, 0.22, 0.2],
            [0.6, vat * 0.75, 0.6],
        ),
        Part::fixed(shell, [0.15, 0.22, 0.15], [0.7, vat, 0.7]),
    ];
    if n >= 2.0 {
        parts.push(Part::fixed(
            BlockType::GreenConcrete,
            [0.84, 0.22, 0.44],
            [0.1, vat + 0.15, 0.12],
        ));
    }
    let lid = match branch(level) {
        Some(Branch::A) => Some(BlockType::VerdantFroglight),
        _ if n >= 3.0 => Some(BlockType::HoneyBlock),
        _ => None,
    };
    if let Some(block) = lid {
        parts.push(Part::turret(block, [0.3, 0.22 + vat, 0.3], [0.4, 0.2, 0.4]));
    }
    let spout_y = 0.22 + vat + if lid.is_some() { 0.2 } else { 0.0 };
    parts.push(Part::turret(
        BlockType::GreenConcrete,
        [0.44, spout_y, 0.44],
        [0.12, 0.1, 0.5],
    ));
    parts
}

fn beacon(level: Level) -> Vec<Part> {
    let n = rank(level);
    let tiers: [BlockType; 3] = match branch(level) {
        Some(Branch::A) => [
            BlockType::IronBlock,
            BlockType::EmeraldBlock,
            BlockType::EmeraldBlock,
        ],
        Some(Branch::B) => [
            BlockType::GoldBlock,
            BlockType::GoldBlock,
            BlockType::RawGoldBlock,
        ],
        None => [
            BlockType::IronBlock,
            BlockType::GoldBlock,
            BlockType::DiamondBlock,
        ],
    };
    let mut parts = Vec::new();
    let steps = (n as usize).min(3);
    for (step, block) in tiers.iter().take(steps).enumerate() {
        let inset = 0.05 + 0.1 * step as f32;
        parts.push(Part::fixed(
            *block,
            [inset, 0.2 * step as f32, inset],
            [1.0 - 2.0 * inset, 0.2, 1.0 - 2.0 * inset],
        ));
    }
    let top = 0.2 * steps as f32;
    parts.push(Part::turret(
        BlockType::Beacon,
        [0.3, top, 0.3],
        [0.4, 0.4, 0.4],
    ));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_level() -> [Level; 5] {
        [
            Level::T1,
            Level::T2,
            Level::T3,
            Level::T4(Branch::A),
            Level::T4(Branch::B),
        ]
    }

    #[test]
    fn every_tower_fits_its_cell_and_has_a_turret() {
        for kind in TowerKind::ALL {
            for level in every_level() {
                let parts = tower_model(kind, level);
                assert!(parts.len() >= 2, "{kind:?} {level:?}");
                assert!(parts.iter().any(|part| part.turret), "{kind:?} {level:?}");
                for part in &parts {
                    for axis in [0, 2] {
                        assert!(part.from[axis] >= -0.01, "{kind:?} {level:?} {part:?}");
                        let overhang = if part.turret { 0.25 } else { 0.01 };
                        assert!(
                            part.from[axis] + part.size[axis] <= 1.0 + overhang,
                            "{kind:?} {level:?} {part:?}"
                        );
                    }
                    assert!(part.size.iter().all(|side| *side > 0.0));
                }
            }
        }
    }

    #[test]
    fn a_tower_grows_with_its_level() {
        for kind in TowerKind::ALL {
            let heights: Vec<f32> = [Level::T1, Level::T2, Level::T3]
                .into_iter()
                .map(|level| model_height(kind, level))
                .collect();
            assert!(
                heights.windows(2).all(|pair| pair[1] >= pair[0]),
                "{kind:?} {heights:?}"
            );
            assert!(heights[2] > heights[0], "{kind:?} {heights:?}");
        }
    }

    #[test]
    fn the_two_fourth_tier_branches_look_different() {
        for kind in TowerKind::ALL {
            assert_ne!(
                tower_model(kind, Level::T4(Branch::A)),
                tower_model(kind, Level::T4(Branch::B)),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_fixed_part_ignores_the_aim() {
        let part = Part::fixed(BlockType::Stone, [0.1, 0.0, 0.1], [0.8, 0.2, 0.8]);
        assert_eq!(part.transformation(0.0), part.transformation(90.0));
    }

    #[test]
    fn a_turret_turns_around_the_cell_center() {
        let part = Part::turret(BlockType::Stone, [0.2, 0.5, 0.3], [0.6, 0.4, 0.4]);
        let center = |yaw: f32| {
            let offset = [
                part.from[0] - 0.5 + part.size[0] / 2.0,
                0.0,
                part.from[2] - 0.5 + part.size[2] / 2.0,
            ];
            let [x, _, z] = rotate(offset, yaw);
            ((x * 1000.0).round(), (z * 1000.0).round())
        };
        assert_eq!(center(0.0), (0.0, 0.0));
        assert_eq!(center(137.0), (0.0, 0.0));
        assert_ne!(part.transformation(0.0), part.transformation(90.0));
        let [_, _, forward] = rotate([0.0, 0.0, 1.0], 0.0);
        assert!((forward - 1.0).abs() < 1e-6);
    }
}
