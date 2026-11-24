use std::collections::BTreeMap;

use rspigot_plugin::BlockType;

use super::arena::Extent;

type Cell = (i32, i32, i32);
type Blocks = BTreeMap<Cell, BlockType>;

pub const TERRACE_ROWS: i32 = 3;
pub const PALACE_DEPTH: i32 = 11;
pub const MODULE: i32 = 12;
pub const BRIDGE_LIFT: i32 = 10;
pub const BRIDGE_HALF: i32 = 3;
pub const PAVILION_HALF: i32 = 6;
pub const PIER_SPACING: i32 = 8;
pub const CORNER_TIERS: i32 = 5;
pub const PAGODA_TIERS: i32 = 3;

pub const FOUNDATION: BlockType = BlockType::StoneBricks;
pub const PLINTH: BlockType = BlockType::ChiseledStoneBricks;
pub const PILLAR: BlockType = BlockType::StrippedMangroveLog;
pub const WALL: BlockType = BlockType::SmoothSandstone;
pub const WINDOW: BlockType = BlockType::BlackStainedGlass;
pub const ROOF: BlockType = BlockType::DeepslateTiles;
pub const TRIM: BlockType = BlockType::YellowTerracotta;
pub const RIDGE: BlockType = BlockType::GoldBlock;
pub const STATUE: BlockType = BlockType::Calcite;
pub const STATUE_BASE: BlockType = BlockType::QuartzBlock;
pub const LANTERN: BlockType = BlockType::Lantern;
pub const POST: BlockType = BlockType::SpruceFence;
pub const WATER: BlockType = BlockType::Water;
pub const CHANNEL_BED: BlockType = BlockType::Stone;
pub const WALKWAY: BlockType = BlockType::PolishedAndesite;
pub const WALKWAY_ACCENT: BlockType = BlockType::StoneBricks;
pub const DECK: BlockType = BlockType::PolishedAndesite;

pub struct Stadium {
    pub outer: Extent,
}

pub fn build(
    blocks: &mut Blocks,
    inside: Extent,
    footprints: &[Extent],
    floor_y: i32,
    decor: bool,
) -> Stadium {
    floor(blocks, inside, footprints, floor_y);
    let outer = inside.grown(TERRACE_ROWS + PALACE_DEPTH);
    if !decor {
        return Stadium { outer };
    }
    terrace(blocks, inside, floor_y);
    palaces(blocks, inside, floor_y);
    corner_towers(blocks, inside, floor_y);
    bridge(blocks, inside, footprints, floor_y);
    lanterns(blocks, footprints, floor_y);
    Stadium { outer }
}

fn put(blocks: &mut Blocks, x: i32, y: i32, z: i32, block: BlockType) {
    blocks.insert((x, y, z), block);
}

fn is_channel(inside: Extent, footprints: &[Extent], x: i32, z: i32) -> bool {
    if inside.ring_inner(x, z) == 1 {
        return true;
    }
    footprints.iter().any(|left| {
        footprints.iter().any(|right| {
            right.min_x - left.max_x == 4
                && x == left.max_x + 2
                && z >= left.min_z.max(right.min_z)
                && z <= left.max_z.min(right.max_z)
        })
    })
}

fn floor(blocks: &mut Blocks, inside: Extent, footprints: &[Extent], floor_y: i32) {
    for x in inside.min_x..=inside.max_x {
        for z in inside.min_z..=inside.max_z {
            if footprints.iter().any(|footprint| footprint.contains(x, z)) {
                continue;
            }
            if is_channel(inside, footprints, x, z) {
                put(blocks, x, floor_y, z, WATER);
                put(blocks, x, floor_y - 1, z, CHANNEL_BED);
                continue;
            }
            let accent = x.rem_euclid(4) == 0 || z.rem_euclid(4) == 0;
            put(
                blocks,
                x,
                floor_y,
                z,
                if accent { WALKWAY_ACCENT } else { WALKWAY },
            );
        }
    }
}

fn terrace(blocks: &mut Blocks, inside: Extent, floor_y: i32) {
    let outer = inside.grown(TERRACE_ROWS);
    for x in outer.min_x..=outer.max_x {
        for z in outer.min_z..=outer.max_z {
            let ring = inside.ring(x, z);
            if ring == 0 {
                continue;
            }
            for y in floor_y..floor_y + ring {
                put(blocks, x, y, z, FOUNDATION);
            }
            put(blocks, x, floor_y + ring, z, WALKWAY);
        }
    }
    let row = inside.grown(1);
    let mut spots = Vec::new();
    for x in (row.min_x..=row.max_x).step_by((MODULE / 2) as usize) {
        spots.push((x, row.min_z));
        spots.push((x, row.max_z));
    }
    for z in (row.min_z..=row.max_z).step_by((MODULE / 2) as usize) {
        spots.push((row.min_x, z));
        spots.push((row.max_x, z));
    }
    for (x, z) in spots {
        let base = floor_y + 2;
        put(blocks, x, base, z, STATUE_BASE);
        put(blocks, x, base + 1, z, STATUE);
        put(blocks, x, base + 2, z, STATUE);
        put(blocks, x, base + 3, z, TRIM);
        put(blocks, x, base + 4, z, LANTERN);
    }
}

#[derive(Clone, Copy)]
struct BandCell {
    depth: i32,
    along: i32,
}

fn band_cell(inside: Extent, x: i32, z: i32) -> Option<BandCell> {
    let ring = inside.ring(x, z);
    if ring <= TERRACE_ROWS || ring > TERRACE_ROWS + PALACE_DEPTH {
        return None;
    }
    let outside_x = x < inside.min_x || x > inside.max_x;
    let outside_z = z < inside.min_z || z > inside.max_z;
    if outside_x && outside_z {
        return None;
    }
    let along = if outside_z {
        x - inside.min_x
    } else {
        z - inside.min_z
    };
    Some(BandCell {
        depth: ring - TERRACE_ROWS,
        along,
    })
}

fn facade(blocks: &mut Blocks, x: i32, z: i32, from: i32, to: i32, along: i32) {
    let slot = along.rem_euclid(4);
    for y in from..=to {
        let block = if slot == 0 {
            PILLAR
        } else if slot == 2 && y > from && y < to {
            WINDOW
        } else {
            WALL
        };
        put(blocks, x, y, z, block);
    }
}

fn wall(blocks: &mut Blocks, x: i32, z: i32, from: i32, to: i32) {
    for y in from..=to {
        put(blocks, x, y, z, WALL);
    }
}

struct Eave {
    low: i32,
    high: i32,
    flare: bool,
}

fn eave(blocks: &mut Blocks, x: i32, z: i32, y: i32, depth: i32, eave: Eave) {
    if depth < eave.low || depth > eave.high {
        return;
    }
    let edge = depth == eave.low || depth == eave.high;
    put(blocks, x, y, z, if edge { TRIM } else { ROOF });
    if eave.flare && depth == eave.low {
        put(blocks, x, y + 1, z, TRIM);
    }
}

const fn span(low: i32, high: i32) -> Eave {
    Eave {
        low,
        high,
        flare: false,
    }
}

const fn flared(low: i32, high: i32, flare: bool) -> Eave {
    Eave { low, high, flare }
}

fn palace_column(blocks: &mut Blocks, x: i32, z: i32, cell: BandCell, floor_y: i32) {
    let base = floor_y + TERRACE_ROWS;
    for y in floor_y - 1..=base {
        put(blocks, x, y, z, FOUNDATION);
    }
    let module = cell.along.rem_euclid(MODULE);
    let flare = module == 0;
    let d = cell.depth;

    let hall = base + 1;
    match d {
        1 => facade(blocks, x, z, hall, hall + 5, cell.along),
        PALACE_DEPTH => wall(blocks, x, z, hall, hall + 5),
        _ => {}
    }
    let first = hall + 6;
    eave(blocks, x, z, first, d, flared(1, PALACE_DEPTH, flare));
    eave(blocks, x, z, first + 1, d, span(2, PALACE_DEPTH - 1));

    let middle = first + 2;
    match d {
        3 => facade(blocks, x, z, middle, middle + 4, cell.along),
        10 => wall(blocks, x, z, middle, middle + 4),
        _ => {}
    }
    let second = middle + 5;
    eave(blocks, x, z, second, d, flared(2, PALACE_DEPTH, flare));
    eave(blocks, x, z, second + 1, d, span(3, 10));

    let top = second + 2;
    match d {
        5 => facade(blocks, x, z, top, top + 3, cell.along),
        9 => wall(blocks, x, z, top, top + 3),
        _ => {}
    }
    let third = top + 4;
    eave(blocks, x, z, third, d, flared(4, 10, flare));
    eave(blocks, x, z, third + 1, d, span(5, 9));
    eave(blocks, x, z, third + 2, d, span(6, 8));
    if d == 7 {
        put(blocks, x, third + 3, z, RIDGE);
    }

    if cell.along.div_euclid(MODULE).rem_euclid(2) == 1 {
        pagoda_column(blocks, x, z, module, d, third + 3);
    }
}

fn pagoda_column(blocks: &mut Blocks, x: i32, z: i32, module: i32, depth: i32, mut y: i32) {
    for tier in 0..PAGODA_TIERS {
        let in_walls =
            (3 + tier..=8 - tier).contains(&module) && (4 + tier..=9 - tier).contains(&depth);
        let in_roof =
            (2 + tier..=9 - tier).contains(&module) && (3 + tier..=10 - tier).contains(&depth);
        let rim_m = module == 3 + tier || module == 8 - tier;
        let rim_d = depth == 4 + tier || depth == 9 - tier;
        if in_walls && (rim_m || rim_d) {
            for level in 0..3 {
                let block = if rim_m && rim_d {
                    PILLAR
                } else if level == 1 && (module + depth).rem_euclid(2) == 0 {
                    WINDOW
                } else {
                    WALL
                };
                put(blocks, x, y + level, z, block);
            }
        }
        if in_roof {
            let edge_m = module == 2 + tier || module == 9 - tier;
            let edge_d = depth == 3 + tier || depth == 10 - tier;
            put(
                blocks,
                x,
                y + 3,
                z,
                if edge_m || edge_d { TRIM } else { ROOF },
            );
            if edge_m && edge_d {
                put(blocks, x, y + 4, z, TRIM);
            }
        }
        y += 4;
    }
    if (5..=6).contains(&module) && (6..=7).contains(&depth) {
        for level in 0..3 {
            put(blocks, x, y + level, z, RIDGE);
        }
    }
}

fn palaces(blocks: &mut Blocks, inside: Extent, floor_y: i32) {
    let outer = inside.grown(TERRACE_ROWS + PALACE_DEPTH);
    for x in outer.min_x..=outer.max_x {
        for z in outer.min_z..=outer.max_z {
            if let Some(cell) = band_cell(inside, x, z) {
                palace_column(blocks, x, z, cell, floor_y);
            }
        }
    }
}

fn corner_towers(blocks: &mut Blocks, inside: Extent, floor_y: i32) {
    let near = TERRACE_ROWS + 1;
    let far = TERRACE_ROWS + PALACE_DEPTH;
    let corners = [
        (inside.min_x - far, inside.min_z - far),
        (inside.max_x + near, inside.min_z - far),
        (inside.min_x - far, inside.max_z + near),
        (inside.max_x + near, inside.max_z + near),
    ];
    let side = PALACE_DEPTH;
    for (x0, z0) in corners {
        let mut y = floor_y + TERRACE_ROWS + 1;
        for dx in 0..side {
            for dz in 0..side {
                for level in floor_y - 1..y - 1 {
                    put(blocks, x0 + dx, level, z0 + dz, FOUNDATION);
                }
                put(blocks, x0 + dx, y - 1, z0 + dz, PLINTH);
            }
        }
        for tier in 0..CORNER_TIERS {
            let low = 1 + tier;
            let high = side - 2 - tier;
            if low > high {
                break;
            }
            let height = 8 - tier.min(3);
            for dx in low..=high {
                for dz in low..=high {
                    let rim_x = dx == low || dx == high;
                    let rim_z = dz == low || dz == high;
                    if !(rim_x || rim_z) {
                        continue;
                    }
                    for level in 0..height {
                        let block = if rim_x && rim_z {
                            PILLAR
                        } else if level > 0 && level < height - 1 && (dx + dz).rem_euclid(2) == 0 {
                            WINDOW
                        } else {
                            WALL
                        };
                        put(blocks, x0 + dx, y + level, z0 + dz, block);
                    }
                }
            }
            let roof_y = y + height;
            for dx in low - 1..=high + 1 {
                for dz in low - 1..=high + 1 {
                    let edge_x = dx == low - 1 || dx == high + 1;
                    let edge_z = dz == low - 1 || dz == high + 1;
                    let block = if edge_x || edge_z { TRIM } else { ROOF };
                    put(blocks, x0 + dx, roof_y, z0 + dz, block);
                    if edge_x && edge_z {
                        put(blocks, x0 + dx, roof_y + 1, z0 + dz, TRIM);
                    }
                }
            }
            y = roof_y + 1;
        }
        let mid = side / 2;
        for level in 0..5 {
            put(blocks, x0 + mid, y + level, z0 + mid, RIDGE);
        }
        put(blocks, x0 + mid, y + 5, z0 + mid, LANTERN);
    }
}

fn bridge(blocks: &mut Blocks, inside: Extent, footprints: &[Extent], floor_y: i32) {
    let center = (inside.min_z + inside.max_z).div_euclid(2);
    let clear = footprints.iter().all(|footprint| {
        footprint.max_z < center - PAVILION_HALF || footprint.min_z > center + PAVILION_HALF
    });
    if !clear {
        return;
    }
    let deck = floor_y + BRIDGE_LIFT;
    let middle = (inside.min_x + inside.max_x).div_euclid(2);
    for x in inside.min_x - TERRACE_ROWS..=inside.max_x + TERRACE_ROWS {
        let offset = x - inside.min_x;
        let pier = offset.rem_euclid(PIER_SPACING) == 0;
        let in_pavilion = (x - middle).abs() <= PAVILION_HALF;
        for dz in -BRIDGE_HALF..=BRIDGE_HALF {
            let z = center + dz;
            let edge = dz.abs() == BRIDGE_HALF;
            put(blocks, x, deck, z, if edge { TRIM } else { DECK });
            if edge && pier {
                put(blocks, x, floor_y + 1, z, PLINTH);
                for y in floor_y + 2..deck {
                    put(blocks, x, y, z, PILLAR);
                }
            }
            if edge && !in_pavilion {
                let column = offset.rem_euclid(4) == 0;
                for y in deck + 1..deck + 5 {
                    if column {
                        put(blocks, x, y, z, PILLAR);
                    } else if y == deck + 1 {
                        put(blocks, x, y, z, POST);
                    }
                }
                put(blocks, x, deck + 5, z, TRIM);
            }
        }
        if in_pavilion {
            continue;
        }
        if offset.rem_euclid(PIER_SPACING) == PIER_SPACING / 2 {
            put(blocks, x, deck + 1, center, LANTERN);
        }
        for step in 0..=BRIDGE_HALF + 1 {
            let half = BRIDGE_HALF + 1 - step;
            let y = deck + 6 + step;
            for dz in -half..=half {
                let block = if half == 0 {
                    RIDGE
                } else if dz.abs() == half {
                    TRIM
                } else {
                    ROOF
                };
                put(blocks, x, y, center + dz, block);
            }
        }
    }
    pavilion(blocks, middle, center, deck);
}

fn pavilion(blocks: &mut Blocks, cx: i32, cz: i32, deck: i32) {
    for dx in -PAVILION_HALF..=PAVILION_HALF {
        for dz in -PAVILION_HALF..=PAVILION_HALF {
            put(blocks, cx + dx, deck, cz + dz, DECK);
        }
    }
    let mut y = deck + 1;
    for tier in 0..3 {
        let half = PAVILION_HALF - 1 - tier * 2;
        if half < 1 {
            break;
        }
        let height = 5 - tier;
        for dx in -half..=half {
            for dz in -half..=half {
                let rim_x = dx.abs() == half;
                let rim_z = dz.abs() == half;
                if !(rim_x || rim_z) {
                    continue;
                }
                for level in 0..height {
                    let block = if (rim_x && rim_z) || (dx + dz).rem_euclid(3) == 0 {
                        PILLAR
                    } else if level > 0 && level < height - 1 {
                        WINDOW
                    } else {
                        WALL
                    };
                    put(blocks, cx + dx, y + level, cz + dz, block);
                }
            }
        }
        let roof_y = y + height;
        for ring in 0..3 {
            let reach = half + 2 - ring;
            for dx in -reach..=reach {
                for dz in -reach..=reach {
                    if dx.abs().max(dz.abs()) != reach && ring < 2 {
                        continue;
                    }
                    let edge = dx.abs() == reach || dz.abs() == reach;
                    let block = if ring == 0 && edge { TRIM } else { ROOF };
                    put(blocks, cx + dx, roof_y + ring, cz + dz, block);
                    if ring == 0 && dx.abs() == reach && dz.abs() == reach {
                        put(blocks, cx + dx, roof_y + 1, cz + dz, TRIM);
                    }
                }
            }
        }
        y = roof_y + 3;
    }
    for level in 0..4 {
        put(blocks, cx, y + level, cz, RIDGE);
    }
}

fn lanterns(blocks: &mut Blocks, footprints: &[Extent], floor_y: i32) {
    for footprint in footprints {
        let corners = [
            (footprint.min_x, footprint.min_z),
            (footprint.max_x, footprint.min_z),
            (footprint.min_x, footprint.max_z),
            (footprint.max_x, footprint.max_z),
        ];
        for (x, z) in corners {
            put(blocks, x, floor_y + 3, z, POST);
            put(blocks, x, floor_y + 4, z, LANTERN);
        }
    }
}
