//! Redelmeier (1981) DFS — 最简实现用于调试
//! 策略: 生成所有 Fixed shapes + HashSet 去重 (与 BFS 相同逻辑，但用 DFS 生成)
//! 目标: 验证 DFS 生成逻辑，隔离约束问题

use crate::bit_utils::*;
use crate::symmetric::{has_symmetry_180, has_symmetry_90};
use crate::types::*;
use rustc_hash::FxHashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// ================================================================
// 简化坐标编码 (无偏移)
// ================================================================

const MAX_UNTRIED: usize = 256;

struct RState {
    occ: u64,          // 位掩码: bit = r*STRIDE + c (标准编码)
    untried: [u8; MAX_UNTRIED],
    untried_len: u8,
    untried_mask: u64, // O(1) 查找
    count: u8,
    max_r: u8, max_c: u8, min_r: u8, min_c: u8, // 用于包围盒追踪
    max_n: usize, max_cells: usize,
}

impl RState {
    fn new(max_n: usize) -> Self {
        Self {
            occ: 1u64, // cell at (0,0)
            untried: [0u8; MAX_UNTRIED], untried_len: 0, untried_mask: 0,
            count: 1, max_r: 0, max_c: 0, min_r: 0, min_c: 0,
            max_n, max_cells: max_n * max_n,
        }
    }

    fn init_untried(&mut self) {
        // 仅添加两个方向的邻居（限制在 r≥0, c≥0 象限）
        for &(r, c) in &[(0usize, 1usize), (1, 0)] {
            let bit = (r * STRIDE + c) as u8;
            self.untried_mask |= 1u64 << bit;
            self.untried[self.untried_len as usize] = bit;
            self.untried_len += 1;
        }
    }

    #[inline]
    fn bbox_ok(&self, nr: usize, nc: usize) -> bool {
        let new_min_r = self.min_r.min(nr as u8);
        let new_min_c = self.min_c.min(nc as u8);
        let new_max_r = self.max_r.max(nr as u8);
        let new_max_c = self.max_c.max(nc as u8);
        let w = new_max_c - new_min_c + 1;
        let h = new_max_r - new_min_r + 1;
        w as usize <= self.max_n && h as usize <= self.max_n
    }

    fn normalized_mask(&self) -> u64 {
        let mut result: u64 = 0;
        let mut m = self.occ;
        while m != 0 {
            let bit = m.trailing_zeros() as usize;
            let r = (bit >> STRIDE_SHIFT) - self.min_r as usize;
            let c = (bit & (STRIDE - 1)) - self.min_c as usize;
            result |= 1u64 << (r * STRIDE + c);
            m &= m - 1;
        }
        result
    }
}

// ================================================================
// DFS (象限约束 r≥0, c≥0 — 与 BFS 相同)
// ================================================================

static DR: [i32; 4] = [0, 1, 0, -1];
static DC: [i32; 4] = [1, 0, -1, 0];

fn dfs(state: &mut RState, seen: &mut FxHashSet<u64>) {
    while state.untried_len > 0 {
        state.untried_len -= 1;
        let bit = state.untried[state.untried_len as usize];
        state.untried_mask &= !(1u64 << bit);
        let r = (bit as usize) >> STRIDE_SHIFT;
        let c = (bit as usize) & (STRIDE - 1);

        // 添加到形状
        state.occ |= 1u64 << bit;
        state.count += 1;
        let smr = state.max_r; let smc = state.max_c;
        let smir = state.min_r; let smic = state.min_c;
        state.max_r = state.max_r.max(r as u8);
        state.max_c = state.max_c.max(c as u8);
        state.min_r = state.min_r.min(r as u8);
        state.min_c = state.min_c.min(c as u8);

        // 包围盒检查
        let w = state.max_c - state.min_c + 1;
        let h = state.max_r - state.min_r + 1;
        if w as usize <= state.max_n && h as usize <= state.max_n {
            // 记录 (翻译归一化 + 去重)
            let norm = state.normalized_mask();
            if !seen.contains(&norm) {
                seen.insert(norm);
            }

            // 继续探索
            if (state.count as usize) < state.max_cells {
                let saved_untried_len = state.untried_len;
                for d in 0..4 {
                    let nr = r as i32 + DR[d];
                    let nc = c as i32 + DC[d];
                    if nr < 0 || nc < 0 { continue; }
                    let nru = nr as usize; let ncu = nc as usize;
                    let nbit = (nru * STRIDE + ncu) as u8;
                    if (state.occ >> nbit) & 1 != 0 { continue; }
                    if (state.untried_mask >> nbit) & 1 != 0 { continue; }
                    if !state.bbox_ok(nru, ncu) { continue; }
                    state.untried_mask |= 1u64 << nbit;
                    state.untried[state.untried_len as usize] = nbit;
                    state.untried_len += 1;
                }

                dfs(state, seen);

                // 恢复 untried
                let saved_mask = state.untried_mask;
                let mut saved: [u8; MAX_UNTRIED] = [0u8; MAX_UNTRIED];
                let cl = saved_untried_len as usize;
                saved[..cl].copy_from_slice(&state.untried[..cl]);
                state.untried_len = saved_untried_len;
                state.untried_mask = saved_mask;
                state.untried[..cl].copy_from_slice(&saved[..cl]);
            }
        }

        // 回溯
        state.count -= 1;
        state.occ &= !(1u64 << bit);
        state.max_r = smr; state.max_c = smc;
        state.min_r = smir; state.min_c = smic;
    }
}

// ================================================================
// 主入口 — 生成 Fixed shapes + 统计 One-sided
// ================================================================

pub fn enumerate_redelmeier(max_n: usize, verbose: bool) -> Vec<RoomCount> {
    assert!(max_n <= MAX_N);
    let start = std::time::Instant::now();

    let mut seen: FxHashSet<u64> = FxHashSet::default();
    seen.reserve(1 << 20);

    let mut state = RState::new(max_n);
    state.init_untried();

    // 记录单格形状
    seen.insert(1u64); // single cell at (0,0)

    dfs(&mut state, &mut seen);

    let unique_count = seen.len();

    // 对每个 canonical mask 分类统计
    let mut total_4x = [0u64; MAX_N + 1];
    let mut hole_4x = [0u64; MAX_N + 1];

    for &mask in seen.iter() {
        let (w, h) = mask_extent(mask);
        if w > max_n || h > max_n { continue; }
        let has_hole = poly_has_hole(mask, w, h);
        let sym90 = has_symmetry_90(mask, w, h);
        let w4: u64 = if sym90 { 4 } else if has_symmetry_180(mask, w, h) { 2 } else { 1 };
        let md = w.max(h);
        for n in md..=max_n {
            total_4x[n] += w4;
            if has_hole { hole_4x[n] += w4; }
        }
    }

    let elapsed = start.elapsed();
    let mut results = Vec::with_capacity(max_n);
    for n in 1..=max_n {
        results.push(RoomCount {
            n, total: total_4x[n] / 4, no_hole: (total_4x[n] - hole_4x[n]) / 4,
            has_hole: hole_4x[n] / 4,
        });
    }

    if verbose {
        eprintln!("  [Redelmeier+HashSet] unique={} time={:.3}s",
                 unique_count, elapsed.as_secs_f64());
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_n1() { assert_eq!(enumerate_redelmeier(1, false)[0].total, 1); }
    #[test] fn test_n2() { assert_eq!(enumerate_redelmeier(2, false)[1].total, 4); }
    #[test] fn test_n3() { assert_eq!(enumerate_redelmeier(3, false)[2].total, 46); }
    #[test] fn test_n4() { assert_eq!(enumerate_redelmeier(4, false)[3].total, 2404); }
    #[test] fn test_n5() { assert_eq!(enumerate_redelmeier(5, false)[4].total, 520818); }
}
