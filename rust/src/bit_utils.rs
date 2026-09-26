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
    let rows = mask | (mask >> 32);
    let rows = rows | (rows >> 16);
    let columns = (rows | (rows >> 8)) as u8;
    let w = 8 - columns.leading_zeros() - columns.trailing_zeros();
    let h = (63 - mask.leading_zeros()) / 8 - mask.trailing_zeros() / 8 + 1;
    (w as usize, h as usize)
}

// ================================================================
// 平移归一化
// ================================================================

/// 将掩码平移归一化到 (0,0) 原点
///
/// 用行列投影计算边界，无需逐格扫描。
#[inline]
pub fn normalize_translation(mask: Mask) -> (Mask, usize, usize) {
    if mask == 0 {
        return (0, 0, 0);
    }
    let rows = mask | (mask >> 32);
    let rows = rows | (rows >> 16);
    let columns = (rows | (rows >> 8)) as u8;
    let min_row = mask.trailing_zeros() / 8;
    let min_col = columns.trailing_zeros();
    let w = 8 - columns.leading_zeros() - min_col;
    let h = (63 - mask.leading_zeros()) / 8 - min_row + 1;
    (mask >> (min_row * 8 + min_col), w as usize, h as usize)
}

// ================================================================
// 90° 旋转 (用于 One-sided 规范化和对称性检测)
// ================================================================

/// 顺时针旋转 90°
#[inline]
pub fn rotate90(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
    // 8×8 位矩阵转置，再反转每个字节的列顺序并裁回紧包围盒。
    let mut x = mask;
    let t = (x ^ (x >> 7)) & 0x00aa00aa00aa00aa;
    x ^= t ^ (t << 7);
    let t = (x ^ (x >> 14)) & 0x0000cccc0000cccc;
    x ^= t ^ (t << 14);
    let t = (x ^ (x >> 28)) & 0x00000000f0f0f0f0;
    x ^= t ^ (t << 28);
    (x.reverse_bits().swap_bytes() >> (8 - h), h, w)
}

/// 计算 One-sided canonical form（4 方向取最小）
///
/// 输入须非空且已经平移归一化，w/h为紧包围盒。
#[inline]
pub fn compute_canonical(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
    let (mut best, mut bw, mut bh) = (mask, w, h);
    let (mut cur, mut cw, mut ch) = (mask, w, h);
    for _ in 0..3 {
        (cur, cw, ch) = rotate90(cur, cw, ch);
        if cur < best {
            (best, bw, bh) = (cur, cw, ch);
        }
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
pub fn grow_mask(pmask: Mask, pw: usize, ph: usize, nr: i32, nc: i32) -> (Mask, usize, usize) {
    let sr = usize::from(nr < 0);
    let sc = usize::from(nc < 0);
    let mask = (pmask << (sr * STRIDE + sc))
        | (1u64 << ((nr + sr as i32) as usize * STRIDE + (nc + sc as i32) as usize));
    let w = if nc < 0 {
        pw + 1
    } else {
        pw.max(nc as usize + 1)
    };
    let h = if nr < 0 {
        ph + 1
    } else {
        ph.max(nr as usize + 1)
    };
    (mask, w, h)
}

// ================================================================
// 洞检测（BFS flood fill）
// ================================================================

/// 检测多联骨牌是否有洞
///
/// 通过闭方格复形的欧拉特征数，判断是否存在背景四邻接封闭区域。
#[inline]
pub fn poly_has_hole(cells: Mask, w: usize, h: usize) -> bool {
    if w < 3 || h < 3 {
        return false;
    }
    // 前提：非空、四连通、左上归一化且宽高≤6。
    // 闭方格复形 V-E+F=1-holes，背景四邻接，保持对角缝隙的原判定。
    debug_assert!(cells != 0 && w <= MAX_N && h <= MAX_N);
    let faces = cells.count_ones();
    let edges = (cells | (cells << 8)).count_ones() + (cells | (cells << 1)).count_ones();
    let vertices = (cells | (cells << 1) | (cells << 8) | (cells << 9)).count_ones();
    edges + 1 > vertices + faces
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
        assert_eq!(w2, 1);
        assert_eq!(new_mask2, (1 << STRIDE) | 1); // original shifted down, new at (0,0)
        assert_eq!(h2, 2);
    }
}
