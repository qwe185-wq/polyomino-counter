//! 旋转固定点的轨道前沿转移。
//!
//! 每次同时决定一个旋转轨道中的全部原始格子，前沿仍保存原始格子的
//! 四邻连通分量。因此商图中连通但原图中断开的形状不会被误计。

use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::io;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct State {
    // 依照当前前沿的格子顺序，0 表示空，其余为规范化连通分量编号。
    labels: Vec<usize>,
    chi: i128,
    // 旋转对称保证另一侧边界也被接触。
    bounds: u8,
    // 唯一的连通分量已离开前沿，此后只能选空轨道。
    sealed: bool,
}

struct Step {
    orbit: Vec<usize>,
    old_live: Vec<usize>,
    new_live: Vec<usize>,
    // 下标先指 old_live，再指 orbit。
    edges: Vec<(usize, usize)>,
    // 依轨道内格子顺序，每格已经处理过的共边邻格、每个顶点的邻格。
    cell_neighbors: Vec<Vec<usize>>,
    vertex_neighbors: Vec<[Vec<usize>; 4]>,
    next_positions: Vec<usize>,
    boundary_bits: u8,
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let a = self.find(a);
        let b = self.find(b);
        if a != b {
            self.parent[b] = a;
        }
    }
}

fn invalid_size() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "grid dimensions overflow address space",
    )
}

fn rotated(index: usize, width: usize, height: usize, quarter: bool) -> usize {
    let x = index % width;
    let y = index / width;
    if quarter {
        x * width + (width - 1 - y)
    } else {
        (height - 1 - y) * width + (width - 1 - x)
    }
}

fn live_after(owner: &[usize], width: usize, height: usize, step: usize) -> Vec<usize> {
    let mut live = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let cell = y * width + x;
            if owner[cell] > step {
                continue;
            }
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(height - 1);
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(width - 1);
            if (y0..=y1).any(|yy| (x0..=x1).any(|xx| owner[yy * width + xx] > step)) {
                live.push(cell);
            }
        }
    }
    live
}

fn make_steps(width: usize, height: usize, quarter: bool) -> Vec<Step> {
    let cells = width * height;
    let mut seen = vec![false; cells];
    let mut orbits = Vec::new();
    for seed in 0..cells {
        if seen[seed] {
            continue;
        }
        let mut orbit = Vec::new();
        let mut cell = seed;
        loop {
            if seen[cell] {
                break;
            }
            seen[cell] = true;
            orbit.push(cell);
            cell = rotated(cell, width, height, quarter);
        }
        orbits.push(orbit);
    }
    let mut owner = vec![0usize; cells];
    for (i, orbit) in orbits.iter().enumerate() {
        for &cell in orbit {
            owner[cell] = i;
        }
    }

    let mut steps = Vec::with_capacity(orbits.len());
    let mut old_live = Vec::new();
    for (i, orbit) in orbits.into_iter().enumerate() {
        let new_live = live_after(&owner, width, height, i);
        let mut positions = vec![usize::MAX; cells];
        for (j, &cell) in old_live.iter().chain(orbit.iter()).enumerate() {
            positions[cell] = j;
        }

        let mut edges = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let cell = y * width + x;
                if x + 1 < width {
                    let other = cell + 1;
                    if owner[cell].max(owner[other]) == i {
                        edges.push((positions[cell], positions[other]));
                    }
                }
                if y + 1 < height {
                    let other = cell + width;
                    if owner[cell].max(owner[other]) == i {
                        edges.push((positions[cell], positions[other]));
                    }
                }
            }
        }
        let mut cell_neighbors = Vec::with_capacity(orbit.len());
        let mut vertex_neighbors = Vec::with_capacity(orbit.len());
        for (within, &cell) in orbit.iter().enumerate() {
            let x = cell % width;
            let y = cell / width;
            let was_processed = |neighbor: usize| {
                owner[neighbor] < i
                    || (owner[neighbor] == i && positions[neighbor] < old_live.len() + within)
            };
            let mut near = Vec::new();
            for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                if nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height {
                    let neighbor = ny as usize * width + nx as usize;
                    if was_processed(neighbor) {
                        near.push(positions[neighbor]);
                    }
                }
            }
            cell_neighbors.push(near);
            let around = std::array::from_fn(|corner| {
                let vx = x + (corner & 1);
                let vy = y + (corner >> 1);
                let mut adjacent = Vec::new();
                for ny in vy.saturating_sub(1)..=vy.min(height - 1) {
                    for nx in vx.saturating_sub(1)..=vx.min(width - 1) {
                        let neighbor = ny * width + nx;
                        if neighbor != cell && was_processed(neighbor) {
                            adjacent.push(positions[neighbor]);
                        }
                    }
                }
                adjacent
            });
            vertex_neighbors.push(around);
        }
        let next_positions = new_live.iter().map(|&cell| positions[cell]).collect();
        let mut boundary_bits = 0;
        for &cell in &orbit {
            if cell % width == 0 {
                boundary_bits |= 1;
            }
            if cell / width == 0 {
                boundary_bits |= 2;
            }
        }
        steps.push(Step {
            orbit,
            old_live: std::mem::take(&mut old_live),
            new_live: new_live.clone(),
            edges,
            cell_neighbors,
            vertex_neighbors,
            next_positions,
            boundary_bits,
        });
        old_live = new_live;
    }
    steps
}

fn advance(state: &State, step: &Step, occupied_orbit: bool) -> Option<State> {
    if state.sealed && occupied_orbit {
        return None;
    }
    debug_assert_eq!(state.labels.len(), step.old_live.len());
    let old_len = step.old_live.len();
    let mut occupied = Vec::with_capacity(old_len + step.orbit.len());
    occupied.extend(state.labels.iter().map(|&label| label != 0));
    occupied.extend(std::iter::repeat(false).take(step.orbit.len()));

    let mut dsu = DisjointSet::new(occupied.len());
    let mut first_label = FxHashMap::default();
    for (j, &label) in state.labels.iter().enumerate() {
        if label != 0 {
            if let Some(&first) = first_label.get(&label) {
                dsu.union(first, j);
            } else {
                first_label.insert(label, j);
            }
        }
    }
    let mut chi = state.chi;
    if occupied_orbit {
        for within in 0..step.orbit.len() {
            let shared_edges = step.cell_neighbors[within]
                .iter()
                .filter(|&&j| occupied[j])
                .count();
            let new_vertices = step.vertex_neighbors[within]
                .iter()
                .filter(|neighbors| neighbors.iter().all(|&j| !occupied[j]))
                .count();
            chi += 1 - (4 - shared_edges) as i128 + new_vertices as i128;
            occupied[old_len + within] = true;
        }
    }
    for &(a, b) in &step.edges {
        if occupied[a] && occupied[b] {
            dsu.union(a, b);
        }
    }

    let mut next_labels = Vec::with_capacity(step.new_live.len());
    let mut canonical = FxHashMap::default();
    for &j in &step.next_positions {
        if occupied[j] {
            let root = dsu.find(j);
            let next = canonical.len() + 1;
            next_labels.push(*canonical.entry(root).or_insert(next));
        } else {
            next_labels.push(0);
        }
    }
    let mut all_roots = FxHashMap::default();
    for j in 0..occupied.len() {
        if occupied[j] {
            all_roots.insert(dsu.find(j), ());
        }
    }
    let closed = all_roots.keys().any(|root| !canonical.contains_key(root));
    if closed && (all_roots.len() != 1 || !canonical.is_empty()) {
        return None;
    }
    Some(State {
        labels: next_labels,
        chi,
        bounds: state.bounds
            | if occupied_orbit {
                step.boundary_bits
            } else {
                0
            },
        sealed: state.sealed || closed,
    })
}

/// 计算精确包围盒为 `width × height`、在半周或四分之一周旋转下不变的
/// 非空四邻连通格子集合，分别返回无洞与有洞计数。
pub fn count_fixed(width: usize, height: usize, quarter: bool) -> io::Result<(BigUint, BigUint)> {
    let zero = BigUint::default();
    if width == 0 || height == 0 || (quarter && width != height) {
        return Ok((zero.clone(), zero));
    }
    width.checked_mul(height).ok_or_else(invalid_size)?;
    if width > isize::MAX as usize || height > isize::MAX as usize {
        return Err(invalid_size());
    }
    let steps = make_steps(width, height, quarter);
    let mut states = FxHashMap::default();
    states.insert(
        State {
            labels: Vec::new(),
            chi: 0,
            bounds: 0,
            sealed: false,
        },
        BigUint::from(1u8),
    );
    for step in &steps {
        let mut next = FxHashMap::default();
        for (state, ways) in &states {
            for occupied in [false, true] {
                if let Some(after) = advance(state, step, occupied) {
                    *next.entry(after).or_insert_with(BigUint::default) += ways;
                }
            }
        }
        states = next;
    }
    let mut no_hole = BigUint::default();
    let mut has_hole = BigUint::default();
    for (state, ways) in states {
        if state.sealed && state.bounds == 3 {
            if state.chi == 1 {
                no_hole += ways;
            } else if state.chi <= 0 {
                has_hole += ways;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "connected final state has Euler characteristic greater than one",
                ));
            }
        }
    }
    Ok((no_hole, has_hole))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(width: usize, height: usize, quarter: bool) -> (u64, u64) {
        let cells = width * height;
        assert!(cells <= 16);
        let mut no_hole = 0;
        let mut has_hole = 0;
        for mask in 1u64..(1u64 << cells) {
            if (0..cells)
                .any(|c| ((mask >> c) & 1) != ((mask >> rotated(c, width, height, quarter)) & 1))
            {
                continue;
            }
            let selected: Vec<_> = (0..cells).filter(|&c| mask & (1 << c) != 0).collect();
            if !selected.iter().any(|&c| c % width == 0)
                || !selected.iter().any(|&c| c % width == width - 1)
                || !selected.iter().any(|&c| c / width == 0)
                || !selected.iter().any(|&c| c / width == height - 1)
            {
                continue;
            }
            let mut reached = 1u64 << selected[0];
            loop {
                let old = reached;
                for &c in &selected {
                    if reached & (1 << c) == 0 {
                        continue;
                    }
                    let x = c % width;
                    let y = c / width;
                    for n in [
                        if x > 0 { Some(c - 1) } else { None },
                        if x + 1 < width { Some(c + 1) } else { None },
                        if y > 0 { Some(c - width) } else { None },
                        if y + 1 < height {
                            Some(c + width)
                        } else {
                            None
                        },
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if mask & (1 << n) != 0 {
                            reached |= 1 << n;
                        }
                    }
                }
                if reached == old {
                    break;
                }
            }
            if reached != mask {
                continue;
            }
            // 独立于 DP 欧拉量：直接枚举背景四邻分量。
            let mut unseen = (!mask) & ((1u64 << cells) - 1);
            let mut holes = 0;
            while unseen != 0 {
                let first = unseen.trailing_zeros() as usize;
                unseen &= !(1u64 << first);
                let mut stack = vec![first];
                let mut touches_outside = false;
                while let Some(c) = stack.pop() {
                    let x = c % width;
                    let y = c / width;
                    if x == 0 || x + 1 == width || y == 0 || y + 1 == height {
                        touches_outside = true;
                    }
                    for n in [
                        if x > 0 { Some(c - 1) } else { None },
                        if x + 1 < width { Some(c + 1) } else { None },
                        if y > 0 { Some(c - width) } else { None },
                        if y + 1 < height {
                            Some(c + width)
                        } else {
                            None
                        },
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if unseen & (1u64 << n) != 0 {
                            unseen &= !(1u64 << n);
                            stack.push(n);
                        }
                    }
                }
                if !touches_outside {
                    holes += 1;
                }
            }
            if holes == 0 {
                no_hole += 1;
            } else {
                has_hole += 1;
            }
        }
        (no_hole, has_hole)
    }

    #[test]
    fn matches_independent_small_subset_oracle() {
        for width in 1..=4 {
            for height in 1..=4 {
                let (a, b) = brute(width, height, false);
                let actual = count_fixed(width, height, false).unwrap();
                assert_eq!(
                    actual,
                    (BigUint::from(a), BigUint::from(b)),
                    "half {width}×{height}"
                );
                if width == height {
                    let (a, b) = brute(width, height, true);
                    let actual = count_fixed(width, height, true).unwrap();
                    assert_eq!(
                        actual,
                        (BigUint::from(a), BigUint::from(b)),
                        "quarter {width}×{height}"
                    );
                }
            }
        }
    }

    #[test]
    fn diagonal_contacts_preserve_four_background_holes() {
        // 旧的 cells−adjacent_pairs+full_2x2 在此错误地得到 χ=1。
        let rows = [".###.", "#.#.#", "#####", "#.#.#", ".###."];
        let selected: Vec<bool> = rows
            .iter()
            .flat_map(|row| row.bytes().map(|ch| ch == b'#'))
            .collect();
        let steps = make_steps(5, 5, true);
        let mut state = State {
            labels: Vec::new(),
            chi: 0,
            bounds: 0,
            sealed: false,
        };
        for step in &steps {
            let occupied = selected[step.orbit[0]];
            assert!(step.orbit.iter().all(|&cell| selected[cell] == occupied));
            state = advance(&state, step, occupied).expect("connected pattern must survive");
        }
        assert!(state.sealed);
        assert_eq!(state.chi, -3);
    }
}
