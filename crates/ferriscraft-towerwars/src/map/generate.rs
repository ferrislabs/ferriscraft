use std::collections::BTreeSet;

use crate::domain::{Aabb, LANE_COUNT, Vec3};

use super::{LaneMap, Map};

const WALL_TOP: f32 = 65.0;
const TRENCH: f32 = 63.0;
const ANCHOR_Y: f32 = 80.0;
const ROW_LENGTH: i32 = 25;
const ROW_GAP: i32 = 6;
const WALL_FROM: i32 = 2;
const WALL_TO: i32 = 4;
const LANE_STRIDE_X: i32 = 41;
const LANE_STRIDE_Z: i32 = 38;
const LANES_PER_ROW: usize = 3;

pub fn test_map() -> Map {
    Map {
        lanes: (0..LANE_COUNT).map(lane).collect(),
    }
}

fn centerline() -> Vec<(i32, i32)> {
    vec![
        (0, 0),
        (ROW_LENGTH, 0),
        (ROW_LENGTH, ROW_GAP),
        (0, ROW_GAP),
        (0, ROW_GAP * 2),
        (ROW_LENGTH, ROW_GAP * 2),
    ]
}

fn reach(path: &[(i32, i32)], x: i32, z: i32) -> i32 {
    path.windows(2)
        .map(|pair| {
            let (low_x, high_x) = (pair[0].0.min(pair[1].0), pair[0].0.max(pair[1].0));
            let (low_z, high_z) = (pair[0].1.min(pair[1].1), pair[0].1.max(pair[1].1));
            let dx = (low_x - x).max(x - high_x).max(0);
            let dz = (low_z - z).max(z - high_z).max(0);
            dx.max(dz)
        })
        .min()
        .unwrap_or(i32::MAX)
}

fn wall_cells() -> BTreeSet<(i32, i32)> {
    let path = centerline();
    let mut cells = BTreeSet::new();
    for x in -WALL_TO..=ROW_LENGTH + WALL_TO {
        for z in -WALL_TO..=ROW_GAP * 2 + WALL_TO {
            if (WALL_FROM..=WALL_TO).contains(&reach(&path, x, z)) {
                cells.insert((x, z));
            }
        }
    }
    cells
}

fn rectangles(cells: &BTreeSet<(i32, i32)>) -> Vec<(i32, i32, i32, i32)> {
    let zs: BTreeSet<i32> = cells.iter().map(|(_, z)| *z).collect();
    let mut open: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut done = Vec::new();
    for z in zs {
        let mut runs = Vec::new();
        let mut xs = cells
            .iter()
            .filter(|(_, cz)| *cz == z)
            .map(|(x, _)| *x)
            .peekable();
        while let Some(start) = xs.next() {
            let mut end = start;
            while xs.peek() == Some(&(end + 1)) {
                end += 1;
                xs.next();
            }
            runs.push((start, end));
        }
        let mut next = Vec::new();
        for (x0, x1) in runs {
            match open
                .iter()
                .position(|(ox0, ox1, _, oz1)| *ox0 == x0 && *ox1 == x1 && *oz1 == z - 1)
            {
                Some(index) => {
                    let (ox0, ox1, oz0, _) = open.remove(index);
                    next.push((ox0, ox1, oz0, z));
                }
                None => next.push((x0, x1, z, z)),
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    done.sort_unstable();
    done
}

fn lane(index: usize) -> LaneMap {
    let origin_x = (index % LANES_PER_ROW) as i32 * LANE_STRIDE_X;
    let origin_z = (index / LANES_PER_ROW) as i32 * LANE_STRIDE_Z;
    let at = |(x, z): (i32, i32)| {
        Vec3::new(
            (origin_x + x) as f32 + 0.5,
            TRENCH,
            (origin_z + z) as f32 + 0.5,
        )
    };
    let path = centerline();
    let build_bounds = rectangles(&wall_cells())
        .into_iter()
        .map(|(x0, x1, z0, z1)| Aabb {
            min: Vec3::new((origin_x + x0) as f32, WALL_TOP, (origin_z + z0) as f32),
            max: Vec3::new(
                (origin_x + x1 + 1) as f32,
                WALL_TOP + 1.0,
                (origin_z + z1 + 1) as f32,
            ),
        })
        .collect();
    LaneMap {
        lane_id: index as u8,
        spawn_point: at(path[0]),
        waypoints: path[1..].iter().copied().map(at).collect(),
        build_bounds,
        spectate_anchor: Vec3::new(
            (origin_x + ROW_LENGTH / 2) as f32 + 0.5,
            ANCHOR_Y,
            (origin_z + ROW_GAP) as f32 + 0.5,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walls_are_three_cells_wide_between_two_rows_of_the_path() {
        let cells = wall_cells();
        let between: Vec<i32> = (1..ROW_GAP).filter(|z| cells.contains(&(10, *z))).collect();
        assert_eq!(between, vec![2, 3, 4]);
    }

    #[test]
    fn rectangles_cover_exactly_the_wall_cells() {
        let cells = wall_cells();
        let mut covered = BTreeSet::new();
        for (x0, x1, z0, z1) in rectangles(&cells) {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    assert!(covered.insert((x, z)), "({x}, {z}) couvert deux fois");
                }
            }
        }
        assert_eq!(covered, cells);
    }

    #[test]
    fn the_walls_stand_two_blocks_above_the_mob_path() {
        let map = test_map();
        for lane in &map.lanes {
            for bounds in &lane.build_bounds {
                assert_eq!(bounds.min.y - lane.spawn_point.y, 2.0);
            }
        }
    }
}
