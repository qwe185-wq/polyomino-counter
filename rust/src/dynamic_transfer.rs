//! 无固定尺寸上限的前沿计数与旋转 Burnside 计数。

use crate::dynamic::RoomCount;
use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::io::{Error, ErrorKind, Result};

fn overflow(what: &str) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("{what} 超出 usize 可表示范围"),
    )
}

fn filled_vec<T: Clone>(len: usize, value: T) -> Result<Vec<T>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(len)
        .map_err(|e| Error::other(e.to_string()))?;
    result.resize(len, value);
    Ok(result)
}

#[derive(Clone, Default, Debug, Eq, PartialEq)]
struct Counts {
    no_hole: BigUint,
    has_hole: BigUint,
}

impl Counts {
    fn add(&mut self, other: &Self) {
        self.no_hole += &other.no_hole;
        self.has_hole += &other.has_hole;
    }

    fn total(&self) -> BigUint {
        &self.no_hole + &self.has_hole
    }

    fn record(&mut self, chi: usize, count: &BigUint) -> Result<()> {
        if chi > 1 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "连通终态的 Euler 特征数大于 1",
            ));
        }
        if chi == 1 {
            self.no_hole += count;
        } else {
            self.has_hole += count;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct State {
    labels: Vec<usize>,
    old_left: bool,
    // 0 尚未开始，1 有活跃分量；关闭后的分量由 Closed 边单独累计。
    phase: u8,
}

fn canonicalize(labels: &mut [usize]) {
    let mut renamed = Vec::new();
    for label in labels {
        if *label != 0 {
            let next = if let Some((_, name)) = renamed.iter().find(|(old, _)| *old == *label) {
                *name
            } else {
                let name = renamed.len() + 1;
                renamed.push((*label, name));
                name
            };
            *label = next;
        }
    }
}

// χ=V-E+F 的局部变化；对角相接的顶点也必须计入。
fn transition(state: &State, col: usize, occupied: bool) -> Result<Option<(State, isize)>> {
    let width = state.labels.len();
    let mut next = state.clone();
    let left_label = if col > 0 { state.labels[col - 1] } else { 0 };
    let up_label = state.labels[col];
    let left = left_label != 0;
    let up = up_label != 0;
    let up_left = col > 0 && state.old_left;
    let up_right = col + 1 < width && state.labels[col + 1] != 0;
    next.old_left = col + 1 < width && up;
    let mut delta = 0;
    if occupied {
        let label = if left {
            left_label
        } else if up {
            up_label
        } else {
            state
                .labels
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| overflow("前沿标签"))?
        };
        next.labels[col] = label;
        if left && up && left_label != up_label {
            for existing in &mut next.labels {
                if *existing == up_label {
                    *existing = left_label;
                }
            }
        }
        let vertices = 1
            + (!left as isize)
            + (!(up || up_right) as isize)
            + (!(left || up || up_left) as isize);
        let edges = 4 - (left as isize) - (up as isize);
        delta = vertices - edges + 1;
        next.phase = 1;
    } else {
        let vanished = state.labels[col];
        next.labels[col] = 0;
        if vanished != 0 && !next.labels.contains(&vanished) {
            if next.labels.iter().any(|&label| label != 0) {
                return Ok(None);
            }
            return Ok(Some((next, 0))); // 上层识别关闭的唯一分量。
        }
    }
    canonicalize(&mut next.labels);
    Ok(Some((next, delta)))
}

#[derive(Clone, Copy)]
enum Edge {
    Invalid,
    Closed,
    Next { id: usize, delta: isize },
}

struct Node {
    state: State,
    col: usize,
    edges: [Edge; 2],
    accepts: bool,
}

impl Node {
    fn new(state: State, col: usize) -> Self {
        let accepts = state.phase == 1 && state.labels.iter().all(|&label| label <= 1);
        Self {
            state,
            col,
            edges: [Edge::Invalid; 2],
            accepts,
        }
    }
}

fn topology_graph(width: usize) -> Result<Vec<Node>> {
    let initial = State {
        labels: filled_vec(width, 0)?,
        old_left: false,
        phase: 0,
    };
    let mut nodes = vec![Node::new(initial.clone(), 0)];
    let mut ids = FxHashMap::default();
    ids.insert((initial, 0usize), 0usize);
    let mut cursor = 0;
    while cursor < nodes.len() {
        let state = nodes[cursor].state.clone();
        let col = nodes[cursor].col;
        for (branch, occupied) in [false, true].into_iter().enumerate() {
            if let Some((next, delta)) = transition(&state, col, occupied)? {
                nodes[cursor].edges[branch] =
                    if next.phase == 1 && next.labels.iter().all(|&l| l == 0) {
                        Edge::Closed
                    } else {
                        let next_col = if col + 1 == width { 0 } else { col + 1 };
                        let key = (next.clone(), next_col);
                        let id = if let Some(&id) = ids.get(&key) {
                            id
                        } else {
                            let id = nodes.len();
                            nodes
                                .try_reserve(1)
                                .map_err(|e| Error::other(e.to_string()))?;
                            ids.try_reserve(1)
                                .map_err(|e| Error::other(e.to_string()))?;
                            nodes.push(Node::new(next, next_col));
                            ids.insert(key, id);
                            id
                        };
                        Edge::Next { id, delta }
                    };
            }
        }
        cursor += 1;
    }
    Ok(nodes)
}

// χ=0 是已成洞的吸收桶。其余 χ≤前沿宽度，因为每个未闭四连通分量
// 必须触及前沿，且 χ=c8-h≤c4。空前缀由 phase=0 与吸收桶区分。
fn next_chi(chi: usize, phase: u8, delta: isize, width: usize) -> Result<usize> {
    if phase != 0 && chi == 0 {
        return Ok(0);
    }
    let next = if delta < 0 {
        chi.saturating_sub(delta.unsigned_abs())
    } else {
        chi.checked_add(delta as usize)
            .ok_or_else(|| overflow("Euler 特征数"))?
    };
    if next > width {
        return Err(Error::new(ErrorKind::InvalidData, "前沿 Euler 特征数越界"));
    }
    Ok(next)
}

fn placement_rows(width: usize, height: usize) -> Result<Vec<Counts>> {
    let row_count = height.checked_add(1).ok_or_else(|| overflow("行数"))?;
    let area = width
        .checked_mul(height)
        .ok_or_else(|| overflow("网格面积"))?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(row_count)
        .map_err(|e| Error::other(e.to_string()))?;
    rows.resize_with(row_count, Counts::default);
    if area == 0 {
        return Ok(rows);
    }
    let nodes = topology_graph(width)?;
    let mut active: FxHashMap<(usize, usize), BigUint> = FxHashMap::default();
    active.insert((0, 0), BigUint::from(1u8));
    let mut closed = Counts::default();
    for index in 0..area {
        let mut following: FxHashMap<(usize, usize), BigUint> = FxHashMap::default();
        let capacity = active
            .len()
            .checked_mul(2)
            .ok_or_else(|| overflow("DP 状态容量"))?;
        following
            .try_reserve(capacity)
            .map_err(|e| Error::other(e.to_string()))?;
        for ((id, chi), multiplicity) in active {
            for edge in nodes[id].edges {
                match edge {
                    Edge::Invalid => (),
                    Edge::Closed => closed.record(chi, &multiplicity)?,
                    Edge::Next { id: target, delta } => {
                        let bin = next_chi(chi, nodes[id].state.phase, delta, width)?;
                        *following.entry((target, bin)).or_default() += &multiplicity;
                    }
                }
            }
        }
        active = following;
        if (index + 1) % width == 0 {
            let mut count = closed.clone();
            for (&(id, chi), multiplicity) in &active {
                if nodes[id].accepts {
                    count.record(chi, multiplicity)?;
                }
            }
            rows[(index + 1) / width] = count;
        }
    }
    Ok(rows)
}

fn rotation_orbits(width: usize, height: usize, quarter: bool) -> Result<Vec<Vec<usize>>> {
    let area = width
        .checked_mul(height)
        .ok_or_else(|| overflow("网格面积"))?;
    let mut seen = filled_vec(area, false)?;
    let mut orbits = Vec::new();
    for first in 0..area {
        if seen[first] {
            continue;
        }
        let mut orbit = Vec::new();
        let mut cell = first;
        loop {
            if seen[cell] {
                break;
            }
            seen[cell] = true;
            orbit.push(cell);
            let (row, col) = (cell / width, cell % width);
            let (r, c) = if quarter {
                (col, width - 1 - row)
            } else {
                (height - 1 - row, width - 1 - col)
            };
            cell = r * width + c;
        }
        orbits.push(orbit);
    }
    Ok(orbits)
}

fn connected_small(mask: u64) -> bool {
    let mut seen = mask & mask.wrapping_neg();
    let mut frontier = seen;
    while frontier != 0 {
        let adjacent =
            ((frontier << 1) | (frontier >> 1) | (frontier << 8) | (frontier >> 8)) & mask;
        frontier = adjacent & !seen;
        seen |= frontier;
    }
    seen == mask
}

fn hole_small(mask: u64) -> bool {
    let faces = mask.count_ones();
    let edges = (mask | mask << 8).count_ones() + (mask | mask << 1).count_ones();
    let vertices = (mask | mask << 1 | mask << 8 | mask << 9).count_ones();
    edges + 1 > vertices + faces
}

fn connected_large(
    mask: &[bool],
    width: usize,
    height: usize,
    seen: &mut [bool],
    stack: &mut Vec<usize>,
) -> bool {
    let Some(start) = mask.iter().position(|&cell| cell) else {
        return false;
    };
    seen.fill(false);
    stack.clear();
    stack.push(start);
    seen[start] = true;
    let mut reached = 0usize;
    while let Some(cell) = stack.pop() {
        reached += 1;
        let (row, col) = (cell / width, cell % width);
        let neighbors = [
            (row > 0).then(|| cell - width),
            (row + 1 < height).then(|| cell + width),
            (col > 0).then(|| cell - 1),
            (col + 1 < width).then(|| cell + 1),
        ];
        for neighbor in neighbors.into_iter().flatten() {
            if mask[neighbor] && !seen[neighbor] {
                seen[neighbor] = true;
                stack.push(neighbor);
            }
        }
    }
    reached == mask.iter().filter(|&&cell| cell).count()
}

fn hole_large(mask: &[bool], width: usize, height: usize) -> Result<bool> {
    let mut chi = 0i128;
    for row in 0..height {
        for col in 0..width {
            let at = |r: usize, c: usize| mask[r * width + c];
            if !at(row, col) {
                continue;
            }
            let left = col > 0 && at(row, col - 1);
            let up = row > 0 && at(row - 1, col);
            let up_left = row > 0 && col > 0 && at(row - 1, col - 1);
            let up_right = row > 0 && col + 1 < width && at(row - 1, col + 1);
            let vertices = 1
                + (!left as isize)
                + (!(up || up_right) as isize)
                + (!(left || up || up_left) as isize);
            let edges = 4 - (left as isize) - (up as isize);
            chi = chi
                .checked_add((vertices - edges + 1) as i128)
                .ok_or_else(|| overflow("Euler 特征数"))?;
        }
    }
    Ok(chi <= 0)
}

// 二进制计数器每次进位到位置 i，Gray 序列恰好翻转第 i 个轨道。
// 计数器按轨道数量动态扩展，不计算 2^轨道数。
fn advance_gray(bits: &mut [bool]) -> Option<usize> {
    let mut i = 0;
    while i < bits.len() && bits[i] {
        bits[i] = false;
        i += 1;
    }
    if i == bits.len() {
        None
    } else {
        bits[i] = true;
        Some(i)
    }
}

fn symmetric_bbox(width: usize, height: usize, quarter: bool) -> Result<Counts> {
    if quarter && width != height {
        return Ok(Counts::default());
    }
    let orbits = rotation_orbits(width, height, quarter)?;
    let mut counter = filled_vec(orbits.len(), false)?;
    let mut counts = Counts::default();
    if width <= 7 && height <= 7 {
        let masks: Vec<u64> = orbits
            .iter()
            .map(|orbit| {
                orbit.iter().fold(0u64, |mask, &cell| {
                    mask | (1u64 << ((cell / width) * 8 + cell % width))
                })
            })
            .collect();
        let top = (1u64 << width) - 1;
        let bottom = top << ((height - 1) * 8);
        let mut left = 0u64;
        let mut right = 0u64;
        for row in 0..height {
            left |= 1u64 << (row * 8);
            right |= 1u64 << (row * 8 + width - 1);
        }
        let mut mask = 0u64;
        // 至多 7×7 的半转轨道有 25 个，候选数小于 2^25。
        let mut small_no_hole = 0u64;
        let mut small_has_hole = 0u64;
        while let Some(changed) = advance_gray(&mut counter) {
            mask ^= masks[changed];
            if mask & top != 0
                && (quarter || mask & left != 0)
                && (quarter || (mask & bottom != 0 && mask & right != 0))
                && connected_small(mask)
            {
                if hole_small(mask) {
                    small_has_hole += 1;
                } else {
                    small_no_hole += 1;
                }
            }
        }
        counts.no_hole = BigUint::from(small_no_hole);
        counts.has_hole = BigUint::from(small_has_hole);
    } else {
        let area = width
            .checked_mul(height)
            .ok_or_else(|| overflow("网格面积"))?;
        let mut mask = filled_vec(area, false)?;
        let mut seen = filled_vec(area, false)?;
        let mut stack = Vec::new();
        while let Some(changed) = advance_gray(&mut counter) {
            for &cell in &orbits[changed] {
                mask[cell] = !mask[cell];
            }
            let top = mask[..width].iter().any(|&v| v);
            let bottom = mask[area - width..].iter().any(|&v| v);
            let left = (0..height).any(|row| mask[row * width]);
            let right = (0..height).any(|row| mask[row * width + width - 1]);
            if top
                && bottom
                && left
                && right
                && connected_large(&mask, width, height, &mut seen, &mut stack)
            {
                if hole_large(&mask, width, height)? {
                    counts.has_hole += 1u32;
                } else {
                    counts.no_hole += 1u32;
                }
            }
        }
    }
    Ok(counts)
}

fn difference(square: &Counts, previous: &Counts, strip: &Counts) -> Result<Counts> {
    let subtract = |a: &BigUint, b: &BigUint, s: &BigUint| -> Result<BigUint> {
        let sum = a + b;
        let twice = s * 2u32;
        if sum < twice {
            return Err(Error::new(ErrorKind::InvalidData, "二维差分为负"));
        }
        Ok(sum - twice)
    };
    Ok(Counts {
        no_hole: subtract(&square.no_hole, &previous.no_hole, &strip.no_hole)?,
        has_hole: subtract(&square.has_hole, &previous.has_hole, &strip.has_hole)?,
    })
}

fn burnside(fixed: &Counts, half: &Counts, quarter: &Counts) -> Result<Counts> {
    let divide = |a: &BigUint, b: &BigUint, c: &BigUint| -> Result<BigUint> {
        let numerator = a + b + c * 2u32;
        if &numerator % 4u32 != BigUint::default() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Burnside 分子不能被 4 整除",
            ));
        }
        Ok(numerator / 4u32)
    };
    Ok(Counts {
        no_hole: divide(&fixed.no_hole, &half.no_hole, &quarter.no_hole)?,
        has_hole: divide(&fixed.has_hole, &half.has_hole, &quarter.has_hole)?,
    })
}

/// 返回 n=1..max_n 的 one-sided 累计结果；镜像保持不同。
pub fn enumerate_transfer(max_n: usize, verbose: bool) -> Result<Vec<RoomCount>> {
    if max_n == 0 {
        return Err(Error::new(ErrorKind::InvalidInput, "n 必须为正整数"));
    }
    max_n
        .checked_mul(max_n)
        .ok_or_else(|| overflow("最大网格面积"))?;
    let slots = max_n.checked_add(1).ok_or_else(|| overflow("尺寸数组"))?;
    let mut squares = Vec::new();
    squares
        .try_reserve_exact(slots)
        .map_err(|e| Error::other(e.to_string()))?;
    squares.resize_with(slots, Counts::default);
    let mut strips = squares.clone();
    for width in 1..=max_n {
        let height = if width < max_n { width + 1 } else { width };
        let rows = placement_rows(width, height)?;
        squares[width] = rows[width].clone();
        if width < max_n {
            strips[width + 1] = rows[width + 1].clone();
        }
    }
    let mut previous = Counts::default();
    let mut sym180 = Counts::default();
    let mut sym90 = Counts::default();
    let mut result = Vec::new();
    result
        .try_reserve_exact(max_n)
        .map_err(|e| Error::other(e.to_string()))?;
    for n in 1..=max_n {
        let fixed = difference(&squares[n], &previous, &strips[n])?;
        previous = squares[n].clone();
        for width in 1..n {
            let count = symmetric_bbox(width, n, false)?;
            sym180.add(&count);
            sym180.add(&count);
        }
        sym180.add(&symmetric_bbox(n, n, false)?);
        sym90.add(&symmetric_bbox(n, n, true)?);
        let orbit_count = burnside(&fixed, &sym180, &sym90)?;
        let count = RoomCount {
            n,
            total: orbit_count.total(),
            no_hole: orbit_count.no_hole,
            has_hole: orbit_count.has_hole,
        };
        if verbose {
            eprintln!("{count}");
        }
        result.push(count);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_counts_match_existing_transfer() {
        let actual = enumerate_transfer(5, false).unwrap();
        let expected = crate::transfer::enumerate_transfer(5, false);
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.total, BigUint::from(e.total));
            assert_eq!(a.no_hole, BigUint::from(e.no_hole));
            assert_eq!(a.has_hole, BigUint::from(e.has_hole));
        }
    }

    #[test]
    fn narrow_tall_rows_and_large_orbits() {
        for height in [8, 9] {
            let rows = placement_rows(1, height).unwrap();
            assert_eq!(
                rows[height].no_hole,
                BigUint::from(height * (height + 1) / 2)
            );
            assert_eq!(rows[height].has_hole, BigUint::default());
            assert_eq!(
                symmetric_bbox(1, height, false).unwrap().total(),
                BigUint::from(1u8)
            );
            let orbits = rotation_orbits(height, height, false).unwrap();
            assert_eq!(orbits.iter().map(Vec::len).sum::<usize>(), height * height);
            assert_eq!(orbits.len(), (height * height + 1) / 2);
        }
        for width in [8, 9] {
            let rows = placement_rows(width, 1).unwrap();
            assert_eq!(rows[1].no_hole, BigUint::from(width * (width + 1) / 2));
            assert_eq!(rows[1].has_hole, BigUint::default());
        }
        assert_eq!(rotation_orbits(8, 9, false).unwrap().len(), 36);
        assert_eq!(rotation_orbits(9, 9, true).unwrap().len(), 21);
        let mut beyond_word = vec![true; 65];
        beyond_word.push(false);
        assert_eq!(advance_gray(&mut beyond_word), Some(65));
        assert!(beyond_word[..65].iter().all(|&bit| !bit));
        assert!(beyond_word[65]);
    }

    #[test]
    fn large_bitmap_geometry_detects_hole() {
        let (width, height) = (8, 9);
        let mut ring = vec![false; width * height];
        for row in 0..height {
            for col in 0..width {
                ring[row * width + col] =
                    row == 0 || row + 1 == height || col == 0 || col + 1 == width;
            }
        }
        let mut seen = vec![false; ring.len()];
        let mut stack = Vec::new();
        assert!(connected_large(&ring, width, height, &mut seen, &mut stack));
        assert!(hole_large(&ring, width, height).unwrap());
        ring[3] = false;
        assert!(connected_large(&ring, width, height, &mut seen, &mut stack));
        assert!(!hole_large(&ring, width, height).unwrap());
    }

    #[test]
    fn bigint_counts_cross_u64() {
        let mut counts = Counts::default();
        let large = BigUint::from(u64::MAX);
        counts.record(1, &large).unwrap();
        counts.record(1, &large).unwrap();
        assert_eq!(counts.no_hole, large * 2u32);
    }

    #[test]
    fn frontier_dp_accumulates_beyond_u64() {
        // 中列全满，左右各40格任意选择，至少构造出2^80种连通形状。
        let rows = placement_rows(3, 40).unwrap();
        assert!(rows[40].total() >= (BigUint::from(1u8) << 80usize));
    }
}
