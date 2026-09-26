//! 保持行主序的旋转电压商图前沿 DP。
//!
//! 自由轨道是商图顶点。格子四邻边保留其 sheet 差的奇偶电压，
//! 因此商图连通之外，还能判断提升后的原格子是否真正连通。

use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::io;
use std::time::Instant;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct State {
    // 0 为空；其余为按当前前沿顺序规范化的商图连通分量。
    labels: Vec<usize>,
    // 未有见证时，分量内相对于首个前沿顶点的 Z2 势。
    parity: Vec<u8>,
    witness: bool,
    chi: i128,
    bounds: u8,
    sealed: bool,
}

struct Step {
    orbit: Vec<usize>,
    old_live_cells: Vec<usize>,
    old_cell_positions: Vec<usize>,
    old_orbits: Vec<usize>,
    next_orbits: Vec<usize>,
    next_positions: Vec<usize>,
    // 商图边允许自环及不同电压的平行边。
    edges: Vec<(usize, usize, u8)>,
    cell_neighbors: Vec<Vec<usize>>,
    vertex_neighbors: Vec<[Vec<usize>; 4]>,
    boundary_bits: u8,
}

struct Geometry {
    steps: Vec<Step>,
    owner: Vec<usize>,
}

struct Branch {
    pattern: usize,
    demands: Vec<Option<bool>>,
    track_chi: bool,
    // 已预设并在对应步骤强制兑现的中央连通锚点。
    assumed_witness: bool,
}

struct DisjointSet {
    parent: Vec<usize>,
    xor_parent: Vec<u8>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            xor_parent: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> (usize, u8) {
        if self.parent[x] == x {
            return (x, 0);
        }
        let (root, delta) = self.find(self.parent[x]);
        self.xor_parent[x] ^= delta;
        self.parent[x] = root;
        (root, self.xor_parent[x])
    }

    // 要求势(a) xor 势(b) = voltage；返回是否出现奇电压闭路。
    fn union(&mut self, a: usize, b: usize, voltage: u8) -> bool {
        let (ra, pa) = self.find(a);
        let (rb, pb) = self.find(b);
        if ra == rb {
            return pa ^ pb != voltage;
        }
        self.parent[rb] = ra;
        self.xor_parent[rb] = pa ^ pb ^ voltage;
        false
    }

    fn union_plain(&mut self, a: usize, b: usize) {
        let (ra, _) = self.find(a);
        let (rb, _) = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
            self.xor_parent[rb] = 0;
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

fn unique_owners(cells: &[usize], owner: &[usize], orbit_count: usize) -> Vec<usize> {
    let mut seen = vec![false; orbit_count];
    let mut result = Vec::new();
    for &cell in cells {
        let orbit = owner[cell];
        if !seen[orbit] {
            seen[orbit] = true;
            result.push(orbit);
        }
    }
    result
}

fn make_geometry(width: usize, height: usize, quarter: bool) -> Geometry {
    let cells = width * height;
    let mut owner = vec![usize::MAX; cells];
    let mut sheet = vec![0u8; cells];
    let mut orbits = Vec::new();
    for seed in 0..cells {
        if owner[seed] != usize::MAX {
            continue;
        }
        let id = orbits.len();
        let mut orbit = Vec::new();
        let mut cell = seed;
        loop {
            if owner[cell] != usize::MAX {
                break;
            }
            owner[cell] = id;
            sheet[cell] = orbit.len() as u8;
            orbit.push(cell);
            cell = rotated(cell, width, height, quarter);
        }
        orbits.push(orbit);
    }

    let mut steps = Vec::with_capacity(orbits.len());
    let mut old_live_cells = Vec::new();
    for (i, orbit) in orbits.into_iter().enumerate() {
        let old_orbits = unique_owners(&old_live_cells, &owner, cells);
        let new_live_cells = live_after(&owner, width, height, i);
        let next_orbits = unique_owners(&new_live_cells, &owner, cells);
        let mut orbit_positions = vec![usize::MAX; i + 1];
        for (j, &u) in old_orbits.iter().enumerate() {
            orbit_positions[u] = j;
        }
        orbit_positions[i] = old_orbits.len();
        let next_positions = next_orbits.iter().map(|&u| orbit_positions[u]).collect();
        let old_cell_positions = old_live_cells
            .iter()
            .map(|&cell| orbit_positions[owner[cell]])
            .collect();

        let mut cell_positions = vec![usize::MAX; cells];
        for (j, &cell) in old_live_cells.iter().chain(orbit.iter()).enumerate() {
            cell_positions[cell] = j;
        }
        let mut edges = Vec::new();
        for &cell in &orbit {
            let x = cell % width;
            let y = cell / width;
            for neighbor in [
                if x > 0 { Some(cell - 1) } else { None },
                if x + 1 < width { Some(cell + 1) } else { None },
                if y > 0 { Some(cell - width) } else { None },
                if y + 1 < height {
                    Some(cell + width)
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            {
                if owner[neighbor] < i || (owner[neighbor] == i && cell < neighbor) {
                    let a = orbit_positions[owner[cell]];
                    let b = orbit_positions[owner[neighbor]];
                    edges.push((a, b, (sheet[cell] ^ sheet[neighbor]) & 1));
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
                    || (owner[neighbor] == i
                        && cell_positions[neighbor] < old_live_cells.len() + within)
            };
            let mut near = Vec::new();
            for neighbor in [
                if x > 0 { Some(cell - 1) } else { None },
                if x + 1 < width { Some(cell + 1) } else { None },
                if y > 0 { Some(cell - width) } else { None },
                if y + 1 < height {
                    Some(cell + width)
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            {
                if was_processed(neighbor) {
                    near.push(cell_positions[neighbor]);
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
                            adjacent.push(cell_positions[neighbor]);
                        }
                    }
                }
                adjacent
            });
            vertex_neighbors.push(around);
        }
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
            old_live_cells: std::mem::take(&mut old_live_cells),
            old_cell_positions,
            old_orbits,
            next_orbits,
            next_positions,
            edges,
            cell_neighbors,
            vertex_neighbors,
            boundary_bits,
        });
        old_live_cells = new_live_cells;
    }
    Geometry { steps, owner }
}

fn branches(
    width: usize,
    height: usize,
    quarter: bool,
    owner: &[usize],
    orbit_count: usize,
) -> Vec<Branch> {
    let xs: Vec<usize> = if width % 2 == 0 {
        vec![width / 2 - 1, width / 2]
    } else {
        vec![width / 2]
    };
    let ys: Vec<usize> = if height % 2 == 0 {
        vec![height / 2 - 1, height / 2]
    } else {
        vec![height / 2]
    };
    let mut centers = Vec::new();
    for y in ys {
        for &x in &xs {
            let u = owner[y * width + x];
            if !centers.contains(&u) {
                centers.push(u);
            }
        }
    }
    debug_assert!(centers.len() == 1 || (!quarter && centers.len() == 2));
    (0..(1usize << centers.len()))
        .map(|pattern| {
            let mut demands = vec![None; orbit_count];
            for (j, &u) in centers.iter().enumerate() {
                demands[u] = Some(pattern & (1 << j) != 0);
            }
            Branch {
                pattern,
                demands,
                track_chi: pattern != 0,
                assumed_witness: pattern == (1 << centers.len()) - 1,
            }
        })
        .collect()
}

fn advance(state: &State, step: &Step, occupied_orbit: bool, track_chi: bool) -> Option<State> {
    if state.sealed && occupied_orbit {
        return None;
    }
    debug_assert_eq!(state.labels.len(), step.old_orbits.len());
    let old_len = step.old_orbits.len();
    let mut occupied = state.labels.iter().map(|&v| v != 0).collect::<Vec<_>>();
    occupied.push(occupied_orbit);
    let mut dsu = DisjointSet::new(occupied.len());
    let mut first_label = FxHashMap::default();
    for (j, &label) in state.labels.iter().enumerate() {
        if label != 0 {
            if let Some(&first) = first_label.get(&label) {
                if state.witness {
                    dsu.union_plain(first, j);
                } else {
                    let odd = dsu.union(first, j, state.parity[first] ^ state.parity[j]);
                    debug_assert!(!odd);
                }
            } else {
                first_label.insert(label, j);
            }
        }
    }
    let mut chi = state.chi;
    if occupied_orbit && track_chi {
        let mut occupied_cell = step
            .old_cell_positions
            .iter()
            .map(|&j| occupied[j])
            .collect::<Vec<_>>();
        occupied_cell.resize(step.old_live_cells.len() + step.orbit.len(), false);
        for within in 0..step.orbit.len() {
            let shared_edges = step.cell_neighbors[within]
                .iter()
                .filter(|&&j| occupied_cell[j])
                .count();
            let new_vertices = step.vertex_neighbors[within]
                .iter()
                .filter(|neighbors| neighbors.iter().all(|&j| !occupied_cell[j]))
                .count();
            chi += 1 - (4 - shared_edges) as i128 + new_vertices as i128;
            occupied_cell[step.old_live_cells.len() + within] = true;
        }
    }
    let mut witness = state.witness;
    for &(a, b, voltage) in &step.edges {
        if occupied[a] && occupied[b] {
            if witness {
                dsu.union_plain(a, b);
            } else if dsu.union(a, b, voltage) {
                witness = true;
            }
        }
    }

    let mut labels = Vec::with_capacity(step.next_orbits.len());
    let mut parity = Vec::with_capacity(step.next_orbits.len());
    let mut canonical: FxHashMap<usize, (usize, u8)> = FxHashMap::default();
    for &j in &step.next_positions {
        if !occupied[j] {
            labels.push(0);
            parity.push(0);
            continue;
        }
        let (root, potential) = dsu.find(j);
        let next = canonical.len() + 1;
        let (label, first_potential) = *canonical.entry(root).or_insert((next, potential));
        labels.push(label);
        parity.push(if witness {
            0
        } else {
            potential ^ first_potential
        });
    }
    let mut all_roots = FxHashMap::default();
    for (j, &selected) in occupied.iter().enumerate() {
        if selected {
            all_roots.insert(dsu.find(j).0, ());
        }
    }
    let closed = all_roots.keys().any(|root| !canonical.contains_key(root));
    if closed && (all_roots.len() != 1 || !canonical.is_empty() || !witness) {
        return None;
    }
    Some(State {
        labels,
        parity,
        witness,
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
/// 非空四邻连通格子集合，返回无洞与有洞计数。
pub fn count_fixed(width: usize, height: usize, quarter: bool) -> io::Result<(BigUint, BigUint)> {
    let zero = BigUint::default();
    if width == 0 || height == 0 || (quarter && width != height) {
        return Ok((zero.clone(), zero));
    }
    width.checked_mul(height).ok_or_else(invalid_size)?;
    if width > isize::MAX as usize || height > isize::MAX as usize {
        return Err(invalid_size());
    }
    let geometry = make_geometry(width, height, quarter);
    let profile = std::env::var_os("ROOM_COUNT_PROFILE")
        .is_some_and(|value| !value.is_empty() && value.to_string_lossy() != "0");
    let mut no_hole = BigUint::default();
    let mut has_hole = BigUint::default();
    for branch in branches(
        width,
        height,
        quarter,
        &geometry.owner,
        geometry.steps.len(),
    ) {
        let branch_start = profile.then(Instant::now);
        let mut states = FxHashMap::default();
        states.insert(
            State {
                labels: Vec::new(),
                parity: Vec::new(),
                witness: branch.assumed_witness,
                chi: 0,
                bounds: 0,
                sealed: false,
            },
            BigUint::from(1u8),
        );
        for (i, step) in geometry.steps.iter().enumerate() {
            let step_start = profile.then(Instant::now);
            let states_in = states.len();
            let mut next = FxHashMap::default();
            for (state, ways) in &states {
                for occupied in [false, true] {
                    if branch.demands[i].is_some_and(|required| required != occupied) {
                        continue;
                    }
                    if let Some(after) = advance(state, step, occupied, branch.track_chi) {
                        *next.entry(after).or_insert_with(BigUint::default) += ways;
                    }
                }
            }
            states = next;
            if let Some(start) = step_start {
                eprintln!(
                    "quotient_step pattern={} track_chi={} anchor={} step={} states_in={} states_out={} frontier={} elapsed_seconds={:.9}",
                    branch.pattern,
                    branch.track_chi,
                    branch.assumed_witness,
                    i + 1,
                    states_in,
                    states.len(),
                    step.next_orbits.len(),
                    start.elapsed().as_secs_f64(),
                );
            }
        }
        for (state, ways) in states {
            if state.sealed && state.bounds == 3 && state.witness {
                if !branch.track_chi {
                    has_hole += ways;
                } else if state.chi == 1 {
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
        if let Some(start) = branch_start {
            eprintln!(
                "quotient_branch pattern={} track_chi={} anchor={} elapsed_seconds={:.9}",
                branch.pattern,
                branch.track_chi,
                branch.assumed_witness,
                start.elapsed().as_secs_f64(),
            );
        }
    }
    Ok((no_hole, has_hole))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neighbors(cell: usize, width: usize, height: usize) -> impl Iterator<Item = usize> {
        let x = cell % width;
        let y = cell / width;
        [
            if x > 0 { Some(cell - 1) } else { None },
            if x + 1 < width { Some(cell + 1) } else { None },
            if y > 0 { Some(cell - width) } else { None },
            if y + 1 < height {
                Some(cell + width)
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
    }

    fn flood(selected: &[bool], width: usize, height: usize, foreground: bool) -> Vec<Vec<usize>> {
        let mut visited = vec![false; selected.len()];
        let mut components = Vec::new();
        for start in 0..selected.len() {
            if visited[start] || selected[start] != foreground {
                continue;
            }
            visited[start] = true;
            let mut stack = vec![start];
            let mut component = Vec::new();
            while let Some(cell) = stack.pop() {
                component.push(cell);
                for next in neighbors(cell, width, height) {
                    if !visited[next] && selected[next] == foreground {
                        visited[next] = true;
                        stack.push(next);
                    }
                }
            }
            components.push(component);
        }
        components
    }

    fn brute(width: usize, height: usize, quarter: bool) -> (BigUint, BigUint) {
        let geometry = make_geometry(width, height, quarter);
        let count = geometry.steps.len();
        assert!(count <= 13);
        let mut result = (BigUint::default(), BigUint::default());
        for mask in 1usize..(1usize << count) {
            let mut selected = vec![false; width * height];
            for (i, step) in geometry.steps.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    for &cell in &step.orbit {
                        selected[cell] = true;
                    }
                }
            }
            if !(0..width).any(|x| selected[x])
                || !(0..width).any(|x| selected[(height - 1) * width + x])
                || !(0..height).any(|y| selected[y * width])
                || !(0..height).any(|y| selected[y * width + width - 1])
            {
                continue;
            }
            if flood(&selected, width, height, true).len() != 1 {
                continue;
            }
            let holes = flood(&selected, width, height, false)
                .iter()
                .filter(|component| {
                    component.iter().all(|&cell| {
                        let x = cell % width;
                        let y = cell / width;
                        x > 0 && x + 1 < width && y > 0 && y + 1 < height
                    })
                })
                .count();
            if holes == 0 {
                result.0 += 1u8;
            } else {
                result.1 += 1u8;
            }
        }
        result
    }

    #[test]
    fn matches_independent_original_grid_flood() {
        for width in 1..=4 {
            for height in 1..=4 {
                assert_eq!(
                    count_fixed(width, height, false).unwrap(),
                    brute(width, height, false),
                    "half {width}×{height}"
                );
                if width == height {
                    assert_eq!(
                        count_fixed(width, height, true).unwrap(),
                        brute(width, height, true),
                        "quarter {width}×{height}"
                    );
                }
            }
        }
        for quarter in [false, true] {
            assert_eq!(
                count_fixed(5, 5, quarter).unwrap(),
                brute(5, 5, quarter),
                "5×5 quarter={quarter}"
            );
        }
        assert_eq!(count_fixed(1, 5, false).unwrap(), brute(1, 5, false));
        assert_eq!(count_fixed(5, 1, false).unwrap(), brute(5, 1, false));
        assert_eq!(
            count_fixed(2, 3, true).unwrap(),
            (BigUint::default(), BigUint::default())
        );
    }

    #[test]
    fn central_branches_are_disjoint_and_identify_only_full_anchor() {
        let geometry = make_geometry(4, 4, false);
        let all = branches(4, 4, false, &geometry.owner, geometry.steps.len());
        assert_eq!(all.len(), 4);
        let patterns = all
            .iter()
            .map(|branch| {
                (
                    branch.demands.iter().filter(|d| **d == Some(true)).count(),
                    branch.track_chi,
                    branch.assumed_witness,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            patterns,
            vec![
                (0, false, false),
                (1, true, false),
                (1, true, false),
                (2, true, true)
            ]
        );
        for (width, height, quarter) in [(3, 3, false), (2, 3, false), (3, 3, true), (4, 4, true)] {
            let geometry = make_geometry(width, height, quarter);
            let all = branches(
                width,
                height,
                quarter,
                &geometry.owner,
                geometry.steps.len(),
            );
            assert_eq!(all.len(), 2);
            assert!(!all[0].track_chi && !all[0].assumed_witness);
            assert!(all[1].track_chi && all[1].assumed_witness);
        }
    }

    #[test]
    fn voltage_constraints_require_an_odd_cycle() {
        let mut dsu = DisjointSet::new(2);
        assert!(!dsu.union(0, 0, 0)); // C4 电压 2 投到 parity 0，不能作见证。
        assert!(!dsu.union(0, 1, 0));
        assert!(dsu.union(0, 1, 1)); // 平行边 0、1 共同产生见证。
        assert!(dsu.union(1, 1, 1)); // 奇电压自环直接产生见证。
    }

    #[test]
    fn plain_quotient_path_does_not_imply_original_connectivity() {
        let width = 3;
        let height = 3;
        let selected = [true, true, true, false, false, false, true, true, true];
        assert_eq!(flood(&selected, width, height, true).len(), 2);
        let geometry = make_geometry(width, height, false);
        let chosen = geometry
            .steps
            .iter()
            .map(|step| selected[step.orbit[0]])
            .collect::<Vec<_>>();
        let mut dsu = DisjointSet::new(chosen.len());
        let mut odd_cycle = false;
        for (i, step) in geometry.steps.iter().enumerate() {
            for &(a, b, voltage) in &step.edges {
                let u = if a == step.old_orbits.len() {
                    i
                } else {
                    step.old_orbits[a]
                };
                let v = if b == step.old_orbits.len() {
                    i
                } else {
                    step.old_orbits[b]
                };
                if chosen[u] && chosen[v] {
                    odd_cycle |= dsu.union(u, v, voltage);
                }
            }
        }
        let roots = chosen
            .iter()
            .enumerate()
            .filter(|(_, yes)| **yes)
            .map(|(i, _)| dsu.find(i).0)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(roots.len(), 1, "plain quotient is connected");
        assert!(
            !odd_cycle,
            "lift has two components without odd voltage cycle"
        );
    }

    #[test]
    fn diagonal_contacts_keep_four_background_holes() {
        let rows = [".###.", "#.#.#", "#####", "#.#.#", ".###."];
        let selected = rows
            .iter()
            .flat_map(|row| row.bytes().map(|ch| ch == b'#'))
            .collect::<Vec<_>>();
        let holes = flood(&selected, 5, 5, false)
            .iter()
            .filter(|component| {
                component.iter().all(|&cell| {
                    let x = cell % 5;
                    let y = cell / 5;
                    x > 0 && x < 4 && y > 0 && y < 4
                })
            })
            .count();
        assert_eq!(holes, 4);
        let geometry = make_geometry(5, 5, true);
        let mut state = State {
            labels: Vec::new(),
            parity: Vec::new(),
            witness: false,
            chi: 0,
            bounds: 0,
            sealed: false,
        };
        for step in &geometry.steps {
            let occupied = selected[step.orbit[0]];
            assert!(step.orbit.iter().all(|&cell| selected[cell] == occupied));
            state = advance(&state, step, occupied, true).expect("connected pattern must survive");
        }
        assert!(state.sealed && state.witness);
        assert_eq!(state.chi, -3);
    }
}
