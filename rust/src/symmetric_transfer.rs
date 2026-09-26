//! 旋转固定点的轨道前沿转移。
//!
//! 每次同时决定一个旋转轨道中的全部原始格子，前沿仍保存原始格子的
//! 四邻连通分量。因此商图中连通但原图中断开的形状不会被误计。

use num_bigint::BigUint;
use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::io;
use std::rc::Rc;
use std::time::Instant;

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

/// 只在单次 count_fixed 调用中复用，不让可变缓冲进入持久状态键。
#[derive(Default)]
struct Scratch {
    occupied: Vec<bool>,
    parent: Vec<usize>,
    canonical: Vec<usize>,
    next_labels: Vec<usize>,
}

impl Scratch {
    fn find(&mut self, node: usize) -> usize {
        if self.parent[node] != node {
            self.parent[node] = self.find(self.parent[node]);
        }
        self.parent[node]
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left != right {
            self.parent[right] = left;
        }
    }
}

fn advance_fast(
    state: &State,
    step: &Step,
    occupied_orbit: bool,
    scratch: &mut Scratch,
) -> Option<State> {
    advance_core(
        &state.labels,
        state.chi,
        state.bounds,
        state.sealed,
        step,
        occupied_orbit,
        scratch,
    )
}

fn advance_core(
    labels: &[usize],
    chi_before: i128,
    bounds_before: u8,
    sealed_before: bool,
    step: &Step,
    occupied_orbit: bool,
    scratch: &mut Scratch,
) -> Option<State> {
    if sealed_before && occupied_orbit {
        return None;
    }
    debug_assert_eq!(labels.len(), step.old_live.len());
    let old_len = labels.len();
    let components = labels.iter().copied().max().unwrap_or(0);
    scratch.next_labels.clear();

    if !occupied_orbit {
        // 没有新边，旧规范分量只需按下一前沿首次出现的顺序重编号。
        scratch.canonical.resize(components + 1, 0);
        scratch.canonical.fill(0);
        let mut surviving = 0;
        for &position in &step.next_positions {
            let old_label = if position < old_len {
                labels[position]
            } else {
                0
            };
            if old_label == 0 {
                scratch.next_labels.push(0);
            } else {
                if scratch.canonical[old_label] == 0 {
                    surviving += 1;
                    scratch.canonical[old_label] = surviving;
                }
                scratch.next_labels.push(scratch.canonical[old_label]);
            }
        }
        let closed = components != surviving;
        if closed && (components != 1 || surviving != 0) {
            return None;
        }
        return Some(State {
            labels: scratch.next_labels.clone(),
            chi: chi_before,
            bounds: bounds_before,
            sealed: sealed_before || closed,
        });
    }

    scratch.occupied.clear();
    scratch
        .occupied
        .extend(labels.iter().map(|&label| label != 0));
    scratch.occupied.resize(old_len + step.orbit.len(), false);
    let nodes = components + step.orbit.len();
    scratch.parent.clear();
    scratch.parent.extend(0..nodes);

    let mut chi = chi_before;
    for within in 0..step.orbit.len() {
        let shared_edges = step.cell_neighbors[within]
            .iter()
            .filter(|&&position| scratch.occupied[position])
            .count();
        let new_vertices = step.vertex_neighbors[within]
            .iter()
            .filter(|neighbors| {
                neighbors
                    .iter()
                    .all(|&position| !scratch.occupied[position])
            })
            .count();
        chi += 1 - (4 - shared_edges) as i128 + new_vertices as i128;
        scratch.occupied[old_len + within] = true;
    }

    for &(left, right) in &step.edges {
        if scratch.occupied[left] && scratch.occupied[right] {
            let left_node = if left < old_len {
                labels[left] - 1
            } else {
                components + left - old_len
            };
            let right_node = if right < old_len {
                labels[right] - 1
            } else {
                components + right - old_len
            };
            scratch.union(left_node, right_node);
        }
    }

    scratch.canonical.resize(nodes, 0);
    scratch.canonical.fill(0);
    let mut surviving = 0;
    for &position in &step.next_positions {
        if !scratch.occupied[position] {
            scratch.next_labels.push(0);
            continue;
        }
        let node = if position < old_len {
            labels[position] - 1
        } else {
            components + position - old_len
        };
        let root = scratch.find(node);
        if scratch.canonical[root] == 0 {
            surviving += 1;
            scratch.canonical[root] = surviving;
        }
        scratch.next_labels.push(scratch.canonical[root]);
    }
    let all_components = (0..nodes)
        .filter(|&node| scratch.parent[node] == node)
        .count();
    let closed = all_components != surviving;
    if closed && (all_components != 1 || surviving != 0) {
        return None;
    }
    Some(State {
        labels: scratch.next_labels.clone(),
        chi,
        bounds: bounds_before | step.boundary_bits,
        sealed: sealed_before || closed,
    })
}

trait LabelKey: Clone + Eq + Hash {
    fn encode(labels: Vec<usize>, bits: usize) -> Self;
    fn decode(&self, len: usize, bits: usize, output: &mut Vec<usize>);
}

impl LabelKey for u64 {
    fn encode(labels: Vec<usize>, bits: usize) -> Self {
        let mut packed = 0u64;
        for (position, label) in labels.into_iter().enumerate() {
            packed |= (label as u64) << (position * bits);
        }
        packed
    }

    fn decode(&self, len: usize, bits: usize, output: &mut Vec<usize>) {
        output.clear();
        let mask = (1u64 << bits) - 1;
        for position in 0..len {
            output.push(((self >> (position * bits)) & mask) as usize);
        }
    }
}

impl LabelKey for u128 {
    fn encode(labels: Vec<usize>, bits: usize) -> Self {
        let mut packed = 0u128;
        for (position, label) in labels.into_iter().enumerate() {
            packed |= (label as u128) << (position * bits);
        }
        packed
    }

    fn decode(&self, len: usize, bits: usize, output: &mut Vec<usize>) {
        output.clear();
        let mask = (1u128 << bits) - 1;
        for position in 0..len {
            output.push(((self >> (position * bits)) & mask) as usize);
        }
    }
}

impl LabelKey for Rc<[usize]> {
    fn encode(labels: Vec<usize>, _bits: usize) -> Self {
        Rc::from(labels)
    }

    fn decode(&self, _len: usize, _bits: usize, output: &mut Vec<usize>) {
        output.clear();
        output.extend(self.iter().copied());
    }
}

fn label_bits(frontier_len: usize) -> usize {
    (usize::BITS - frontier_len.leading_zeros()) as usize
}

struct TopologyPool<K: LabelKey> {
    index: FxHashMap<K, usize>,
    labels: Vec<K>,
    frontier_len: usize,
    bits: usize,
}

impl<K: LabelKey> TopologyPool<K> {
    fn new(frontier_len: usize) -> Self {
        Self {
            index: FxHashMap::default(),
            labels: Vec::new(),
            frontier_len,
            bits: label_bits(frontier_len),
        }
    }

    fn intern(&mut self, labels: Vec<usize>) -> usize {
        debug_assert_eq!(labels.len(), self.frontier_len);
        let key = K::encode(labels, self.bits);
        if let Some(&id) = self.index.get(&key) {
            return id;
        }
        let id = self.labels.len();
        self.index.insert(key.clone(), id);
        self.labels.push(key);
        id
    }

    fn decode(&self, id: usize, output: &mut Vec<usize>) {
        self.labels[id].decode(self.frontier_len, self.bits, output);
    }
}

#[derive(Clone, Copy)]
enum OrbitEdge {
    Invalid,
    Next { target: usize, delta_chi: i8 },
    Closed { delta_chi: i8 },
}

trait Ways: Clone + Default {
    fn one() -> Self;
    fn add_checked(&mut self, other: &Self) -> io::Result<()>;
    fn into_biguint(self) -> BigUint;
}

impl Ways for u64 {
    fn one() -> Self {
        1
    }

    fn add_checked(&mut self, other: &Self) -> io::Result<()> {
        *self = self.checked_add(*other).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "u64 fixed-set count overflow")
        })?;
        Ok(())
    }

    fn into_biguint(self) -> BigUint {
        BigUint::from(self)
    }
}

impl Ways for u128 {
    fn one() -> Self {
        1
    }

    fn add_checked(&mut self, other: &Self) -> io::Result<()> {
        *self = self.checked_add(*other).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "u128 fixed-set count overflow")
        })?;
        Ok(())
    }

    fn into_biguint(self) -> BigUint {
        BigUint::from(self)
    }
}

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

fn classify_closed<W: Ways>(
    chi: i128,
    bounds: u8,
    ways: &W,
    no_hole: &mut W,
    has_hole: &mut W,
) -> io::Result<()> {
    if bounds != 3 {
        return Ok(());
    }
    if chi == 1 {
        no_hole.add_checked(ways)?;
    } else if chi <= 0 {
        has_hole.add_checked(ways)?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "connected final state has Euler characteristic greater than one",
        ));
    }
    Ok(())
}

fn run_count<K: LabelKey, W: Ways>(
    width: usize,
    height: usize,
    quarter: bool,
    steps: &[Step],
    profile: bool,
) -> io::Result<(BigUint, BigUint)> {
    let mut suffix_bounds = vec![0u8; steps.len() + 1];
    for index in (0..steps.len()).rev() {
        suffix_bounds[index] = suffix_bounds[index + 1] | steps[index].boundary_bits;
    }
    let mut topologies = TopologyPool::<K>::new(0);
    let empty = topologies.intern(Vec::new());
    let mut weights: FxHashMap<(usize, i128, u8), W> = FxHashMap::default();
    weights.insert((empty, 0i128, 0u8), W::one());
    let mut scratch = Scratch::default();
    let mut decoded = Vec::new();
    let mut no_hole = W::default();
    let mut has_hole = W::default();
    for (step_index, step) in steps.iter().enumerate() {
        let states_in = weights.len();
        let capacity_in = weights.capacity();
        let topologies_in = topologies.labels.len();
        let step_start = Instant::now();
        let mut valid = 0usize;
        let mut invalid = 0usize;
        let mut closed = 0usize;
        let mut pruned = 0usize;
        let mut next_topologies = TopologyPool::<K>::new(step.new_live.len());
        let mut next_weights: FxHashMap<(usize, i128, u8), W> = FxHashMap::default();
        let mut edges = vec![[None; 2]; topologies.labels.len()];
        for ((topology, chi, bounds), ways) in &weights {
            if *bounds | suffix_bounds[step_index] != 3 {
                if profile {
                    pruned += 2;
                }
                continue;
            }
            for occupied in [false, true] {
                let bounds_after = *bounds | if occupied { step.boundary_bits } else { 0 };
                if bounds_after | suffix_bounds[step_index + 1] != 3 {
                    if profile {
                        pruned += 1;
                    }
                    continue;
                }
                let branch = usize::from(occupied);
                let edge = if let Some(edge) = edges[*topology][branch] {
                    edge
                } else {
                    topologies.decode(*topology, &mut decoded);
                    let outcome = advance_core(&decoded, 0, 0, false, step, occupied, &mut scratch);
                    let edge = match outcome {
                        None => OrbitEdge::Invalid,
                        Some(after) if after.sealed => OrbitEdge::Closed {
                            delta_chi: i8::try_from(after.chi).expect("orbit Euler delta fits i8"),
                        },
                        Some(after) => OrbitEdge::Next {
                            target: next_topologies.intern(after.labels),
                            delta_chi: i8::try_from(after.chi).expect("orbit Euler delta fits i8"),
                        },
                    };
                    edges[*topology][branch] = Some(edge);
                    edge
                };
                match edge {
                    OrbitEdge::Invalid => {
                        if profile {
                            invalid += 1;
                        }
                    }
                    OrbitEdge::Closed { delta_chi } => {
                        if profile {
                            closed += 1;
                            valid += 1;
                        }
                        classify_closed(
                            *chi + i128::from(delta_chi),
                            bounds_after,
                            ways,
                            &mut no_hole,
                            &mut has_hole,
                        )?;
                    }
                    OrbitEdge::Next { target, delta_chi } => {
                        if profile {
                            valid += 1;
                        }
                        next_weights
                            .entry((target, *chi + i128::from(delta_chi), bounds_after))
                            .or_default()
                            .add_checked(ways)?;
                    }
                }
            }
        }
        if profile {
            eprintln!(
                "symmetric_profile width={width} height={height} quarter={quarter} step={} orbit={} frontier_in={} frontier_out={} states_in={} topologies_in={} capacity_in={} states_out={} topologies_out={} capacity_out={} valid={} invalid={} closed={} pruned={} elapsed_ms={:.3}",
                step_index + 1,
                step.orbit.len(),
                step.old_live.len(),
                step.new_live.len(),
                states_in,
                topologies_in,
                capacity_in,
                next_weights.len(),
                next_topologies.labels.len(),
                next_weights.capacity(),
                valid,
                invalid,
                closed,
                pruned,
                step_start.elapsed().as_secs_f64() * 1_000.0
            );
        }
        topologies = next_topologies;
        weights = next_weights;
    }
    Ok((no_hole.into_biguint(), has_hole.into_biguint()))
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
    let profile = std::env::var_os("ROOM_COUNT_PROFILE").is_some();
    let build_start = Instant::now();
    let steps = make_steps(width, height, quarter);
    let packed_bits = steps
        .iter()
        .flat_map(|step| [step.old_live.len(), step.new_live.len()])
        .map(|len| len.checked_mul(label_bits(len)).unwrap_or(usize::MAX))
        .max()
        .unwrap_or(0);
    let key_kind = if packed_bits <= 64 {
        "u64"
    } else if packed_bits <= 128 {
        "u128"
    } else {
        "dynamic"
    };
    let ways_kind = if steps.len() <= 63 {
        "u64"
    } else if steps.len() <= 127 {
        "u128"
    } else {
        "biguint"
    };
    if profile {
        let key_bytes = if packed_bits <= 64 {
            std::mem::size_of::<u64>()
        } else if packed_bits <= 128 {
            std::mem::size_of::<u128>()
        } else {
            std::mem::size_of::<Rc<[usize]>>()
        };
        let ways_bytes = if steps.len() <= 63 {
            std::mem::size_of::<u64>()
        } else if steps.len() <= 127 {
            std::mem::size_of::<u128>()
        } else {
            std::mem::size_of::<BigUint>()
        };
        eprintln!(
            "symmetric_profile width={width} height={height} quarter={quarter} phase=build steps={} packed_bits={} key={} key_bytes={} ways={} ways_bytes={} edge_cache_entry_bytes={} elapsed_ms={:.3}",
            steps.len(),
            packed_bits,
            key_kind,
            key_bytes,
            ways_kind,
            ways_bytes,
            std::mem::size_of::<[Option<OrbitEdge>; 2]>(),
            build_start.elapsed().as_secs_f64() * 1_000.0
        );
    }
    if steps.len() <= 63 {
        if packed_bits <= 64 {
            run_count::<u64, u64>(width, height, quarter, &steps, profile)
        } else if packed_bits <= 128 {
            run_count::<u128, u64>(width, height, quarter, &steps, profile)
        } else {
            run_count::<Rc<[usize]>, u64>(width, height, quarter, &steps, profile)
        }
    } else if steps.len() <= 127 {
        if packed_bits <= 64 {
            run_count::<u64, u128>(width, height, quarter, &steps, profile)
        } else if packed_bits <= 128 {
            run_count::<u128, u128>(width, height, quarter, &steps, profile)
        } else {
            run_count::<Rc<[usize]>, u128>(width, height, quarter, &steps, profile)
        }
    } else if packed_bits <= 64 {
        run_count::<u64, BigUint>(width, height, quarter, &steps, profile)
    } else if packed_bits <= 128 {
        run_count::<u128, BigUint>(width, height, quarter, &steps, profile)
    } else {
        run_count::<Rc<[usize]>, BigUint>(width, height, quarter, &steps, profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_count(width: usize, height: usize, quarter: bool) -> (BigUint, BigUint) {
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
        for step in make_steps(width, height, quarter) {
            let mut next = FxHashMap::default();
            for (state, ways) in &states {
                for occupied in [false, true] {
                    if let Some(after) = advance(state, &step, occupied) {
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
                    panic!("reference state has χ > 1");
                }
            }
        }
        (no_hole, has_hole)
    }

    #[test]
    fn grouped_topologies_and_early_close_match_reference() {
        for width in 1..=5 {
            for height in 1..=5 {
                assert_eq!(
                    count_fixed(width, height, false).unwrap(),
                    reference_count(width, height, false),
                    "half {width}×{height}"
                );
                if width == height {
                    assert_eq!(
                        count_fixed(width, height, true).unwrap(),
                        reference_count(width, height, true),
                        "quarter {width}×{height}"
                    );
                }
            }
        }
    }

    #[test]
    fn occupied_single_cell_closes_with_euler_and_bounds() {
        let step = make_steps(1, 1, false).pop().unwrap();
        let after = advance_core(&[], 0, 0, false, &step, true, &mut Scratch::default()).unwrap();
        assert_eq!(after.chi, 1);
        assert_eq!(after.bounds, 3);
        assert!(after.sealed);
        assert_eq!(
            count_fixed(1, 1, false).unwrap(),
            (BigUint::from(1u8), BigUint::default())
        );
    }

    #[test]
    fn packed_labels_cross_word_boundary_and_dynamic_fallback() {
        let labels: Vec<usize> = (0..24).map(|position| position % 20).collect();
        let bits = label_bits(labels.len());
        assert!(labels.len() * bits > 64);
        assert!(labels.len() * bits <= 128);
        let packed = <u128 as LabelKey>::encode(labels.clone(), bits);
        let mut decoded = Vec::new();
        packed.decode(labels.len(), bits, &mut decoded);
        assert_eq!(decoded, labels);

        let wide_labels: Vec<usize> = (0..48).map(|position| position % 32).collect();
        assert!(wide_labels.len() * label_bits(wide_labels.len()) > 128);
        let wide = <Rc<[usize]> as LabelKey>::encode(wide_labels.clone(), 0);
        wide.decode(wide_labels.len(), 0, &mut decoded);
        assert_eq!(decoded, wide_labels);
    }

    #[test]
    fn forced_wide_labels_and_biguint_ways_match_small_reference() {
        for (width, height, quarter) in [(1, 1, false), (3, 3, true), (4, 5, false)] {
            let steps = make_steps(width, height, quarter);
            let expected = reference_count(width, height, quarter);
            assert_eq!(
                run_count::<Rc<[usize]>, BigUint>(width, height, quarter, &steps, false).unwrap(),
                expected
            );
            assert_eq!(
                run_count::<u128, u128>(width, height, quarter, &steps, false).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn scratch_and_component_dsu_match_reference_at_every_step() {
        for width in 1..=5 {
            for height in 1..=5 {
                for quarter in [false, true] {
                    if quarter && width != height {
                        continue;
                    }
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
                    let mut scratch = Scratch::default();
                    for step in make_steps(width, height, quarter) {
                        let mut reference = FxHashMap::default();
                        let mut actual = FxHashMap::default();
                        for (state, ways) in &states {
                            for occupied in [false, true] {
                                let expected = advance(state, &step, occupied);
                                let found = advance_fast(state, &step, occupied, &mut scratch);
                                assert_eq!(
                                    found, expected,
                                    "{width}×{height} quarter={quarter} orbit={:?} occupied={occupied}",
                                    step.orbit
                                );
                                if let Some(after) = expected {
                                    *reference.entry(after).or_insert_with(BigUint::default) +=
                                        ways;
                                }
                                if let Some(after) = found {
                                    *actual.entry(after).or_insert_with(BigUint::default) += ways;
                                }
                            }
                        }
                        assert_eq!(actual, reference);
                        states = reference;
                    }
                }
            }
        }
    }

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
