//! 保持行主序的旋转电压商图前沿 DP。
//!
//! 自由轨道是商图顶点。格子四邻边保留其 sheet 差的奇偶电压，
//! 因此商图连通之外，还能判断提升后的原格子是否真正连通。

use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::io;
use std::time::Instant;

#[cfg(test)]
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
    // 中心约束小自动机：至少一个中心占用值等于 !center_empty。
    any_centers: Vec<usize>,
    center_empty: bool,
}

// 拓扑转移与 χ、包围盒无关；把同一拓扑的数值权重放在同一桶内。
#[derive(Clone, Default, Debug, Eq, Hash, PartialEq)]
struct Topology {
    labels: Vec<usize>,
    parity: Vec<u8>,
    witness: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Values {
    chi: i128,
    bounds: u8,
    center_seen: bool,
}

trait TopologyKey: Clone + Eq + Hash {
    fn encode(topology: Topology, bits: usize) -> Self;
    fn decode(&self, len: usize, bits: usize, output: &mut Topology);
}

macro_rules! packed_topology_key {
    ($integer:ty) => {
        impl TopologyKey for $integer {
            fn encode(topology: Topology, bits: usize) -> Self {
                let mut packed = topology.witness as Self;
                for (position, (&label, &parity)) in
                    topology.labels.iter().zip(&topology.parity).enumerate()
                {
                    packed |= ((label as Self) << 1 | parity as Self) << (1 + position * bits);
                }
                packed
            }
            fn decode(&self, len: usize, bits: usize, output: &mut Topology) {
                output.labels.clear();
                output.parity.clear();
                output.witness = self & 1 != 0;
                let mask = ((1 as Self) << bits) - 1;
                for position in 0..len {
                    let entry = ((self >> (1 + position * bits)) & mask) as usize;
                    output.labels.push(entry >> 1);
                    output.parity.push((entry & 1) as u8);
                }
            }
        }
    };
}
packed_topology_key!(u64);
packed_topology_key!(u128);

impl TopologyKey for Topology {
    fn encode(topology: Topology, _bits: usize) -> Self {
        topology
    }
    fn decode(&self, _len: usize, _bits: usize, output: &mut Topology) {
        output.labels.clone_from(&self.labels);
        output.parity.clone_from(&self.parity);
        output.witness = self.witness;
    }
}

fn entry_bits(frontier_len: usize) -> usize {
    (usize::BITS - frontier_len.leading_zeros()) as usize + 1
}

trait Ways: Clone + Default {
    fn one() -> Self;
    fn add_checked(&mut self, other: &Self) -> io::Result<()>;
    fn into_biguint(self) -> BigUint;
}
macro_rules! integer_ways {
    ($integer:ty) => {
        impl Ways for $integer {
            fn one() -> Self {
                1
            }
            fn add_checked(&mut self, other: &Self) -> io::Result<()> {
                *self = self.checked_add(*other).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "quotient fixed-set count overflow",
                    )
                })?;
                Ok(())
            }
            fn into_biguint(self) -> BigUint {
                BigUint::from(self)
            }
        }
    };
}
integer_ways!(u64);
integer_ways!(u128);
impl Ways for BigUint {
    fn one() -> Self {
        BigUint::from(1u8)
    }
    fn add_checked(&mut self, other: &Self) -> io::Result<()> {
        *self += other;
        Ok(())
    }
    fn into_biguint(self) -> BigUint {
        self
    }
}

struct Transition {
    topology: Topology,
    delta_chi: i128,
    closed: bool,
}

#[derive(Default)]
struct Scratch {
    dsu: DisjointSet,
    occupied: Vec<bool>,
    occupied_cell: Vec<bool>,
    first_label: Vec<usize>,
    root_label: Vec<usize>,
    root_parity: Vec<u8>,
    root_seen: Vec<bool>,
}

#[derive(Default)]
struct DisjointSet {
    parent: Vec<usize>,
    xor_parent: Vec<u8>,
}

impl DisjointSet {
    fn reset(&mut self, n: usize) {
        self.parent.clear();
        self.parent.extend(0..n);
        self.xor_parent.clear();
        self.xor_parent.resize(n, 0);
    }

    #[cfg(test)]
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
                any_centers: Vec::new(),
                center_empty: false,
            }
        })
        .collect()
}

#[cfg(test)]
fn advance(state: &State, step: &Step, occupied_orbit: bool, track_chi: bool) -> Option<State> {
    if state.sealed && occupied_orbit {
        return None;
    }
    debug_assert_eq!(state.labels.len(), step.old_orbits.len());
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

fn advance_topology(
    topology: &Topology,
    step: &Step,
    occupied_orbit: bool,
    track_chi: bool,
    scratch: &mut Scratch,
) -> Option<Transition> {
    let n = step.old_orbits.len() + 1;
    debug_assert_eq!(topology.labels.len() + 1, n);
    scratch.occupied.clear();
    scratch
        .occupied
        .extend(topology.labels.iter().map(|&v| v != 0));
    scratch.occupied.push(occupied_orbit);
    scratch.dsu.reset(n);
    scratch.first_label.clear();
    scratch.first_label.resize(n, usize::MAX);
    for (j, &label) in topology.labels.iter().enumerate() {
        if label == 0 {
            continue;
        }
        let first = scratch.first_label[label];
        if first == usize::MAX {
            scratch.first_label[label] = j;
        } else if topology.witness {
            scratch.dsu.union_plain(first, j);
        } else {
            let odd = scratch
                .dsu
                .union(first, j, topology.parity[first] ^ topology.parity[j]);
            debug_assert!(!odd);
        }
    }
    let mut delta_chi = 0;
    if occupied_orbit && track_chi {
        scratch.occupied_cell.clear();
        scratch
            .occupied_cell
            .extend(step.old_cell_positions.iter().map(|&j| scratch.occupied[j]));
        scratch
            .occupied_cell
            .resize(step.old_live_cells.len() + step.orbit.len(), false);
        for within in 0..step.orbit.len() {
            let shared_edges = step.cell_neighbors[within]
                .iter()
                .filter(|&&j| scratch.occupied_cell[j])
                .count();
            let new_vertices = step.vertex_neighbors[within]
                .iter()
                .filter(|neighbors| neighbors.iter().all(|&j| !scratch.occupied_cell[j]))
                .count();
            delta_chi += 1 - (4 - shared_edges) as i128 + new_vertices as i128;
            scratch.occupied_cell[step.old_live_cells.len() + within] = true;
        }
    }
    let mut witness = topology.witness;
    for &(a, b, voltage) in &step.edges {
        if scratch.occupied[a] && scratch.occupied[b] {
            if witness {
                scratch.dsu.union_plain(a, b);
            } else if scratch.dsu.union(a, b, voltage) {
                witness = true;
            }
        }
    }
    scratch.root_label.clear();
    scratch.root_label.resize(n, 0);
    scratch.root_parity.clear();
    scratch.root_parity.resize(n, 0);
    let mut labels = Vec::with_capacity(step.next_orbits.len());
    let mut parity = Vec::with_capacity(step.next_orbits.len());
    let mut live_roots = 0;
    for &j in &step.next_positions {
        if !scratch.occupied[j] {
            labels.push(0);
            parity.push(0);
            continue;
        }
        let (root, potential) = scratch.dsu.find(j);
        if scratch.root_label[root] == 0 {
            live_roots += 1;
            scratch.root_label[root] = live_roots;
            scratch.root_parity[root] = potential;
        }
        labels.push(scratch.root_label[root]);
        parity.push(if witness {
            0
        } else {
            potential ^ scratch.root_parity[root]
        });
    }
    scratch.root_seen.clear();
    scratch.root_seen.resize(n, false);
    let mut all_roots = 0;
    let mut closed = false;
    for j in 0..n {
        if scratch.occupied[j] {
            let root = scratch.dsu.find(j).0;
            if !scratch.root_seen[root] {
                scratch.root_seen[root] = true;
                all_roots += 1;
                closed |= scratch.root_label[root] == 0;
            }
        }
    }
    if closed && (all_roots != 1 || live_roots != 0 || !witness) {
        return None;
    }
    Some(Transition {
        topology: Topology {
            labels,
            parity,
            witness,
        },
        delta_chi,
        closed,
    })
}

fn accumulate_closed<W: Ways>(
    result: &mut (W, W),
    chi: i128,
    track_chi: bool,
    ways: &W,
) -> io::Result<()> {
    if !track_chi || chi <= 0 {
        result.1.add_checked(ways)?;
    } else if chi == 1 {
        result.0.add_checked(ways)?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "connected final state has Euler characteristic greater than one",
        ));
    }
    Ok(())
}

fn run_branch<K: TopologyKey, W: Ways>(
    geometry: &Geometry,
    branch: &Branch,
    profile: bool,
) -> io::Result<(BigUint, BigUint)> {
    let branch_start = profile.then(Instant::now);
    let step_count = geometry.steps.len();
    // 后缀只计该分支允许选择的轨道；强制空的中央轨道不能补包围盒。
    let mut suffix_bounds = vec![0u8; step_count + 1];
    let mut suffix_required = vec![false; step_count + 1];
    for i in (0..step_count).rev() {
        suffix_bounds[i] = suffix_bounds[i + 1]
            | if branch.demands[i] != Some(false) {
                geometry.steps[i].boundary_bits
            } else {
                0
            };
        suffix_required[i] = suffix_required[i + 1] || branch.demands[i] == Some(true);
    }
    let mut states = FxHashMap::default();
    let mut initial_values = FxHashMap::default();
    initial_values.insert(
        Values {
            chi: 0,
            bounds: 0,
            center_seen: false,
        },
        W::one(),
    );
    states.insert(
        K::encode(
            Topology {
                labels: Vec::new(),
                parity: Vec::new(),
                witness: branch.assumed_witness,
            },
            entry_bits(0),
        ),
        initial_values,
    );
    let mut result = (W::default(), W::default());
    let mut scratch = Scratch::default();
    let mut topology = Topology::default();
    let mut total_states = 0usize;
    let mut total_topologies = 0usize;
    let mut total_transitions = 0usize;
    for (i, step) in geometry.steps.iter().enumerate() {
        let step_start = profile.then(Instant::now);
        let states_in: usize = states.values().map(FxHashMap::len).sum();
        let unique_topologies = states.len();
        let mut computed_transitions = 0usize;
        let mut bbox_pruned = 0usize;
        let mut closed_accepted = 0usize;
        let mut closed_required_rejected = 0usize;
        let mut next: FxHashMap<K, FxHashMap<Values, W>> = FxHashMap::default();
        for (key, values) in states {
            key.decode(
                step.old_orbits.len(),
                entry_bits(step.old_orbits.len()),
                &mut topology,
            );
            for occupied in [false, true] {
                if branch.demands[i].is_some_and(|required| required != occupied) {
                    continue;
                }
                computed_transitions += 1;
                // 非自由固定点被占用时，其提升已是一个原格子，而非多个 sheet。
                // 此事件不能仅依靠自由轨道边的奇电压闭路检测。
                let old_witness = topology.witness;
                topology.witness |= occupied && step.orbit.len() == 1;
                let Some(after) =
                    advance_topology(&topology, step, occupied, branch.track_chi, &mut scratch)
                else {
                    topology.witness = old_witness;
                    continue;
                };
                topology.witness = old_witness;
                if after.closed && suffix_required[i + 1] {
                    closed_required_rejected += values.len();
                    continue;
                }
                let extra_bounds = if occupied { step.boundary_bits } else { 0 };
                let selected_center =
                    occupied != branch.center_empty && branch.any_centers.contains(&i);
                let last_center = branch.any_centers.last().copied();
                if after.closed {
                    // 唯一可能的后缀是全空；未来强制 occupied 已在上面排除。
                    for (value, ways) in &values {
                        // 闭合后所有未来轨道必为空；这可满足“至少一个中心空”，
                        // 却不能满足“至少一个中心 occupied”。
                        let future_empty_center =
                            branch.center_empty && last_center.is_some_and(|last| i < last);
                        if last_center.is_some_and(|last| i <= last)
                            && !(value.center_seen || selected_center || future_empty_center)
                        {
                            closed_required_rejected += 1;
                            continue;
                        }
                        if value.bounds | extra_bounds == 3 {
                            accumulate_closed(
                                &mut result,
                                value.chi + after.delta_chi,
                                branch.track_chi,
                                ways,
                            )?;
                            closed_accepted += 1;
                        } else {
                            bbox_pruned += 1;
                        }
                    }
                } else {
                    let destination = next
                        .entry(K::encode(
                            after.topology,
                            entry_bits(step.next_orbits.len()),
                        ))
                        .or_default();
                    for (value, ways) in &values {
                        let center_seen = value.center_seen || selected_center;
                        if last_center == Some(i) && !center_seen {
                            closed_required_rejected += 1;
                            continue;
                        }
                        let bounds = value.bounds | extra_bounds;
                        if bounds | suffix_bounds[i + 1] != 3 {
                            bbox_pruned += 1;
                            continue;
                        }
                        destination
                            .entry(Values {
                                chi: value.chi + after.delta_chi,
                                bounds,
                                center_seen: last_center.is_some_and(|last| i < last)
                                    && center_seen,
                            })
                            .or_default()
                            .add_checked(ways)?;
                    }
                }
            }
        }
        next.retain(|_, values| !values.is_empty());
        states = next;
        total_states += states_in;
        total_topologies += unique_topologies;
        total_transitions += computed_transitions;
        if let Some(start) = step_start {
            let states_out: usize = states.values().map(FxHashMap::len).sum();
            eprintln!(
                "quotient_step pattern={} track_chi={} anchor={} step={} states_in={} states_out={} unique_topologies={} computed_transitions={} bbox_pruned={} closed_accepted={} closed_required_rejected={} frontier={} elapsed_seconds={:.9}",
                branch.pattern, branch.track_chi, branch.assumed_witness, i + 1,
                states_in, states_out, unique_topologies, computed_transitions,
                bbox_pruned, closed_accepted, closed_required_rejected,
                step.next_orbits.len(), start.elapsed().as_secs_f64(),
            );
        }
    }
    if let Some(start) = branch_start {
        eprintln!(
            "quotient_branch pattern={} track_chi={} anchor={} states_in={} unique_topologies={} computed_transitions={} elapsed_seconds={:.9}",
            branch.pattern, branch.track_chi, branch.assumed_witness, total_states,
            total_topologies, total_transitions, start.elapsed().as_secs_f64(),
        );
    }
    Ok((result.0.into_biguint(), result.1.into_biguint()))
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Split,
    Single,
    Two,
    Center,
}

fn mode_branches(
    width: usize,
    height: usize,
    quarter: bool,
    geometry: &Geometry,
    mode: Mode,
) -> Vec<Branch> {
    let count = geometry.steps.len();
    if matches!(mode, Mode::Single) {
        return vec![Branch {
            pattern: usize::MAX,
            demands: vec![None; count],
            track_chi: true,
            assumed_witness: false,
            any_centers: Vec::new(),
            center_empty: false,
        }];
    }
    let mut selected = branches(width, height, quarter, &geometry.owner, count);
    if matches!(mode, Mode::Two) && selected.len() == 4 {
        let centers = selected[0]
            .demands
            .iter()
            .enumerate()
            .filter_map(|(i, demand)| demand.map(|_| i))
            .collect();
        let empty = selected.remove(0);
        return vec![
            empty,
            Branch {
                pattern: usize::MAX - 1,
                demands: vec![None; count],
                track_chi: true,
                assumed_witness: false,
                any_centers: centers,
                center_empty: false,
            },
        ];
    }
    if matches!(mode, Mode::Center) && selected.len() == 4 {
        let centers = selected[0]
            .demands
            .iter()
            .enumerate()
            .filter_map(|(i, demand)| demand.map(|_| i))
            .collect();
        let full = selected
            .pop()
            .expect("four center branches include full center");
        // 中央 00 必有洞；01/10 的两个永久空格由中央 checkerboard 与
        // 最终 fg4 连通形成的 Jordan 曲线分隔，也必有 bg4 洞。
        // 因此“非全满中心”可统一不跟踪 χ；对角已选格并不是连通锚点，
        // 仍须电压见证与最终 fg4 连通。
        return vec![
            full,
            Branch {
                pattern: usize::MAX - 2,
                demands: vec![None; count],
                track_chi: false,
                assumed_witness: false,
                any_centers: centers,
                center_empty: true,
            },
        ];
    }
    selected
}

fn run_selected<K: TopologyKey, W: Ways>(
    geometry: &Geometry,
    selected: &[Branch],
    profile: bool,
) -> io::Result<(BigUint, BigUint)> {
    if profile {
        eprintln!(
            "quotient_encoding key={} ways={}",
            std::any::type_name::<K>(),
            std::any::type_name::<W>()
        );
    }
    let mut result = (BigUint::default(), BigUint::default());
    for branch in selected {
        let counts = run_branch::<K, W>(geometry, branch, profile)?;
        result.0 += counts.0;
        result.1 += counts.1;
    }
    Ok(result)
}

fn dispatch_keys<W: Ways>(
    geometry: &Geometry,
    selected: &[Branch],
    profile: bool,
) -> io::Result<(BigUint, BigUint)> {
    let packed_bits = geometry
        .steps
        .iter()
        .map(|step| {
            let n = step.next_orbits.len();
            n.saturating_mul(entry_bits(n)).saturating_add(1)
        })
        .max()
        .unwrap_or(1);
    if packed_bits <= 64 {
        run_selected::<u64, W>(geometry, selected, profile)
    } else if packed_bits <= 128 {
        run_selected::<u128, W>(geometry, selected, profile)
    } else {
        run_selected::<Topology, W>(geometry, selected, profile)
    }
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
    let mode = match std::env::var("ROOM_COUNT_QUOTIENT_MODE").as_deref() {
        Err(_) | Ok("split") => Mode::Split,
        Ok("single") => Mode::Single,
        Ok("two") => Mode::Two,
        Ok("center") => Mode::Center,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ROOM_COUNT_QUOTIENT_MODE must be split, single, two, or center",
            ))
        }
    };
    let selected = mode_branches(width, height, quarter, &geometry, mode);
    if profile {
        eprintln!(
            "quotient_config mode={mode:?} branches={} orbits={}",
            selected.len(),
            geometry.steps.len()
        );
    }
    // 任意前缀权重与最终结果都不超过 2^m；边界严格，避免 2^64/2^128 溢出。
    if geometry.steps.len() < 64 {
        dispatch_keys::<u64>(&geometry, &selected, profile)
    } else if geometry.steps.len() < 128 {
        dispatch_keys::<u128>(&geometry, &selected, profile)
    } else {
        dispatch_keys::<BigUint>(&geometry, &selected, profile)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_topology_transitions_match_reference_at_every_layer() {
        for (width, height, quarter) in [(4, 4, false), (3, 5, false), (5, 5, true)] {
            let geometry = make_geometry(width, height, quarter);
            for track_chi in [false, true] {
                for witness in [false, true] {
                    let mut states = std::collections::HashSet::new();
                    states.insert(State {
                        labels: Vec::new(),
                        parity: Vec::new(),
                        witness,
                        chi: 0,
                        bounds: 0,
                        sealed: false,
                    });
                    let mut scratch = Scratch::default();
                    for step in &geometry.steps {
                        let mut next = std::collections::HashSet::new();
                        for state in states {
                            let topology = Topology {
                                labels: state.labels.clone(),
                                parity: state.parity.clone(),
                                witness: state.witness,
                            };
                            for occupied in [false, true] {
                                let expected = advance(&state, step, occupied, track_chi);
                                let actual = advance_topology(
                                    &topology,
                                    step,
                                    occupied,
                                    track_chi,
                                    &mut scratch,
                                )
                                .map(|after| State {
                                    labels: after.topology.labels,
                                    parity: after.topology.parity,
                                    witness: after.topology.witness,
                                    chi: state.chi + after.delta_chi,
                                    bounds: state.bounds
                                        | if occupied { step.boundary_bits } else { 0 },
                                    sealed: after.closed,
                                });
                                assert_eq!(actual, expected);
                                if let Some(after) = expected {
                                    // 生产路径当步归类闭合态；这里只有活跃态继续比较。
                                    if !after.sealed {
                                        next.insert(after);
                                    }
                                }
                            }
                        }
                        states = next;
                    }
                }
            }
        }
    }

    #[test]
    fn branch_counts_match_independent_flood_oracle() {
        for (width, height, quarter, expected) in [
            (4, 4, false, vec![(0u64, 1u64), (0, 2), (0, 2), (42, 0)]),
            (4, 6, false, vec![(0, 20), (0, 33), (0, 33), (348, 65)]),
            (6, 4, false, vec![(0, 20), (0, 33), (0, 33), (348, 65)]),
            (7, 7, true, vec![(0, 749), (412, 288)]),
            // 端点分量可能在强制中心之前关闭，不可预支锚点接受它们。
            (1, 9, false, vec![(0, 0), (1, 0)]),
        ] {
            let geometry = make_geometry(width, height, quarter);
            for (branch, counts) in branches(
                width,
                height,
                quarter,
                &geometry.owner,
                geometry.steps.len(),
            )
            .iter()
            .zip(expected)
            {
                assert_eq!(
                    run_branch::<Topology, BigUint>(&geometry, branch, false).unwrap(),
                    (BigUint::from(counts.0), BigUint::from(counts.1)),
                    "{width}x{height} quarter={quarter} pattern={}",
                    branch.pattern
                );
            }
        }
    }

    #[test]
    fn modes_and_forced_encodings_match_independent_flood() {
        for (width, height, quarter) in [
            (1, 1, false),
            (1, 9, false),
            (3, 3, false),
            (4, 4, false),
            (4, 6, false),
            (5, 5, false),
            (5, 5, true),
            (7, 7, true),
        ] {
            let geometry = make_geometry(width, height, quarter);
            let expected = brute(width, height, quarter);
            for mode in [Mode::Split, Mode::Single, Mode::Two, Mode::Center] {
                let selected = mode_branches(width, height, quarter, &geometry, mode);
                assert_eq!(
                    run_selected::<u64, u64>(&geometry, &selected, false).unwrap(),
                    expected,
                    "{width}x{height} quarter={quarter} {mode:?} u64"
                );
                assert_eq!(
                    run_selected::<u128, u128>(&geometry, &selected, false).unwrap(),
                    expected,
                    "{width}x{height} quarter={quarter} {mode:?} u128"
                );
                assert_eq!(
                    run_selected::<Topology, BigUint>(&geometry, &selected, false).unwrap(),
                    expected,
                    "{width}x{height} quarter={quarter} {mode:?} dynamic/biguint"
                );
            }
        }
    }

    #[test]
    fn packed_phase_roundtrips_and_integer_count_bounds() {
        for (len, witness) in [(0usize, false), (12, false), (19, true)] {
            let topology = Topology {
                labels: (0..len).collect(),
                parity: (0..len)
                    .map(|i| if witness { 0 } else { (i & 1) as u8 })
                    .collect(),
                witness,
            };
            let bits = entry_bits(len);
            let packed = <u128 as TopologyKey>::encode(topology.clone(), bits);
            let mut decoded = Topology::default();
            packed.decode(len, bits, &mut decoded);
            assert_eq!(decoded, topology);
            if len * bits + 1 <= 64 {
                let packed = <u64 as TopologyKey>::encode(topology.clone(), bits);
                packed.decode(len, bits, &mut decoded);
                assert_eq!(decoded, topology);
            }
        }
        let topology = Topology {
            labels: (0..40).collect(),
            parity: vec![0; 40],
            witness: true,
        };
        let encoded = <Topology as TopologyKey>::encode(topology.clone(), entry_bits(40));
        let mut decoded = Topology::default();
        encoded.decode(40, entry_bits(40), &mut decoded);
        assert_eq!(decoded, topology);
        let mut narrow = u64::MAX;
        assert!(narrow.add_checked(&1).is_err());
        let mut wide = u128::MAX;
        assert!(wide.add_checked(&1).is_err());
        let mut unbounded = BigUint::from(u128::MAX);
        unbounded.add_checked(&BigUint::from(1u8)).unwrap();
        assert_eq!(unbounded, BigUint::from(1u8) << 128usize);
    }

    #[test]
    fn not_full_center_branch_matches_flood_oracle() {
        for (width, height, full, not_full) in [
            (4, 4, (42u64, 0u64), (0u64, 5u64)),
            (4, 6, (348, 65), (0, 86)),
            (6, 4, (348, 65), (0, 86)),
        ] {
            let geometry = make_geometry(width, height, false);
            let selected = mode_branches(width, height, false, &geometry, Mode::Center);
            assert_eq!(selected.len(), 2);
            for (branch, expected) in selected.iter().zip([full, not_full]) {
                assert_eq!(
                    run_branch::<u64, u64>(&geometry, branch, false).unwrap(),
                    (BigUint::from(expected.0), BigUint::from(expected.1))
                );
            }
        }
    }

    #[test]
    fn production_count_dispatch_handles_64_and_128_orbits() {
        for height in [127usize, 255] {
            assert_eq!(
                count_fixed(1, height, false).unwrap(),
                (BigUint::from(1u8), BigUint::default())
            );
        }
    }

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
