//! 任意宽度的紧包围盒枚举。前沿拓扑和后缀可行性共享，位图始终随路径独立展开。

use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::io;

const ALL_SIDES: u8 = 0b1111;

#[derive(Clone, Eq, Hash, PartialEq)]
struct State {
    // 0 为空格，其余数字是触及当前前沿的四连通分量标签。
    labels: Vec<usize>,
    // 0 尚未占格；1 分量仍在前沿；2 唯一分量已关闭。
    phase: u8,
}

impl State {
    fn empty(width: usize) -> Self {
        Self {
            labels: vec![0; width],
            phase: 0,
        }
    }

    fn accepting(&self) -> bool {
        self.phase == 2
            || (self.phase == 1
                && self.labels.iter().any(|&label| label != 0)
                && self.labels.iter().all(|&label| label <= 1))
    }
}

fn canonicalize(labels: &mut [usize]) {
    let mut rename = vec![0; labels.len() + 2];
    let mut next = 1;
    for label in labels {
        if *label != 0 {
            if rename[*label] == 0 {
                rename[*label] = next;
                next += 1;
            }
            *label = rename[*label];
        }
    }
}

fn transition(state: &State, col: usize, occupied: bool) -> Option<State> {
    if occupied && state.phase == 2 {
        return None;
    }
    let mut next = state.clone();
    let up = state.labels[col];
    let left = if col == 0 { 0 } else { state.labels[col - 1] };
    if occupied {
        let label = if left != 0 {
            left
        } else if up != 0 {
            up
        } else {
            state.labels.iter().copied().max().unwrap_or(0) + 1
        };
        next.labels[col] = label;
        if left != 0 && up != 0 && left != up {
            for existing in &mut next.labels {
                if *existing == up {
                    *existing = left;
                }
            }
        }
        next.phase = 1;
    } else {
        next.labels[col] = 0;
        if up != 0 && !next.labels.contains(&up) {
            // 离开前沿的分量无法再接回；只能关闭唯一的分量。
            if next.labels.iter().any(|&label| label != 0) {
                return None;
            }
            next.phase = 2;
        }
    }
    canonicalize(&mut next.labels);
    Some(next)
}

struct Node {
    state: State,
    col: usize,
    edge: [Option<usize>; 2],
}

fn topology_graph(width: usize) -> Vec<Node> {
    let initial = State::empty(width);
    let mut nodes = vec![Node {
        state: initial.clone(),
        col: 0,
        edge: [None; 2],
    }];
    let mut ids = FxHashMap::default();
    ids.insert((initial, 0), 0usize);
    let mut cursor = 0;
    while cursor < nodes.len() {
        let state = nodes[cursor].state.clone();
        let col = nodes[cursor].col;
        for choice in 0..2 {
            if let Some(next) = transition(&state, col, choice == 1) {
                let next_col = if col + 1 == width { 0 } else { col + 1 };
                let key = (next.clone(), next_col);
                let id = if let Some(&id) = ids.get(&key) {
                    id
                } else {
                    let id = nodes.len();
                    ids.insert(key, id);
                    nodes.push(Node {
                        state: next,
                        col: next_col,
                        edge: [None; 2],
                    });
                    id
                };
                nodes[cursor].edge[choice] = Some(id);
            }
        }
        cursor += 1;
    }
    nodes
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct Suffix {
    index: usize,
    node: usize,
    touched: u8,
}

struct BoxSearch<'a> {
    width: usize,
    height: usize,
    area: usize,
    nodes: &'a [Node],
    feasible: FxHashMap<Suffix, bool>,
}

impl<'a> BoxSearch<'a> {
    fn new(width: usize, height: usize, nodes: &'a [Node]) -> io::Result<Self> {
        let area = width
            .checked_mul(height)
            .ok_or_else(|| io::Error::other("包围盒面积溢出"))?;
        let mut search = Self {
            width,
            height,
            area,
            nodes,
            feasible: FxHashMap::default(),
        };
        search.compute_feasible();
        Ok(search)
    }

    fn next_touched(&self, index: usize, touched: u8) -> u8 {
        let row = index / self.width;
        let col = index % self.width;
        touched
            | u8::from(row == 0)
            | (u8::from(row + 1 == self.height) << 1)
            | (u8::from(col == 0) << 2)
            | (u8::from(col + 1 == self.width) << 3)
    }

    fn successors(&self, suffix: Suffix) -> [Option<Suffix>; 2] {
        let edges = self.nodes[suffix.node].edge;
        [0, 1].map(|choice| {
            edges[choice].map(|node| Suffix {
                index: suffix.index + 1,
                node,
                touched: if choice == 1 {
                    self.next_touched(suffix.index, suffix.touched)
                } else {
                    suffix.touched
                },
            })
        })
    }

    fn terminal(&self, suffix: Suffix) -> Option<bool> {
        if suffix.index == self.area {
            Some(suffix.touched == ALL_SIDES && self.nodes[suffix.node].state.accepting())
        } else if self.nodes[suffix.node].state.phase == 2 && suffix.touched != ALL_SIDES {
            Some(false)
        } else if suffix.index >= self.width && suffix.touched & 1 == 0 {
            Some(false)
        } else {
            None
        }
    }

    fn compute_feasible(&mut self) {
        let root = Suffix {
            index: 0,
            node: 0,
            touched: 0,
        };
        let mut stack = vec![(root, false)];
        while let Some((suffix, expanded)) = stack.pop() {
            if self.feasible.contains_key(&suffix) {
                continue;
            }
            if let Some(result) = self.terminal(suffix) {
                self.feasible.insert(suffix, result);
                continue;
            }
            let children = self.successors(suffix);
            if expanded {
                let result = children
                    .into_iter()
                    .flatten()
                    .any(|child| self.feasible.get(&child) == Some(&true));
                self.feasible.insert(suffix, result);
            } else {
                stack.push((suffix, true));
                for child in children.into_iter().flatten() {
                    if !self.feasible.contains_key(&child) {
                        stack.push((child, false));
                    }
                }
            }
        }
    }

    fn possible(&self, suffix: Suffix) -> bool {
        self.feasible.get(&suffix) == Some(&true)
    }
}

// 几何运算在本模块内使用动态 stride，避免旧 u64/stride8 工具的尺寸限制。
fn rotate90(mask: &BigUint, width: usize, height: usize, stride: usize) -> BigUint {
    let mut rotated = BigUint::default();
    for row in 0..height {
        for col in 0..width {
            if mask.bit((row * stride + col) as u64) {
                rotated.set_bit((col * stride + height - 1 - row) as u64, true);
            }
        }
    }
    rotated
}

fn canonical_mask(mask: &BigUint, width: usize, height: usize, stride: usize) -> bool {
    let quarter = rotate90(mask, width, height, stride);
    let half = rotate90(&quarter, height, width, stride);
    if &half < mask {
        return false;
    }
    if width != height {
        return true;
    }
    let three_quarters = rotate90(&half, width, height, stride);
    &quarter >= mask && &three_quarters >= mask
}

fn popcount(mask: &BigUint) -> u128 {
    mask.to_bytes_le()
        .iter()
        .map(|byte| byte.count_ones() as u128)
        .sum()
}

fn has_hole(mask: &BigUint, width: usize, height: usize, stride: usize) -> bool {
    if width < 3 || height < 3 {
        return false;
    }
    // Euler 的水平位移需要一列空隙；否则行末格会与下一行首格相连。
    let (cells, cell_stride): (Cow<'_, BigUint>, usize) = if stride == width {
        let padded_stride = stride + 1;
        let mut padded = BigUint::default();
        for row in 0..height {
            for col in 0..width {
                if mask.bit((row * stride + col) as u64) {
                    padded.set_bit((row * padded_stride + col) as u64, true);
                }
            }
        }
        (Cow::Owned(padded), padded_stride)
    } else {
        (Cow::Borrowed(mask), stride)
    };
    let cells = cells.as_ref();
    // 连通格子组成的闭方格复形满足 V-E+F=1-holes。
    let faces = popcount(cells);
    let edges = popcount(&(cells | (cells << cell_stride))) + popcount(&(cells | (cells << 1)));
    let vertices =
        popcount(&(cells | (cells << 1) | (cells << cell_stride) | (cells << (cell_stride + 1))));
    edges + 1 > vertices + faces
}

struct Prefix {
    suffix: Suffix,
    mask: BigUint,
}

/// 枚举单个紧包围盒，返回 [无洞, 有洞] 计数。
///
/// `width >= height`；`stride >= width` 必须与完整导出使用的全局 stride 一致。
/// 回调收到旋转类最小代表，以及 `max(width, height)` 和有洞标志。
pub fn enumerate_bbox(
    width: usize,
    height: usize,
    stride: usize,
    emit: &mut dyn FnMut(&BigUint, usize, bool) -> io::Result<()>,
) -> io::Result<[BigUint; 2]> {
    validate_bbox(width, height, stride)?;
    let nodes = topology_graph(width);
    enumerate_bbox_with_nodes(width, height, stride, &nodes, emit)
}

fn validate_bbox(width: usize, height: usize, stride: usize) -> io::Result<()> {
    if width == 0 || height == 0 || width < height || stride < width {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "无效的包围盒或 stride",
        ));
    }
    let last_bit = (width - 1)
        .checked_mul(stride)
        .and_then(|bit| bit.checked_add(height - 1))
        .ok_or_else(|| io::Error::other("掩码位数溢出"))?;
    u64::try_from(last_bit).map_err(|_| io::Error::other("掩码位数超出 BigUint 接口"))?;
    let padded_stride = stride
        .checked_add(1)
        .ok_or_else(|| io::Error::other("stride 溢出"))?;
    if stride == width && width >= 3 && height >= 3 {
        let padded_last_bit = (height - 1)
            .checked_mul(padded_stride)
            .and_then(|bit| bit.checked_add(width - 1))
            .ok_or_else(|| io::Error::other("洞检测位数溢出"))?;
        u64::try_from(padded_last_bit)
            .map_err(|_| io::Error::other("洞检测位数超出 BigUint 接口"))?;
    }
    width
        .checked_mul(height)
        .ok_or_else(|| io::Error::other("包围盒面积溢出"))?;
    Ok(())
}

fn enumerate_bbox_with_nodes(
    width: usize,
    height: usize,
    stride: usize,
    nodes: &[Node],
    emit: &mut dyn FnMut(&BigUint, usize, bool) -> io::Result<()>,
) -> io::Result<[BigUint; 2]> {
    let search = BoxSearch::new(width, height, nodes)?;
    let mut counts = [BigUint::default(), BigUint::default()];
    let root = Suffix {
        index: 0,
        node: 0,
        touched: 0,
    };
    let mut stack = vec![Prefix {
        suffix: root,
        mask: BigUint::default(),
    }];
    while let Some(prefix) = stack.pop() {
        if !search.possible(prefix.suffix) {
            continue;
        }
        if prefix.suffix.index == search.area {
            if canonical_mask(&prefix.mask, width, height, stride) {
                let hole = has_hole(&prefix.mask, width, height, stride);
                emit(&prefix.mask, width, hole)?;
                counts[usize::from(hole)] += 1u8;
            }
            continue;
        }
        let children = search.successors(prefix.suffix);
        for (choice, child) in children.into_iter().enumerate() {
            if let Some(suffix) = child.filter(|child| search.possible(*child)) {
                let mut mask = prefix.mask.clone();
                if choice == 1 {
                    let row = prefix.suffix.index / width;
                    let col = prefix.suffix.index % width;
                    mask.set_bit((row * stride + col) as u64, true);
                }
                stack.push(Prefix { suffix, mask });
            }
        }
    }
    Ok(counts)
}

/// 枚举 n≤max_n 的旋转类。每条记录使用同一 stride=max(8,max_n)。
pub fn enumerate_frontier(
    max_n: usize,
    verbose: bool,
    emit: &mut dyn FnMut(&BigUint, usize, bool) -> io::Result<()>,
) -> io::Result<Vec<crate::dynamic::RoomCount>> {
    if max_n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "max_n 必须大于零",
        ));
    }
    let stride = max_n.max(8);
    (max_n - 1)
        .checked_mul(stride)
        .and_then(|bit| bit.checked_add(max_n - 1))
        .ok_or_else(|| io::Error::other("掩码位数溢出"))?;
    let mut bins = vec![[BigUint::default(), BigUint::default()]; max_n];
    for width in 1..=max_n {
        let nodes = topology_graph(width);
        for height in 1..=width {
            validate_bbox(width, height, stride)?;
            let counts = enumerate_bbox_with_nodes(width, height, stride, &nodes, emit)?;
            bins[width - 1][0] += &counts[0];
            bins[width - 1][1] += &counts[1];
            if verbose {
                eprintln!(
                    "  [dynamic-frontier] bbox={width}x{height} accepted={}",
                    &counts[0] + &counts[1]
                );
            }
        }
    }
    let mut cumulative = [BigUint::default(), BigUint::default()];
    let mut result = Vec::with_capacity(max_n);
    for (index, exact) in bins.into_iter().enumerate() {
        cumulative[0] += exact[0].clone();
        cumulative[1] += exact[1].clone();
        let count = crate::dynamic::RoomCount {
            n: index + 1,
            total: &cumulative[0] + &cumulative[1],
            no_hole: cumulative[0].clone(),
            has_hole: cumulative[1].clone(),
        };
        if verbose {
            eprintln!(
                "n={} total={} no_hole={} has_hole={}",
                count.n, count.total, count.no_hole, count.has_hole
            );
        }
        result.push(count);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ExportManager;
    use crate::frontier_export;
    use std::collections::BTreeSet;
    use std::fs;
    use std::sync::Arc;

    fn flood_has_hole(mask: &BigUint, width: usize, height: usize, stride: usize) -> bool {
        let padded_width = width + 2;
        let padded_height = height + 2;
        let mut seen = vec![false; padded_width * padded_height];
        let mut stack = vec![(0usize, 0usize)];
        while let Some((row, col)) = stack.pop() {
            let index = row * padded_width + col;
            if seen[index] {
                continue;
            }
            seen[index] = true;
            for (next_row, next_col) in [
                row.checked_sub(1).map(|next| (next, col)),
                (row + 1 < padded_height).then_some((row + 1, col)),
                col.checked_sub(1).map(|next| (row, next)),
                (col + 1 < padded_width).then_some((row, col + 1)),
            ]
            .into_iter()
            .flatten()
            {
                let cell = next_row > 0
                    && next_row <= height
                    && next_col > 0
                    && next_col <= width
                    && mask.bit(((next_row - 1) * stride + next_col - 1) as u64);
                if !cell && !seen[next_row * padded_width + next_col] {
                    stack.push((next_row, next_col));
                }
            }
        }
        (0..height).any(|row| {
            (0..width).any(|col| {
                !mask.bit((row * stride + col) as u64) && !seen[(row + 1) * padded_width + col + 1]
            })
        })
    }

    fn mask_from_cells(
        width: usize,
        height: usize,
        stride: usize,
        mut keep: impl FnMut(usize, usize) -> bool,
    ) -> BigUint {
        let mut mask = BigUint::default();
        for row in 0..height {
            for col in 0..width {
                if keep(row, col) {
                    mask.set_bit((row * stride + col) as u64, true);
                }
            }
        }
        mask
    }

    #[test]
    fn hole_detection_matches_background_flood_with_full_stride() {
        for side in [8usize, 9] {
            let stride = side;
            let full = mask_from_cells(side, side, stride, |_, _| true);
            let missing_corner =
                mask_from_cells(side, side, stride, |row, col| row != 0 || col != 0);
            let ring = mask_from_cells(side, side, stride, |row, col| {
                row == 0 || row + 1 == side || col == 0 || col + 1 == side
            });
            for (name, mask) in [
                ("full", full),
                ("missing_corner", missing_corner),
                ("ring", ring),
            ] {
                assert_eq!(
                    has_hole(&mask, side, side, stride),
                    flood_has_hole(&mask, side, side, stride),
                    "{name} side={side}"
                );
            }
            // 从实心图上随机删除非边界格，只保留一个连通的外框。
            let mut seed = 0x9e37_79b9_7f4a_7c15u64 ^ side as u64;
            for case in 0..128 {
                let mask = mask_from_cells(side, side, stride, |row, col| {
                    if row == 0 || row + 1 == side || col == 0 || col + 1 == side {
                        true
                    } else {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed & 3 != 0
                    }
                });
                assert_eq!(
                    has_hole(&mask, side, side, stride),
                    flood_has_hole(&mask, side, side, stride),
                    "side={side} case={case}"
                );
            }
        }
    }

    #[test]
    fn n4_masks_and_holes_match_fixed_frontier() {
        let mut observed = BTreeSet::new();
        let mut emit = |mask: &BigUint, dimension: usize, hole: bool| {
            observed.insert((mask.clone(), dimension, hole));
            Ok(())
        };
        let counts = enumerate_frontier(4, false, &mut emit).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "room-count-dynamic-frontier-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let export = Arc::new(ExportManager::new(&dir).unwrap());
        let fixed = frontier_export::enumerate_frontier(4, false, Some(export.clone())).unwrap();
        drop(export);
        for (actual, reference) in counts.iter().zip(fixed) {
            assert_eq!(actual.total, BigUint::from(reference.total));
            assert_eq!(actual.no_hole, BigUint::from(reference.no_hole));
            assert_eq!(actual.has_hole, BigUint::from(reference.has_hole));
        }
        let mut expected = BTreeSet::new();
        for (category, hole) in [("no_holes", false), ("with_holes", true)] {
            for dimension in 1..=4 {
                let path = dir
                    .join(category)
                    .join(format!("n{dimension:02}_fixed"))
                    .join("shapes_000001.bin");
                if path.exists() {
                    let bytes = fs::read(path).unwrap();
                    assert_eq!(bytes.len() % 8, 0);
                    for chunk in bytes.chunks_exact(8) {
                        expected.insert((
                            BigUint::from(u64::from_le_bytes(chunk.try_into().unwrap())),
                            dimension,
                            hole,
                        ));
                    }
                }
            }
        }
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(observed, expected);
        assert_eq!(observed.len(), 2404);
    }

    #[test]
    fn narrow_box_and_bits_above_u64() {
        let mut masks = Vec::new();
        let counts = enumerate_bbox(9, 1, 9, &mut |mask, _, _| {
            masks.push(mask.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(counts, [BigUint::from(1u8), BigUint::default()]);
        assert_eq!(masks.len(), 1);
        let upright = rotate90(&masks[0], 9, 1, 9);
        assert!(upright.bit(72));
        assert_eq!(rotate90(&upright, 1, 9, 9), masks[0]);
        assert!(!has_hole(&upright, 1, 9, 9));

        let mut ring = BigUint::default();
        for col in 0..9 {
            ring.set_bit(col, true);
            ring.set_bit(8 * 9 + col, true);
        }
        for row in 1..8 {
            ring.set_bit((row * 9) as u64, true);
            ring.set_bit((row * 9 + 8) as u64, true);
        }
        assert!(ring.bit(80));
        assert!(has_hole(&ring, 9, 9, 9));
        assert!(canonical_mask(&ring, 9, 9, 9));
    }

    #[test]
    fn callback_error_propagates() {
        let error = enumerate_bbox(1, 1, 8, &mut |_, _, _| Err(io::Error::other("stop")));
        assert_eq!(error.unwrap_err().to_string(), "stop");
    }
}
