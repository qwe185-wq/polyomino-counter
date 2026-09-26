//! 独立逐格 oracle：不复用生产的轨道构造、Euler 公式或位扩散。
use super::*;

fn flood(grid: &[bool], width: usize, height: usize, start: usize) -> usize {
    let mut seen = vec![false; grid.len()];
    let mut pending = vec![start];
    seen[start] = true;
    let mut count = 0;
    while let Some(cell) = pending.pop() {
        count += 1;
        let (r, c) = (cell / width, cell % width);
        for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
            let (nr, nc) = (r as isize + dr, c as isize + dc);
            if nr < 0 || nc < 0 || nr >= height as isize || nc >= width as isize {
                continue;
            }
            let next = nr as usize * width + nc as usize;
            if grid[next] && !seen[next] {
                seen[next] = true;
                pending.push(next);
            }
        }
    }
    count
}

fn brute_symmetric(width: usize, height: usize, quarter: bool) -> Counts {
    let area = width * height;
    let mut counts = Counts::default();
    for bits in 1u64..1u64 << area {
        let grid: Vec<bool> = (0..area).map(|i| bits & (1 << i) != 0).collect();
        if !(0..area).all(|i| {
            let (r, c) = (i / width, i % width);
            let target = if quarter {
                c * width + width - 1 - r
            } else {
                area - 1 - i
            };
            grid[i] == grid[target]
        }) {
            continue;
        }
        if !grid[..width].iter().any(|&v| v)
            || !grid[area - width..].iter().any(|&v| v)
            || !(0..height).any(|r| grid[r * width])
            || !(0..height).any(|r| grid[r * width + width - 1])
        {
            continue;
        }
        let start = grid.iter().position(|&v| v).unwrap();
        if flood(&grid, width, height, start) != bits.count_ones() as usize {
            continue;
        }
        let outer_width = width + 2;
        let outer_height = height + 2;
        let mut background = vec![true; outer_width * outer_height];
        for r in 0..height {
            for c in 0..width {
                background[(r + 1) * outer_width + c + 1] = !grid[r * width + c];
            }
        }
        let holes = flood(&background, outer_width, outer_height, 0)
            != background.iter().filter(|&&v| v).count();
        if holes {
            counts.has_hole += 1u32;
        } else {
            counts.no_hole += 1u32;
        }
    }
    counts
}

#[test]
fn optimized_symmetric_counts_match_independent_exhaustive_grid() {
    for (width, height, quarter) in [
        (2, 8, false),
        (8, 2, false),
        (2, 9, false),
        (9, 2, false),
        (4, 4, true),
    ] {
        let expected = brute_symmetric(width, height, quarter);
        let actual = symmetric_bbox(width, height, quarter).unwrap();
        assert_eq!(
            actual.no_hole, expected.no_hole,
            "{width}x{height} quarter={quarter}"
        );
        assert_eq!(
            actual.has_hole, expected.has_hole,
            "{width}x{height} quarter={quarter}"
        );
    }
}

#[test]
fn padded_holes_match_independent_background_flood() {
    let mut seed = 0x1327_b49a_8241_23adu64;
    let mut checked_holes = 0;
    let mut checked_no_holes = 0;
    for (width, height) in [(8, 8), (9, 9), (10, 10), (7, 15)] {
        for sample in 0..256 {
            let mut grid = vec![false; width * height];
            let mut packed = 0u128;
            for r in 0..height {
                for c in 0..width {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    grid[r * width + c] = match sample {
                        0 => r == 0 || c == 0 || r == height - 1 || c == width - 1,
                        1 => true,
                        _ => seed % 5 != 0,
                    };
                    if grid[r * width + c] {
                        packed |= 1u128 << (r * (width + 1) + c);
                    }
                }
            }
            let start = grid.iter().position(|&v| v).unwrap();
            let connected =
                flood(&grid, width, height, start) == grid.iter().filter(|&&v| v).count();
            assert_eq!(connected_padded(packed, width + 1), connected);
            if !connected {
                continue;
            }
            let ow = width + 2;
            let oh = height + 2;
            let mut background = vec![true; ow * oh];
            for r in 0..height {
                for c in 0..width {
                    background[(r + 1) * ow + c + 1] = !grid[r * width + c];
                }
            }
            let hole = flood(&background, ow, oh, 0) != background.iter().filter(|&&v| v).count();
            assert_eq!(
                hole_padded(packed, width + 1),
                hole,
                "{width}x{height} sample={sample}"
            );
            if hole {
                checked_holes += 1;
            } else {
                checked_no_holes += 1;
            }
        }
    }
    assert!(checked_holes > 4);
    assert!(checked_no_holes >= 4);
}
