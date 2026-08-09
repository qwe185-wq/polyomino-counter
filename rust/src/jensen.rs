//! Jensen 转移矩阵法 — Fixed Polyomino 逐列计数
//!
//! 对于 n≤6: 状态 ~800, DP 列×状态×高度×面积 ~ 1M 条目
//! 边界固定: row 0 必须在第0列被占用 (消除翻译重复)

use crate::types::*;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    labels: [u8; MAX_N],
}

impl State {
    fn empty() -> Self { Self { labels: [0u8; MAX_N] } }
    fn max_row(&self) -> usize {
        self.labels.iter().rposition(|&l| l != 0).map_or(0, |i| i)
    }
    fn is_empty(&self) -> bool { self.labels.iter().all(|&l| l == 0) }
}

fn normalize(labels: &mut [u8; MAX_N]) {
    let mut map = [0u8; MAX_N + 1]; let mut next: u8 = 1;
    for i in 0..MAX_N {
        let l = labels[i];
        if l != 0 {
            if map[l as usize] == 0 { map[l as usize] = next; next += 1; }
            labels[i] = map[l as usize];
        }
    }
}

struct JensenEngine {
    states: Vec<State>,
    /// transitions[sidx][occ] = Some((next_sidx, cells_added))
    transitions: Vec<[Option<(usize, usize)>; 64]>,
}

impl JensenEngine {
    fn new(max_n: usize) -> Self {
        let mut sidx_map: FxHashMap<State, usize> = FxHashMap::default();
        let mut states = Vec::new();
        let mut queue = VecDeque::new();

        // 空状态 (索引0, 用于终止)
        let empty = State::empty();
        states.push(empty);
        sidx_map.insert(empty, 0);

        // 第0列初始状态: row 0 必须占用 (边界固定)
        for occ in 1u64..(1u64 << max_n) {
            if (occ & 1) == 0 { continue; }
            let mut s = State::empty();
            // 分配临时标签，垂直邻接格共享标签
            let mut next_label: u8 = 1;
            for row in 0..max_n {
                if (occ >> row) & 1 == 0 { continue; }
                if row > 0 && s.labels[row - 1] != 0 {
                    s.labels[row] = s.labels[row - 1];
                } else {
                    s.labels[row] = next_label;
                    next_label += 1;
                }
            }
            normalize(&mut s.labels);
            if !sidx_map.contains_key(&s) {
                let idx = states.len();
                sidx_map.insert(s, idx);
                states.push(s);
                queue.push_back(idx);
            }
        }

        while let Some(sidx) = queue.pop_front() {
            let cur = states[sidx];
            for next_occ in 0u64..(1u64 << max_n) {
                if let Some(next) = Self::transition(&cur, next_occ, max_n) {
                    if !sidx_map.contains_key(&next) {
                        let idx = states.len();
                        sidx_map.insert(next, idx);
                        states.push(next);
                        queue.push_back(idx);
                    }
                }
            }
        }

        let n_states = states.len();
        let mut transitions: Vec<[Option<(usize, usize)>; 64]> =
            vec![[None; 64]; n_states];

        for sidx in 0..n_states {
            let cur = states[sidx];
            for occ in 0u64..(1u64 << max_n) {
                if let Some(next) = Self::transition(&cur, occ, max_n) {
                    let nsidx = sidx_map[&next];
                    let cells = occ.count_ones() as usize;
                    transitions[sidx][occ as usize] = Some((nsidx, cells));
                }
            }
        }

        Self { states, transitions }
    }

    fn transition(cur: &State, next_occ: u64, max_n: usize) -> Option<State> {
        let cur_nonempty = !cur.is_empty();
        let next_empty = next_occ == 0;

        // 空→空: 无意义
        if cur_nonempty && next_empty {
            // 形状终止: 返回空状态
            return Some(State::empty());
        }
        if !cur_nonempty && next_empty {
            return None; // 空再转空无意义
        }
        if !cur_nonempty && !next_empty {
            return None; // 空状态不能突然出现新格子（形状必须从row0 col0开始）
        }

        // cur 非空, next 非空
        let mut next = State::empty();

        // 检查连通性: next 中至少一个格子必须与 cur 连接
        let mut connected = false;
        for row in 0..max_n {
            if (next_occ >> row) & 1 == 0 { continue; }

            let left_label = cur.labels[row];
            if left_label != 0 {
                next.labels[row] = left_label;
                connected = true;
            } else {
                let mut label: u8 = 0;
                if row > 0 && (next_occ >> (row - 1)) & 1 != 0 {
                    label = next.labels[row - 1];
                }
                if label == 0 {
                    label = (max_n + 1) as u8;
                }
                next.labels[row] = label;
            }
        }

        if !connected {
            return None; // 断开
        }

        normalize(&mut next.labels);
        Some(next)
    }

    fn enumerate(&self, max_n: usize, verbose: bool) -> Vec<RoomCount> {
        let max_area = max_n * max_n;
        let n_states = self.states.len();

        // dp[state_idx][max_h][area] = count
        type DpMap = FxHashMap<(usize, usize, usize), u64>;
        let mut dp: DpMap = FxHashMap::default();
        let mut next_dp: DpMap;

        // 初始化: 第0列非空状态
        for (sidx, state) in self.states.iter().enumerate() {
            if state.is_empty() { continue; }
            let max_h = state.max_row();
            if max_h >= max_n { continue; }
            let area = state.labels.iter().filter(|&&l| l != 0).count();
            *dp.entry((sidx, max_h, area)).or_insert(0) += 1u64;
        }

        let idx_rc = |md: usize, area: usize| -> usize { area * (max_n + 1) + md };
        let mut results = vec![0u64; (max_area + 1) * (max_n + 1)];

        // 逐列推进 (含终止计数)
        for col in 0..max_n - 1 {
            next_dp = FxHashMap::default();

            for (&(sidx, max_h, area), &count) in dp.iter() {
                let trans = &self.transitions[sidx];

                for occ in 0usize..(1usize << max_n) {
                    if let Some((nsidx, cells_added)) = trans[occ] {
                        let next_state = &self.states[nsidx];
                        let new_area = area + cells_added;
                        if new_area > max_area { continue; }

                        if next_state.is_empty() {
                            // 形状终止
                            let w = col + 1; // 宽度 = 当前列索引+1
                            let h = max_h + 1;
                            let md = w.max(h);
                            if md <= max_n {
                                results[idx_rc(md, area)] += count;
                            }
                        } else {
                            let next_max_h = max_h.max(next_state.max_row());
                            if next_max_h >= max_n { continue; }
                            let key = (nsidx, next_max_h, new_area);
                            *next_dp.entry(key).or_insert(0) += count;
                        }
                    }
                }
            }

            dp = next_dp;
        }

        // 最后一列剩余状态 (无法再扩展)
        for (&(sidx, max_h, area), &count) in dp.iter() {
            if self.states[sidx].is_empty() { continue; }
            let w = max_n; // 已达最后一列
            let h = max_h + 1;
            let md = w.max(h);
            if md <= max_n {
                results[idx_rc(md, area)] += count;
            }
        }

        if verbose {
            eprintln!("  [Jensen] 状态数: {} n={} results:", n_states, max_n);
            for md in 1..=max_n {
                for area in 1..=max_area {
                    let c = results[idx_rc(md, area)];
                    if c > 0 { eprintln!("    md={} area={} count={}", md, area, c); }
                }
            }
        }

        // 汇总 RoomCount (按 bounding box ≤ n)
        let mut room_counts = Vec::with_capacity(max_n);
        for n in 1..=max_n {
            let mut total: u64 = 0;
            for md in 1..=n {
                for area in 1..=max_area {
                    total += results[idx_rc(md, area)];
                }
            }
            room_counts.push(RoomCount { n, total, no_hole: total, has_hole: 0 });
        }
        room_counts
    }
}

pub fn enumerate_jensen(max_n: usize, verbose: bool) -> Vec<RoomCount> {
    JensenEngine::new(max_n).enumerate(max_n, verbose)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_n1() { assert_eq!(enumerate_jensen(1, false)[0].total, 1); }
    #[test] fn test_n2() { assert_eq!(enumerate_jensen(2, false)[1].total, 8); }
    #[test] fn test_n3() { assert_eq!(enumerate_jensen(3, false)[2].total, 151); }
}
