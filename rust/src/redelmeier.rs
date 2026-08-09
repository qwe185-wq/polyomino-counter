//! Redelmeier (1981) DFS 回溯枚举 — u128 位掩码，半平面约束
//!
//! ## 核心优化
//! 1. **Untried Set**: 每个形状恰好生成一次，无需哈希集
//! 2. **半平面约束**: r≥0 且 (r>0 或 c≥0)，天然消除平移重复
//! 3. **对称性加权**: DFS 中直接按 1/4, 1/2, 1 权重累加
//! 4. **u128 位掩码**: STRIDE=16 offset=8，支持 n≤6
//! 5. **O(n) 内存**: 仅当前搜索栈 + untried 集合

use crate::bit_utils::*;
use crate::symmetric::{has_symmetry_180, has_symmetry_90};
use crate::types::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// DFS 内部位掩码编码 (独立于 bit_utils 的 STRIDE=8)
// STRIDE=16 offset=8: 支持 c ∈ [-8, 7], r ∈ [0, 15]
const DFS_STRIDE: usize = 16;
const DFS_OFFSET: i32 = 8;
const MAX_UNTRIED: usize = 200;

type DfsMask = u128;

#[inline]
fn dfs_cell_bit(r: i32, c: i32) -> usize {
    ((r) as usize) * DFS_STRIDE + ((c + DFS_OFFSET) as usize)
}

#[inline]
fn dfs_bit_rc(bit: u8) -> (i32, i32) {
    let b = bit as usize;
    ((b / DFS_STRIDE) as i32, (b % DFS_STRIDE) as i32 - DFS_OFFSET)
}

static DR: [i32; 4] = [0, 1, 0, -1];
static DC: [i32; 4] = [1, 0, -1, 0];

struct RState {
    occ: DfsMask,
    untried_mask: DfsMask,
    untried: [u8; MAX_UNTRIED],
    untried_len: u8,
    count: u8,
    max_r: i32, max_c: i32, min_c: i32,
    total_4x: [u64; MAX_N + 1],
    hole_4x: [u64; MAX_N + 1],
    shapes_visited: u64,
    max_n: usize,
    max_cells: usize,
}

impl RState {
    fn new(max_n: usize) -> Self {
        let start = dfs_cell_bit(0, 0);
        Self {
            occ: 1u128 << start,
            untried_mask: 0,
            untried: [0u8; MAX_UNTRIED],
            untried_len: 0, count: 1,
            max_r: 0, max_c: 0, min_c: 0,
            total_4x: [0u64; MAX_N + 1],
            hole_4x: [0u64; MAX_N + 1],
            shapes_visited: 0,
            max_n, max_cells: max_n * max_n,
        }
    }

    fn init_untried(&mut self) {
        for &(dr, dc) in &[(0, 1), (1, 0)] {
            let r = 0 + dr; let c = 0 + dc;
            if Self::valid_halfplane(r, c) {
                let bit = dfs_cell_bit(r, c) as u8;
                self.untried_mask |= 1u128 << bit;
                self.untried[self.untried_len as usize] = bit;
                self.untried_len += 1;
            }
        }
    }

    #[inline] fn valid_halfplane(r: i32, c: i32) -> bool { r >= 0 && (r > 0 || c >= 0) }
    #[inline] fn bit_in_range(r: i32, c: i32) -> bool {
        let b = dfs_cell_bit(r, c); b < 128
    }
    #[inline] fn bbox_w(&self) -> usize { (self.max_c - self.min_c + 1) as usize }
    #[inline] fn bbox_h(&self) -> usize { (self.max_r + 1) as usize }
    #[inline] fn bbox_ok(&self, r: i32, c: i32) -> bool {
        let w = (self.max_c.max(c) - self.min_c.min(c) + 1) as usize;
        let h = (self.max_r.max(r) + 1) as usize;
        w <= self.max_n && h <= self.max_n
    }
}

// ================================================================
// 归一化 + 统计 (转换到 bit_utils 的 STRIDE=8 编码)
// ================================================================

#[inline]
fn normalize_to_origin(occ: DfsMask, min_c: i32) -> u64 {
    let mut result: u64 = 0;
    let mut m = occ;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = (bit / DFS_STRIDE) as i32;
        let c = (bit % DFS_STRIDE) as i32 - DFS_OFFSET;
        result |= 1u64 << ((r as usize) * STRIDE + ((c - min_c) as usize));
        m &= m - 1;
    }
    result
}

#[inline]
fn count_shape(state: &mut RState, w: usize, h: usize) {
    if w.max(h) > state.max_n { return; }
    let norm = normalize_to_origin(state.occ, state.min_c);
    let has_hole = poly_has_hole(norm, w, h);
    let sym90 = has_symmetry_90(norm, w, h);
    let w4: u64 = if sym90 { 4 } else if has_symmetry_180(norm, w, h) { 2 } else { 1 };
    let md = w.max(h);
    for n in md..=state.max_n {
        state.total_4x[n] += w4;
        if has_hole { state.hole_4x[n] += w4; }
    }
}

// ================================================================
// DFS 核心
// ================================================================

fn dfs(state: &mut RState) {
    while state.untried_len > 0 {
        state.untried_len -= 1;
        let bit = state.untried[state.untried_len as usize];
        state.untried_mask &= !(1u128 << bit);
        let (r, c) = dfs_bit_rc(bit);

        state.occ |= 1u128 << bit; state.count += 1;
        let smr = state.max_r; let smc = state.max_c; let smic = state.min_c;
        state.max_r = state.max_r.max(r);
        state.max_c = state.max_c.max(c);
        state.min_c = state.min_c.min(c);

        let w = state.bbox_w(); let h = state.bbox_h();
        if w > state.max_n || h > state.max_n {
            state.count -= 1; state.occ &= !(1u128 << bit);
            state.max_r = smr; state.max_c = smc; state.min_c = smic;
            continue;
        }

        count_shape(state, w, h);
        state.shapes_visited += 1;

        if state.count as usize >= state.max_cells {
            state.count -= 1; state.occ &= !(1u128 << bit);
            state.max_r = smr; state.max_c = smc; state.min_c = smic;
            continue;
        }

        // 添加邻居
        let saved_untried_len = state.untried_len;
        for d in 0..4 {
            let nr = r + DR[d]; let nc = c + DC[d];
            if !RState::valid_halfplane(nr, nc) || !RState::bit_in_range(nr, nc) { continue; }
            let nbit = dfs_cell_bit(nr, nc) as u8;
            if (state.occ >> nbit) & 1 != 0 { continue; }
            if (state.untried_mask >> nbit) & 1 != 0 { continue; }
            if !state.bbox_ok(nr, nc) { continue; }
            state.untried_mask |= 1u128 << nbit;
            state.untried[state.untried_len as usize] = nbit;
            state.untried_len += 1;
        }

        // 保存/恢复 untried
        let saved_mask = state.untried_mask;
        let mut saved_untried: [u8; MAX_UNTRIED] = [0u8; MAX_UNTRIED];
        let cl = saved_untried_len as usize;
        saved_untried[..cl].copy_from_slice(&state.untried[..cl]);

        dfs(state);

        state.untried_len = saved_untried_len;
        state.untried_mask = saved_mask;
        state.untried[..cl].copy_from_slice(&saved_untried[..cl]);

        state.count -= 1; state.occ &= !(1u128 << bit);
        state.max_r = smr; state.max_c = smc; state.min_c = smic;
    }
}

// ================================================================
// 主入口 (串行)
// ================================================================

pub fn enumerate_redelmeier(max_n: usize, verbose: bool) -> Vec<RoomCount> {
    assert!(max_n <= MAX_N);
    let start = std::time::Instant::now();
    let total_4x: Arc<Vec<AtomicU64>> = Arc::new((0..=max_n).map(|_| AtomicU64::new(0)).collect());
    let hole_4x: Arc<Vec<AtomicU64>> = Arc::new((0..=max_n).map(|_| AtomicU64::new(0)).collect());
    let visited: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

    {
        let mut state = RState::new(max_n);
        state.init_untried();
        count_shape(&mut state, 1, 1);
        state.shapes_visited = 1;
        dfs(&mut state);
        if verbose {
            for n in 1..=max_n {
                eprintln!("  [DFS] n={} total_4x={} hole_4x={}",
                         n, state.total_4x[n], state.hole_4x[n]);
            }
        }
        for n in 0..=max_n {
            total_4x[n].fetch_add(state.total_4x[n], Ordering::Relaxed);
            hole_4x[n].fetch_add(state.hole_4x[n], Ordering::Relaxed);
        }
        visited.fetch_add(state.shapes_visited, Ordering::Relaxed);
    }

    let elapsed = start.elapsed();
    let mut results = Vec::with_capacity(max_n);
    for n in 1..=max_n {
        let t = total_4x[n].load(Ordering::Relaxed);
        let h = hole_4x[n].load(Ordering::Relaxed);
        results.push(RoomCount { n, total: t / 4, no_hole: (t - h) / 4, has_hole: h / 4 });
    }
    if verbose {
        eprintln!("  [Redelmeier] 访问形状: {} 耗时: {:.3}s",
                  visited.load(Ordering::Relaxed), elapsed.as_secs_f64());
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    macro_rules! test_n {
        ($name:ident, $n:expr, $exp:expr) => {
            #[test] fn $name() {
                let r = enumerate_redelmeier($n, false);
                assert_eq!(r[$n-1].total, $exp, "n={} failed: got {}", $n, r[$n-1].total);
            }
        };
    }
    test_n!(test_n1, 1, 1);
    test_n!(test_n2, 2, 4);
    test_n!(test_n3, 3, 46);
    test_n!(test_n4, 4, 2404);
    test_n!(test_n5, 5, 520818);
}
