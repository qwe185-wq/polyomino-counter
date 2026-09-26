//! 只计数的前沿连通性 DP；旋转固定点另以紧包围盒轨道枚举。

use crate::types::{RoomCount, MAX_N};
use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct State {
    // 扫描线左侧是新行，右侧是旧行；0 表示空格。
    labels: [u8; MAX_N],
    // 上一步处理位置的旧行占据情况，即当前格左上邻居。
    old_left: bool,
    // 0=尚未开始，1=仍有活跃前景，2=唯一组件已经关闭。
    phase: u8,
    chi: i8,
}

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
struct Counts {
    no_hole: u64,
    has_hole: u64,
}

impl Counts {
    fn add(&mut self, other: Self) {
        self.no_hole += other.no_hole;
        self.has_hole += other.has_hole;
    }

    fn total(self) -> u64 {
        self.no_hole + self.has_hole
    }

    fn record(&mut self, chi: i8, multiplicity: u64) {
        assert!(chi <= 1, "连通终态 Euler 特征数不应大于 1");
        if chi == 1 {
            self.no_hole += multiplicity;
        } else {
            self.has_hole += multiplicity;
        }
    }
}

// 标签规范化保证同一种前沿分区只有一个键。
fn canonicalize(labels: &mut [u8; MAX_N], width: usize) {
    let mut rename = [0u8; MAX_N + 1];
    let mut next = 1;
    for label in labels[..width].iter_mut() {
        if *label != 0 {
            if rename[*label as usize] == 0 {
                rename[*label as usize] = next;
                next += 1;
            }
            *label = rename[*label as usize];
        }
    }
}

fn transition(state: State, width: usize, col: usize, occupied: bool) -> Option<State> {
    if occupied && state.phase == 2 {
        return None;
    }
    let mut next = state;
    let left = col > 0 && state.labels[col - 1] != 0;
    let up = state.labels[col] != 0;
    let up_left = col > 0 && state.old_left;
    let up_right = col + 1 < width && state.labels[col + 1] != 0;
    // 行末的旧左上邻居不会被下一行首格读取，归零以合并等价状态。
    next.old_left = col + 1 < width && up;

    if occupied {
        let left_label = if left { state.labels[col - 1] } else { 0 };
        let up_label = state.labels[col];
        let label = if left_label != 0 {
            left_label
        } else if up_label != 0 {
            up_label
        } else {
            state.labels[..width].iter().copied().max().unwrap_or(0) + 1
        };
        next.labels[col] = label;
        if left_label != 0 && up_label != 0 && left_label != up_label {
            for existing in next.labels[..width].iter_mut() {
                if *existing == up_label {
                    *existing = left_label;
                }
            }
        }
        // 闭方格复形：χ=V−E+F。对角接触的顶点也参与计数。
        let new_vertices =
            1 + (!left as i8) + (!(up || up_right) as i8) + (!(left || up || up_left) as i8);
        let new_edges = 4 - (left as i8) - (up as i8);
        next.chi += new_vertices - new_edges + 1;
        next.phase = 1;
    } else {
        let vanished = state.labels[col];
        next.labels[col] = 0;
        if vanished != 0 && !next.labels[..width].contains(&vanished) {
            // 消失的分量永远无法再与未来格子连接。
            if next.labels[..width].iter().any(|&label| label != 0) {
                return None;
            }
            next.phase = 2;
        }
    }
    canonicalize(&mut next.labels, width);
    Some(next)
}

// 每一行末读取 C(w,h)，不终止仍可向下一行生长的前沿。
fn placement_rows(width: usize, height: usize) -> Vec<Counts> {
    if width == 0 || height == 0 {
        return vec![Counts::default(); height + 1];
    }
    let mut rows = vec![Counts::default(); height + 1];
    let mut states = FxHashMap::default();
    let mut following = FxHashMap::default();
    // 已关闭的唯一组件只有全空的未来延续，不再留在活跃前沿中。
    let mut closed = Counts::default();
    states.insert(
        State {
            labels: [0; MAX_N],
            old_left: false,
            phase: 0,
            chi: 0,
        },
        1u64,
    );
    for index in 0..width * height {
        let col = index % width;
        for (state, multiplicity) in states.drain() {
            for occupied in [false, true] {
                if let Some(next) = transition(state, width, col, occupied) {
                    if next.phase == 2 {
                        closed.record(next.chi, multiplicity);
                    } else {
                        *following.entry(next).or_insert(0) += multiplicity;
                    }
                }
            }
        }
        std::mem::swap(&mut states, &mut following);
        if col + 1 == width {
            let mut count = closed;
            count.add(summarize(&states, width));
            rows[index / width + 1] = count;
        }
    }
    rows
}

fn summarize(states: &FxHashMap<State, u64>, width: usize) -> Counts {
    let mut result = Counts::default();
    for (state, &multiplicity) in states {
        if state.phase != 0 {
            // 扫描结束时仍在前沿的不同标签代表不同分量。
            if state.labels[..width].iter().any(|&label| label > 1) {
                continue;
            }
            result.record(state.chi, multiplicity);
        }
    }
    result
}

#[cfg(test)]
fn connected_placements(width: usize, height: usize) -> Counts {
    placement_rows(width, height)[height]
}

fn connected(mask: u64, _width: usize, _height: usize) -> bool {
    if mask == 0 {
        return false;
    }
    let mut seen = mask & mask.wrapping_neg();
    let mut frontier = seen;
    while frontier != 0 {
        // 行步长 8、宽度至多 6，水平移位不会进入相邻行的有效列。
        let adjacent =
            ((frontier << 1) | (frontier >> 1) | (frontier << 8) | (frontier >> 8)) & mask;
        frontier = adjacent & !seen;
        seen |= frontier;
    }
    seen == mask
}

fn has_hole(mask: u64) -> bool {
    let faces = mask.count_ones();
    let edges = (mask | mask << 8).count_ones() + (mask | mask << 1).count_ones();
    let vertices = (mask | mask << 1 | mask << 8 | mask << 9).count_ones();
    edges + 1 > vertices + faces
}

fn boundary_masks(width: usize, height: usize) -> [u64; 4] {
    let top = (1u64 << width) - 1;
    let bottom = top << ((height - 1) * 8);
    let mut left = 0u64;
    let mut right = 0u64;
    for row in 0..height {
        left |= 1u64 << (row * 8);
        right |= 1u64 << (row * 8 + width - 1);
    }
    [top, bottom, left, right]
}

#[cfg(test)]
fn full_bounding_box(mask: u64, boundaries: [u64; 4]) -> bool {
    boundaries.iter().all(|&side| mask & side != 0)
}

fn rotation_orbits(width: usize, height: usize, quarter_turn: bool) -> Vec<u64> {
    let mut unseen = 0u64;
    for row in 0..height {
        for col in 0..width {
            unseen |= 1u64 << (row * 8 + col);
        }
    }
    let mut orbits = Vec::new();
    while unseen != 0 {
        let first = unseen.trailing_zeros() as usize;
        let mut row = first / 8;
        let mut col = first % 8;
        let mut orbit = 0u64;
        loop {
            let bit = 1u64 << (row * 8 + col);
            if orbit & bit != 0 {
                break;
            }
            orbit |= bit;
            (row, col) = if quarter_turn {
                (col, width - 1 - row)
            } else {
                (height - 1 - row, width - 1 - col)
            };
        }
        unseen &= !orbit;
        orbits.push(orbit);
    }
    orbits
}

fn symmetric_bbox(width: usize, height: usize, quarter_turn: bool) -> Counts {
    if quarter_turn && width != height {
        return Counts::default();
    }
    let orbits = rotation_orbits(width, height, quarter_turn);
    let boundaries = boundary_masks(width, height);
    let mut counts = Counts::default();
    let mut mask = 0u64;
    for choice in 1usize..(1usize << orbits.len()) {
        // 二进制序号对应的 Gray code 每次只翻转一个轨道。
        mask ^= orbits[choice.trailing_zeros() as usize];
        // 180° 把上边映到下边、左边映到右边；90° 把四边循环置换。
        let touches_all_sides =
            mask & boundaries[0] != 0 && (quarter_turn || mask & boundaries[2] != 0);
        if touches_all_sides && connected(mask, width, height) {
            if has_hole(mask) {
                counts.has_hole += 1;
            } else {
                counts.no_hole += 1;
            }
        }
    }
    counts
}

/// 返回 n=1..max_n 的 one-sided 累计结果；镜像保持不同。
pub fn enumerate_transfer(max_n: usize, verbose: bool) -> Vec<RoomCount> {
    assert!(max_n <= MAX_N, "位图及前沿仅支持 n≤{MAX_N}");
    let mut previous_square = Counts::default();
    let mut sym180 = Counts::default();
    let mut sym90 = Counts::default();
    let mut result = Vec::with_capacity(max_n);
    let mut squares = [Counts::default(); MAX_N + 1];
    let mut strips = [Counts::default(); MAX_N + 1];
    for width in 1..=max_n {
        let height = (width + 1).min(max_n);
        let rows = placement_rows(width, height);
        squares[width] = rows[width];
        if width < max_n {
            strips[width + 1] = rows[width + 1];
        }
    }
    for n in 1..=max_n {
        // C(w,h)=C(h,w)，故二维差分只需当前正方形、相邻窄矩形和前一正方形。
        let square = squares[n];
        let strip = strips[n];
        let fixed = Counts {
            no_hole: square.no_hole + previous_square.no_hole - 2 * strip.no_hole,
            has_hole: square.has_hole + previous_square.has_hole - 2 * strip.has_hole,
        };
        previous_square = square;
        // 转置给出 w×n 与 n×w 间保留连通和孔分类的一一对应。
        for width in 1..n {
            let count = symmetric_bbox(width, n, false);
            sym180.add(count);
            sym180.add(count);
        }
        sym180.add(symmetric_bbox(n, n, false));
        sym90.add(symmetric_bbox(n, n, true));
        let total_num = fixed.total() + 2 * sym90.total() + sym180.total();
        let no_hole_num = fixed.no_hole + 2 * sym90.no_hole + sym180.no_hole;
        let has_hole_num = fixed.has_hole + 2 * sym90.has_hole + sym180.has_hole;
        assert_eq!(total_num % 4, 0, "Burnside total n={n}");
        assert_eq!(no_hole_num % 4, 0, "Burnside no-hole n={n}");
        assert_eq!(has_hole_num % 4, 0, "Burnside has-hole n={n}");
        let count = RoomCount {
            n,
            total: total_num / 4,
            no_hole: no_hole_num / 4,
            has_hole: has_hole_num / 4,
        };
        assert_eq!(count.total, count.no_hole + count.has_hole);
        if verbose {
            eprintln!("{count}");
        }
        result.push(count);
    }
    result
}

#[cfg(test)]
#[path = "transfer_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_snapshot_matches_frozen_reference_through_five() {
        for width in 1..=5 {
            let rows = placement_rows(width, 5);
            for height in 1..=5 {
                let expected = reference::placements_for_test(width, height);
                assert_eq!((rows[height].no_hole, rows[height].has_hole), expected,
                    "board={width}x{height}");
            }
        }
        let actual = enumerate_transfer(5, false);
        let expected = reference::enumerate_transfer(5, false);
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!((a.total, a.no_hole, a.has_hole), (e.total, e.no_hole, e.has_hole));
        }
    }

    // 独立 oracle：紧凑逐格位图、前景四邻接洪泛、背景加一圈后四邻接洪泛。
    fn brute_placements(width: usize, height: usize) -> Counts {
        let area = width * height;
        let mut result = Counts::default();
        for mask in 1u32..(1u32 << area) {
            let mut reached = 0u32;
            let start = mask.trailing_zeros() as usize;
            let mut stack = vec![start];
            while let Some(cell) = stack.pop() {
                let bit = 1u32 << cell;
                if reached & bit != 0 {
                    continue;
                }
                reached |= bit;
                let row = cell / width;
                let col = cell % width;
                if row > 0 && mask & (bit >> width) != 0 {
                    stack.push(cell - width);
                }
                if row + 1 < height && mask & (bit << width) != 0 {
                    stack.push(cell + width);
                }
                if col > 0 && mask & (bit >> 1) != 0 {
                    stack.push(cell - 1);
                }
                if col + 1 < width && mask & (bit << 1) != 0 {
                    stack.push(cell + 1);
                }
            }
            if reached != mask {
                continue;
            }

            let outer_width = width + 2;
            let outer_height = height + 2;
            let mut background_seen = vec![false; outer_width * outer_height];
            let mut outside = vec![0usize];
            while let Some(cell) = outside.pop() {
                if background_seen[cell] {
                    continue;
                }
                background_seen[cell] = true;
                let row = cell / outer_width;
                let col = cell % outer_width;
                let neighbors = [
                    row.checked_sub(1).map(|r| (r, col)),
                    (row + 1 < outer_height).then_some((row + 1, col)),
                    col.checked_sub(1).map(|c| (row, c)),
                    (col + 1 < outer_width).then_some((row, col + 1)),
                ];
                for (r, c) in neighbors.into_iter().flatten() {
                    if r > 0
                        && r <= height
                        && c > 0
                        && c <= width
                        && mask & (1u32 << ((r - 1) * width + c - 1)) != 0
                    {
                        continue;
                    }
                    let neighbor = r * outer_width + c;
                    if !background_seen[neighbor] {
                        outside.push(neighbor);
                    }
                }
            }
            let mut hole = false;
            for row in 0..height {
                for col in 0..width {
                    let bit = 1u32 << (row * width + col);
                    if mask & bit == 0 && !background_seen[(row + 1) * outer_width + col + 1] {
                        hole = true;
                    }
                }
            }
            if hole {
                result.has_hole += 1;
            } else {
                result.no_hole += 1;
            }
        }
        result
    }

    #[test]
    fn every_board_through_four_matches_independent_flood_fill() {
        for width in 1..=4 {
            for height in 1..=4 {
                assert_eq!(
                    connected_placements(width, height),
                    brute_placements(width, height),
                    "board={width}x{height}"
                );
            }
        }
    }

    #[test]
    fn empty_prefix_and_missing_corner_remain_valid() {
        assert_eq!(
            connected_placements(2, 2),
            Counts {
                no_hole: 13,
                has_hole: 0
            }
        );
        assert_eq!(connected_placements(3, 3), brute_placements(3, 3));
    }

    #[test]
    fn rotation_orbits_match_direct_subset_rotation_through_four() {
        for width in 1..=4 {
            for height in 1..=4 {
                for quarter_turn in [false, true] {
                    if quarter_turn && width != height {
                        continue;
                    }
                    let mut expected = Counts::default();
                    for subset in 1u32..(1u32 << (width * height)) {
                        let mut mask = 0u64;
                        let mut rotated = 0u64;
                        for row in 0..height {
                            for col in 0..width {
                                if subset & (1u32 << (row * width + col)) != 0 {
                                    mask |= 1u64 << (row * 8 + col);
                                    let (new_row, new_col) = if quarter_turn {
                                        (col, width - row - 1)
                                    } else {
                                        (height - row - 1, width - col - 1)
                                    };
                                    rotated |= 1u64 << (new_row * 8 + new_col);
                                }
                            }
                        }
                        if mask == rotated
                            && full_bounding_box(mask, boundary_masks(width, height))
                            && connected(mask, width, height)
                        {
                            if has_hole(mask) {
                                expected.has_hole += 1;
                            } else {
                                expected.no_hole += 1;
                            }
                        }
                    }
                    assert_eq!(
                        symmetric_bbox(width, height, quarter_turn),
                        expected,
                        "board={width}x{height}, quarter_turn={quarter_turn}"
                    );
                }
            }
        }
    }
}
