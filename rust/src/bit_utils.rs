//! 位运算工具 — 掩码操作、归一化、洞检测、前沿计算
//!
//! 所有核心位操作集中在此模块，便于 SIMD 友好优化。

use crate::types::*;

// ================================================================
// 基础位操作
// ================================================================

/// 提取掩码中所有置位格子的 (row, col) 坐标
#[inline]
pub fn mask_to_cells(mask: Mask, count: &mut usize, cells_r: &mut [usize], cells_c: &mut [usize]) {
    *count = 0;
    let mut m = mask;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        cells_r[*count] = bit >> STRIDE_SHIFT;
        cells_c[*count] = bit & (STRIDE - 1);
        *count += 1;
        m &= m - 1;
    }
}

/// 计算掩码的包围矩形尺寸
#[inline]
pub fn mask_extent(mask: Mask) -> (usize, usize) {
    if mask == 0 {
        return (0, 0);
    }
    let mut min_r = 64usize;
    let mut max_r = 0usize;
    let mut min_c = 64usize;
    let mut max_c = 0usize;
    let mut m = mask;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = bit >> STRIDE_SHIFT;
        let c = bit & (STRIDE - 1);
        if r < min_r {
            min_r = r;
        }
        if r > max_r {
            max_r = r;
        }
        if c < min_c {
            min_c = c;
        }
        if c > max_c {
            max_c = c;
        }
        m &= m - 1;
    }
    (max_c - min_c + 1, max_r - min_r + 1)
}

// ================================================================
// 平移归一化
// ================================================================

/// 将掩码平移归一化到 (0,0) 原点
///
/// 这是 Fixed polyomino 唯一需要的规范化操作。
/// 相比 One-sided 的 4 旋转 + 4 归一化，便宜 ~4×。
#[inline]
pub fn normalize_translation(mask: Mask) -> (Mask, usize, usize) {
    if mask == 0 {
        return (0, 0, 0);
    }

    // 单次遍历同时找到最小行列和构造归一化掩码
    let mut min_r = 64u32;
    let mut min_c = 64u32;
    let mut m = mask;

    // 第一遍：找 min_r, min_c
    while m != 0 {
        let bit = m.trailing_zeros();
        let r = bit >> STRIDE_SHIFT as u32;
        let c = bit & (STRIDE as u32 - 1);
        if r < min_r {
            min_r = r;
        }
        if c < min_c {
            min_c = c;
        }
        m &= m - 1;
    }

    // 第二遍：平移构造
    let mut result: Mask = 0;
    m = mask;
    while m != 0 {
        let bit = m.trailing_zeros();
        let r = (bit >> STRIDE_SHIFT as u32) - min_r;
        let c = (bit & (STRIDE as u32 - 1)) - min_c;
        result |= 1u64 << (r * STRIDE as u32 + c);
        m &= m - 1;
    }

    // 需要 w,h 的话再算 extent
    let (w, h) = mask_extent(result);
    (result, w, h)
}

// ================================================================
// 90° 旋转 (用于 One-sided 规范化和对称性检测)
// ================================================================

/// 顺时针旋转 90°
#[inline]
pub fn rotate90(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
    let mut result: Mask = 0;
    let mut m = mask;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = bit >> STRIDE_SHIFT;
        let c = bit & (STRIDE - 1);
        // 旋转: (r, c) -> (c, h-1-r)
        result |= 1u64 << (c * STRIDE + (h - 1 - r));
        m &= m - 1;
    }
    (result, h, w) // 旋转后宽高互换
}

/// 计算 One-sided canonical form（4 方向取最小）
///
/// 仍保留此函数，用于对称枚举和 Jensen 验证。
#[inline]
pub fn compute_canonical(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
    let mut best: Mask = u64::MAX;
    let mut bw = 0usize;
    let mut bh = 0usize;

    let (mut cur, mut cw, mut ch) = (mask, w, h);

    for _rot in 0..4 {
        let (norm, nw, nh) = normalize_translation(cur);
        if norm < best {
            best = norm;
            bw = nw;
            bh = nh;
        }
        let (r, rw, rh) = rotate90(cur, cw, ch);
        cur = r;
        cw = rw;
        ch = rh;
    }

    (best, bw, bh)
}

// ================================================================
// 前沿计算（位掩码优化版）
// ================================================================

/// 用位掩码替代 memset bool 数组的前沿计算
///
/// 核心优化：
/// - 用两个 u128 分别表示 occ 和 inf 位掩码（10×10=100 bits < 128）
/// - 单指令清零、置位、测试
/// - 避免 200 字节 memset 开销
#[inline]
pub fn compute_frontier_mask(
    pmask: Mask,
    pw: usize,
    ph: usize,
    box_at_max: bool,
    max_n: usize,
) -> smallvec::SmallVec<[(usize, usize); 256]> {
    // occ: 10×10 网格，用 u128 位掩码
    // bit (r, c) → bit index = r * 10 + c
    let mut occ: u128 = 0;
    let mut m = pmask;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = (bit >> STRIDE_SHIFT) + 1; // +1 padding
        let c = (bit & (STRIDE - 1)) + 1;
        occ |= 1u128 << (r * 10 + c);
        m &= m - 1;
    }

    let mut inf: u128 = 0;
    let mut frontier = smallvec::SmallVec::new();

    // 遍历每个已占格子的邻居
    m = pmask;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = bit >> STRIDE_SHIFT;
        let c = bit & (STRIDE - 1);

        // 4 个方向
        for (dr, dc) in &[(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            let nr = r as i32 + dr;
            let nc = c as i32 + dc;

            // 检查 occ 中是否已有
            let nri = (nr + 1) as usize;
            let nci = (nc + 1) as usize;
            if (occ >> (nri * 10 + nci)) & 1 != 0 {
                continue;
            }

            // 检查 inf 中是否已标记
            let inf_bit = 1u128 << (nri * 10 + nci);
            if (inf & inf_bit) != 0 {
                continue;
            }

            // 包围盒检查
            if box_at_max {
                if nr < 0 || nr >= ph as i32 || nc < 0 || nc >= pw as i32 {
                    continue;
                }
            } else {
                let mut nw = pw;
                let mut nh = ph;
                if nc < 0 {
                    nw += 1;
                } else if nc >= pw as i32 {
                    nw = nc as usize + 1;
                }
                if nr < 0 {
                    nh += 1;
                } else if nr >= ph as i32 {
                    nh = nr as usize + 1;
                }
                if nw > max_n || nh > max_n {
                    continue;
                }
            }

            inf |= inf_bit;
            frontier.push((nr as usize, nc as usize));
        }
        m &= m - 1;
    }

    frontier
}

/// 简化的前沿计算（返回 Vec）
///
/// 对于小规模 (n≤6)，用固定数组比 u128 位掩码更直观。
/// 两种实现供性能对比。
#[inline]
#[allow(unused_variables)]
pub fn compute_frontier(
    pmask: Mask,
    pw: usize,
    ph: usize,
    box_at_max: bool,
    max_n: usize,
    cells_r: &[usize],
    cells_c: &[usize],
    cc: usize,
) -> (Vec<(usize, usize)>, [u16; GRID_PAD]) {
    // 用 u16 位掩码表示 inf（每行一个 u16，10 行）
    // bit c 表示列 c 是否已标记
    let mut inf_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];

    // 用两个 u64 表示 occ（每行一个 u64 的低 10 位）
    let mut occ_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];
    for i in 0..cc {
        let r = cells_r[i] + 1;
        let c = cells_c[i] + 1;
        occ_rows[r] |= 1u16 << c;
    }

    let mut frontier = Vec::with_capacity(256);

    static DR: [i32; 4] = [-1, 1, 0, 0];
    static DC: [i32; 4] = [0, 0, -1, 1];

    for i in 0..cc {
        let r = cells_r[i] as i32;
        let c = cells_c[i] as i32;

        for d in 0..4 {
            let nr = r + DR[d];
            let nc = c + DC[d];

            let nri = (nr + 1) as usize;
            let nci = (nc + 1) as usize;

            // 已占用
            if (occ_rows[nri] >> nci) & 1 != 0 {
                continue;
            }
            // 已标记
            if (inf_rows[nri] >> nci) & 1 != 0 {
                continue;
            }

            // 包围盒检查
            if box_at_max {
                if nr < 0 || nr >= ph as i32 || nc < 0 || nc >= pw as i32 {
                    continue;
                }
            } else {
                let mut nw = pw;
                let mut nh = ph;
                if nc < 0 {
                    nw += 1;
                } else if nc >= pw as i32 {
                    nw = nc as usize + 1;
                }
                if nr < 0 {
                    nh += 1;
                } else if nr >= ph as i32 {
                    nh = nr as usize + 1;
                }
                if nw > max_n || nh > max_n {
                    continue;
                }
            }

            inf_rows[nri] |= 1u16 << nci;
            frontier.push((nr as usize, nc as usize));
        }
    }

    (frontier, inf_rows)
}

// ================================================================
// GROW 操作：在 (nr,nc) 处扩展形状
// ================================================================

/// 在指定位置扩展形状，返回新的平移归一化掩码
///
/// new_mask 已归一化（左上角对齐 (0,0)），可直接用于去重比较。
#[inline]
pub fn grow_mask(
    pmask: Mask,
    pw: usize,
    ph: usize,
    nr: i32,
    nc: i32,
) -> (Mask, usize, usize) {
    // 计算平移量
    let sr = if nr < 0 { 1usize } else { 0usize };
    let sc = if nc < 0 { 1usize } else { 0usize };

    // 平移已有掩码
    let mut new_mask: Mask = 0;
    for r in 0..ph {
        let row = (pmask >> (r * STRIDE)) & ((1u64 << pw) - 1);
        new_mask |= (row << sc) << ((r + sr) * STRIDE);
    }

    // 添加新格子
    let nr_abs = (nr + sr as i32) as usize;
    let nc_abs = (nc + sc as i32) as usize;
    new_mask |= 1u64 << (nr_abs * STRIDE + nc_abs);

    // 计算新包围盒
    let mut raw_w = pw;
    let mut raw_h = ph;
    if nc < 0 {
        raw_w += 1;
    } else if nc as usize >= pw {
        raw_w = nc as usize + 1;
    }
    if nr < 0 {
        raw_h += 1;
    } else if nr as usize >= ph {
        raw_h = nr as usize + 1;
    }

    (new_mask, raw_w, raw_h)
}

// ================================================================
// 洞检测（BFS flood fill）
// ================================================================

/// 检测多联骨牌是否有洞
///
/// 从包围矩形外部 BFS 泛洪，若存在无法从外部到达的空格子 → 有洞。
/// 用 u16 位掩码替代 bool 数组，减少内存操作。
#[inline]
pub fn poly_has_hole(cells: Mask, w: usize, h: usize) -> bool {
    if w < 3 || h < 3 {
        return false;
    }

    let gh = h + 2;
    let gw = w + 2;

    // occ: 已占格子位掩码
    let mut occ_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];
    let mut m = cells;
    while m != 0 {
        let bit = m.trailing_zeros() as usize;
        let r = (bit >> STRIDE_SHIFT) + 1;
        let c = (bit & (STRIDE - 1)) + 1;
        occ_rows[r] |= 1u16 << c;
        m &= m - 1;
    }

    // vis: 已访问格子位掩码
    let mut vis_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];

    // BFS 队列（环形缓冲）
    let mut qr = [0u16; GRID_PAD * GRID_PAD];
    let mut qc = [0u16; GRID_PAD * GRID_PAD];
    let mut head = 0usize;
    let mut tail = 0usize;

    qr[tail] = 0;
    qc[tail] = 0;
    tail += 1;
    vis_rows[0] |= 1u16;

    static DR: [i32; 4] = [-1, 1, 0, 0];
    static DC: [i32; 4] = [0, 0, -1, 1];

    while head < tail {
        let r = qr[head] as i32;
        let c = qc[head] as i32;
        head += 1;

        for d in 0..4 {
            let nr = r + DR[d];
            let nc = c + DC[d];

            if nr < 0 || nr >= gh as i32 || nc < 0 || nc >= gw as i32 {
                continue;
            }

            let nru = nr as usize;
            let ncu = nc as usize;
            let bit = 1u16 << ncu;

            if (vis_rows[nru] & bit) != 0 || (occ_rows[nru] & bit) != 0 {
                continue;
            }

            vis_rows[nru] |= bit;
            qr[tail] = nru as u16;
            qc[tail] = ncu as u16;
            tail += 1;
        }
    }

    // 检查内部是否有未被访问的空格
    for r in 1..=h {
        for c in 1..=w {
            let bit = 1u16 << c;
            if (occ_rows[r] & bit) == 0 && (vis_rows[r] & bit) == 0 {
                return true;
            }
        }
    }

    false
}

// ================================================================
// popcount
// ================================================================

/// 掩码中置位计数
#[inline]
pub fn popcount(mask: Mask) -> usize {
    mask.count_ones() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_translation_single_cell() {
        let mask: Mask = 1u64 << (3 * STRIDE + 4); // cell at (3,4)
        let (norm, w, h) = normalize_translation(mask);
        assert_eq!(norm, 1u64); // should be at (0,0)
        assert_eq!(w, 1);
        assert_eq!(h, 1);
    }

    #[test]
    fn test_normalize_translation_domino() {
        // two cells at (2,3) and (2,4)
        let mask: Mask = (1u64 << (2 * STRIDE + 3)) | (1u64 << (2 * STRIDE + 4));
        let (norm, w, h) = normalize_translation(mask);
        assert_eq!(norm, (1u64 << 0) | (1u64 << 1)); // should be at (0,0) and (0,1)
        assert_eq!(w, 2);
        assert_eq!(h, 1);
    }

    #[test]
    fn test_hole_detection_no_hole() {
        // 2x2 square, no hole
        let mask: Mask = (1 << 0) | (1 << 1) | (1 << STRIDE) | (1 << (STRIDE + 1));
        assert!(!poly_has_hole(mask, 2, 2));
    }

    #[test]
    fn test_hole_detection_with_hole() {
        // 3x3 ring, has hole
        let mask: Mask = (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << STRIDE)
            | (1 << (STRIDE + 2))
            | (1 << (2 * STRIDE))
            | (1 << (2 * STRIDE + 1))
            | (1 << (2 * STRIDE + 2));
        assert!(poly_has_hole(mask, 3, 3));
    }

    #[test]
    fn test_grow_mask() {
        // Single cell at (0,0), grow to (1,0)
        let pmask: Mask = 1;
        let (new_mask, w, h) = grow_mask(pmask, 1, 1, 1, 0);
        assert_eq!(new_mask, 1 | (1 << STRIDE)); // cells at (0,0) and (1,0)
        assert_eq!(w, 1);
        assert_eq!(h, 2);

        // Grow left: (-1, 0) → should shift right by 1
        let (new_mask2, w2, h2) = grow_mask(pmask, 1, 1, -1, 0);
        assert_eq!(new_mask2, (1 << STRIDE) | 1); // original shifted down, new at (0,0)
        assert_eq!(h2, 2);
    }
}
