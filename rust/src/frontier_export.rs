//! 在紧包围盒内逐格回溯，按前沿连通状态剪去不可能完成的前缀。
//!
//! 状态图和后缀可行性表只保存有限前沿；不同前缀即使到达同一状态，
//! 仍各自沿标有 0/1 的边展开，因而不会丢失具体位图。

use crate::bit_utils::{poly_has_hole, rotate90};
use crate::export::ExportManager;
use crate::types::{Mask, RoomCount, MAX_N, STRIDE};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::io;
use std::sync::Arc;

const INVALID: u32 = u32::MAX;
const BATCH_SIZE: usize = 8192;
const SPLIT_DEPTH: usize = 8;
type Bins = [[u64; 2]; MAX_N];

#[derive(Clone, Copy, Default)]
struct State {
    // 0 是空格；非零标签表示仍碰到前沿的四连通分量。
    labels: [u8; MAX_N],
    // 0 尚未占格；1 有活跃分量；2 唯一分量已关闭，只能再选空格。
    phase: u8,
}

impl State {
    fn key(self) -> u32 {
        let mut key = (self.phase as u32) << (MAX_N * 3);
        for (col, label) in self.labels.iter().enumerate() {
            key |= (*label as u32) << (col * 3);
        }
        key
    }

    fn accepting(self) -> bool {
        self.phase == 2
            || (self.phase == 1
                && self.labels.iter().any(|&x| x != 0)
                && self.labels.iter().all(|&x| x <= 1))
    }
}

fn canonicalize(labels: &mut [u8; MAX_N], width: usize) {
    let mut rename = [0u8; MAX_N + 1];
    let mut next = 1;
    for label in &mut labels[..width] {
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
    let up = state.labels[col];
    let left = if col == 0 { 0 } else { state.labels[col - 1] };
    if occupied {
        let label = if left != 0 {
            left
        } else if up != 0 {
            up
        } else {
            state.labels[..width].iter().copied().max().unwrap_or(0) + 1
        };
        next.labels[col] = label;
        if left != 0 && up != 0 && left != up {
            for existing in &mut next.labels[..width] {
                if *existing == up {
                    *existing = left;
                }
            }
        }
        next.phase = 1;
    } else {
        next.labels[col] = 0;
        if up != 0 && !next.labels[..width].contains(&up) {
            // 一个分量离开前沿后，再也不能同其它分量接通。
            if next.labels[..width].iter().any(|&x| x != 0) {
                return None;
            }
            next.phase = 2;
        }
    }
    canonicalize(&mut next.labels, width);
    Some(next)
}

struct Node {
    state: State,
    col: usize,
    edge: [u32; 2],
}

fn topology_graph(width: usize) -> Vec<Node> {
    let mut nodes = vec![Node {
        state: State::default(),
        col: 0,
        edge: [INVALID; 2],
    }];
    let mut ids = FxHashMap::default();
    ids.insert(0u32, 0u32);
    let mut cursor = 0;
    while cursor < nodes.len() {
        let state = nodes[cursor].state;
        let col = nodes[cursor].col;
        for choice in 0..2 {
            if let Some(next) = transition(state, width, col, choice == 1) {
                let next_col = (col + 1) % width;
                let key = (next.key() << 3) | next_col as u32;
                let id = *ids.entry(key).or_insert_with(|| {
                    let id = nodes.len() as u32;
                    nodes.push(Node {
                        state: next,
                        col: next_col,
                        edge: [INVALID; 2],
                    });
                    id
                });
                nodes[cursor].edge[choice] = id;
            }
        }
        cursor += 1;
    }
    nodes
}

#[derive(Clone, Copy)]
struct Prefix {
    index: usize,
    node: u32,
    touched: u8,
    mask: Mask,
}

struct BoxSearch<'a> {
    width: usize,
    height: usize,
    nodes: &'a [Node],
    // 0 尚未计算；1 无合法后缀；2 有合法后缀。
    feasible: Vec<u8>,
}

impl<'a> BoxSearch<'a> {
    fn new(width: usize, height: usize, nodes: &'a [Node]) -> io::Result<Self> {
        let len = (width * height + 1)
            .checked_mul(nodes.len())
            .and_then(|x| x.checked_mul(16))
            .ok_or_else(|| io::Error::other("前沿后缀表尺寸溢出"))?;
        let mut search = Self {
            width,
            height,
            nodes,
            feasible: vec![0; len],
        };
        search.can_finish(0, 0, 0);
        Ok(search)
    }

    fn slot(&self, index: usize, node: u32, touched: u8) -> usize {
        (index * self.nodes.len() + node as usize) * 16 + touched as usize
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

    fn can_finish(&mut self, index: usize, node: u32, touched: u8) -> bool {
        let slot = self.slot(index, node, touched);
        if self.feasible[slot] != 0 {
            return self.feasible[slot] == 2;
        }
        let result = if index == self.width * self.height {
            touched == 15 && self.nodes[node as usize].state.accepting()
        } else if self.nodes[node as usize].state.phase == 2 && touched != 15 {
            false
        } else if index >= self.width && touched & 1 == 0 {
            false
        } else {
            debug_assert_eq!(self.nodes[node as usize].col, index % self.width);
            let [empty, full] = self.nodes[node as usize].edge;
            let empty_ok = empty != INVALID && self.can_finish(index + 1, empty, touched);
            let full_ok = full != INVALID
                && self.can_finish(index + 1, full, self.next_touched(index, touched));
            empty_ok || full_ok
        };
        self.feasible[slot] = if result { 2 } else { 1 };
        result
    }

    fn possible(&self, prefix: Prefix) -> bool {
        self.feasible[self.slot(prefix.index, prefix.node, prefix.touched)] == 2
    }

    fn children(&self, prefix: Prefix) -> impl Iterator<Item = Prefix> + '_ {
        let edges = self.nodes[prefix.node as usize].edge;
        edges
            .into_iter()
            .enumerate()
            .filter_map(move |(choice, node)| {
                if node == INVALID {
                    return None;
                }
                let touched = if choice == 1 {
                    self.next_touched(prefix.index, prefix.touched)
                } else {
                    prefix.touched
                };
                let next = Prefix {
                    index: prefix.index + 1,
                    node,
                    touched,
                    mask: prefix.mask
                        | if choice == 1 {
                            1u64 << ((prefix.index / self.width) * STRIDE
                                + prefix.index % self.width)
                        } else {
                            0
                        },
                };
                self.possible(next).then_some(next)
            })
    }

    fn tasks(&self) -> Vec<Prefix> {
        let mut tasks = vec![Prefix {
            index: 0,
            node: 0,
            touched: 0,
            mask: 0,
        }];
        for _ in 0..(self.width * self.height).min(SPLIT_DEPTH) {
            tasks = tasks
                .into_iter()
                .flat_map(|task| self.children(task))
                .collect();
        }
        tasks
    }
}

struct Local {
    counts: [u64; 2],
    batch: Vec<(Mask, usize, bool)>,
}

impl Local {
    fn new(export: bool) -> Self {
        Self {
            counts: [0; 2],
            batch: if export {
                Vec::with_capacity(BATCH_SIZE)
            } else {
                Vec::new()
            },
        }
    }

    fn emit(
        &mut self,
        mask: Mask,
        md: usize,
        hole: bool,
        export: Option<&ExportManager>,
    ) -> io::Result<()> {
        self.counts[usize::from(hole)] += 1;
        if let Some(export) = export {
            self.batch.push((mask, md, hole));
            if self.batch.len() == BATCH_SIZE {
                export.write_batch(&self.batch)?;
                self.batch.clear();
            }
        }
        Ok(())
    }

    fn flush(&mut self, export: Option<&ExportManager>) -> io::Result<()> {
        if let Some(export) = export {
            if !self.batch.is_empty() {
                export.write_batch(&self.batch)?;
                self.batch.clear();
            }
        }
        Ok(())
    }
}

fn canonical_mask(mask: Mask, width: usize, height: usize) -> bool {
    // 高度较小的方向在 stride8 u64 序中必小于其 90° 旋转方向。
    let (quarter, qw, qh) = rotate90(mask, width, height);
    let (half, _, _) = rotate90(quarter, qw, qh);
    if half < mask {
        return false;
    }
    if width != height {
        return true;
    }
    let (three_quarters, _, _) = rotate90(half, width, height);
    quarter >= mask && three_quarters >= mask
}

fn dfs(
    search: &BoxSearch<'_>,
    prefix: Prefix,
    local: &mut Local,
    export: Option<&ExportManager>,
) -> io::Result<()> {
    if prefix.index == search.width * search.height {
        if canonical_mask(prefix.mask, search.width, search.height) {
            let hole = poly_has_hole(prefix.mask, search.width, search.height);
            local.emit(prefix.mask, search.width, hole, export)?;
        }
        return Ok(());
    }
    for child in search.children(prefix) {
        dfs(search, child, local, export)?;
    }
    Ok(())
}

/// 枚举 n≤max_n 的旋转类，返回累计计数；导出使用原有 stride8 u64 格式。
pub fn enumerate_frontier(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
) -> io::Result<Vec<RoomCount>> {
    assert!((1..=MAX_N).contains(&max_n));
    let mut bins: Bins = [[0; 2]; MAX_N];
    for width in 1..=max_n {
        let nodes = topology_graph(width);
        for height in 1..=width {
            let search = BoxSearch::new(width, height, &nodes)?;
            let tasks = search.tasks();
            let counts = tasks
                .par_iter()
                .map(|&task| -> io::Result<[u64; 2]> {
                    let mut local = Local::new(export.is_some());
                    dfs(&search, task, &mut local, export.as_deref())?;
                    local.flush(export.as_deref())?;
                    Ok(local.counts)
                })
                .try_reduce(|| [0, 0], |a, b| Ok([a[0] + b[0], a[1] + b[1]]))?;
            bins[width - 1][0] += counts[0];
            bins[width - 1][1] += counts[1];
            if verbose {
                eprintln!(
                    "  [frontier] bbox={width}x{height} states={} table={} B tasks={} accepted={}",
                    search.nodes.len(),
                    search.feasible.len(),
                    tasks.len(),
                    counts[0] + counts[1]
                );
            }
        }
    }
    if let Some(export) = export {
        export.flush_all()?;
    }
    let mut cumulative = [0u64; 2];
    let mut result = Vec::with_capacity(max_n);
    for (index, exact) in bins.into_iter().take(max_n).enumerate() {
        cumulative[0] += exact[0];
        cumulative[1] += exact[1];
        let count = RoomCount {
            n: index + 1,
            total: cumulative[0] + cumulative[1],
            no_hole: cumulative[0],
            has_hole: cumulative[1],
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
    fn small_cumulative_counts() {
        let counts = enumerate_frontier(4, false, None).unwrap();
        let expected = [(1, 1, 0), (4, 4, 0), (46, 44, 2), (2404, 1899, 505)];
        for (actual, (total, no_hole, has_hole)) in counts.iter().zip(expected) {
            assert_eq!(
                (actual.total, actual.no_hole, actual.has_hole),
                (total, no_hole, has_hole),
                "n={}",
                actual.n
            );
        }
    }

    #[test]
    fn closed_component_cannot_restart_or_lose_another_component() {
        let first = transition(State::default(), 2, 0, true).unwrap();
        let gap = transition(first, 2, 1, false).unwrap();
        let closed = transition(gap, 2, 0, false).unwrap();
        assert_eq!(closed.phase, 2);
        assert!(transition(closed, 2, 1, true).is_none());
        let two = transition(first, 2, 1, true).unwrap();
        let still_live = transition(two, 2, 0, false).unwrap();
        assert_eq!(still_live.phase, 1);
    }

    #[test]
    fn terminal_rejects_two_live_components() {
        let left = transition(State::default(), 3, 0, true).unwrap();
        let gap = transition(left, 3, 1, false).unwrap();
        let separated = transition(gap, 3, 2, true).unwrap();
        assert_eq!(separated.labels[..3], [1, 0, 2]);
        assert!(!separated.accepting());
    }
}
