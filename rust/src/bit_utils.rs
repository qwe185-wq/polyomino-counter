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

/// 在 stride=8 的紧包围盒内旋转 180°。
#[inline]
fn rotate180(mask: Mask, w: usize, h: usize) -> Mask {
    mask.reverse_bits() >> (64 - ((h - 1) * STRIDE + w))
}

/// 计算 One-sided canonical form（4 方向取最小）
///
/// 输入须非空且已经平移归一化，w/h为紧包围盒。
#[inline]
pub fn compute_canonical(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
    // 最高置位所在的行决定数值量级；较矮的方向必然更小。
    if w > h {
        return (mask.min(rotate180(mask, w, h)), w, h);
    }

    let (rotated, _, _) = rotate90(mask, w, h);
    let rotated_best = rotated.min(rotate180(rotated, h, w));
    if w < h {
        return (rotated_best, h, w);
    }

    (mask.min(rotate180(mask, w, h)).min(rotated_best), w, h)
}

/// 父形状的四个旋转方向，供多个扩展候选复用。
#[derive(Clone, Copy)]
pub(crate) struct ParentRotations {
    p0: Mask,
    p90: Mask,
    p180: Mask,
    p270: Mask,
}

#[derive(Clone, Copy)]
struct Growth {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    w: usize,
    h: usize,
    row: usize,
    col: usize,
}

impl Growth {
    #[inline]
    fn new(pw: usize, ph: usize, r: i32, c: i32) -> Self {
        let left = usize::from(c < 0);
        let top = usize::from(r < 0);
        let right = usize::from(c >= pw as i32);
        let bottom = usize::from(r >= ph as i32);
        Self {
            left,
            top,
            right,
            bottom,
            w: pw + left + right,
            h: ph + top + bottom,
            row: (r + top as i32) as usize,
            col: (c + left as i32) as usize,
        }
    }
}

impl ParentRotations {
    #[inline]
    pub(crate) fn new(mask: Mask, w: usize, h: usize) -> Self {
        let p90 = rotate90(mask, w, h).0;
        Self {
            p0: mask,
            p90,
            p180: rotate180(mask, w, h),
            p270: rotate180(p90, h, w),
        }
    }

    #[inline]
    fn child0(self, g: Growth) -> Mask {
        (self.p0 << (STRIDE * g.top + g.left)) | (1u64 << (STRIDE * g.row + g.col))
    }

    #[inline]
    fn child90(self, g: Growth) -> Mask {
        (self.p90 << (STRIDE * g.left + g.bottom)) | (1u64 << (STRIDE * g.col + g.h - 1 - g.row))
    }

    #[inline]
    fn child180(self, g: Growth) -> Mask {
        (self.p180 << (STRIDE * g.bottom + g.right))
            | (1u64 << (STRIDE * (g.h - 1 - g.row) + g.w - 1 - g.col))
    }

    #[inline]
    fn child270(self, g: Growth) -> Mask {
        (self.p270 << (STRIDE * g.right + g.top)) | (1u64 << (STRIDE * (g.w - 1 - g.col) + g.row))
    }

    /// 从父形状与新增格直接生成孩子的旋转最小代表。
    #[inline]
    pub(crate) fn grow_canonical(
        self,
        pw: usize,
        ph: usize,
        r: i32,
        c: i32,
    ) -> (Mask, usize, usize) {
        let g = Growth::new(pw, ph, r, c);
        if g.w > g.h {
            return (self.child0(g).min(self.child180(g)), g.w, g.h);
        }
        if g.w < g.h {
            return (self.child90(g).min(self.child270(g)), g.h, g.w);
        }
        (
            self.child0(g)
                .min(self.child90(g))
                .min(self.child180(g))
                .min(self.child270(g)),
            g.w,
            g.h,
        )
    }
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
    use std::collections::HashSet;

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

    // 测试 oracle 逐格搬移坐标，不复用生产代码的位转置或 reverse_bits。
    fn coordinate_normalize(mask: Mask) -> (Mask, usize, usize) {
        let mut min_r = STRIDE;
        let mut min_c = STRIDE;
        let mut max_r = 0;
        let mut max_c = 0;
        for r in 0..STRIDE {
            for c in 0..STRIDE {
                if mask & (1u64 << (r * STRIDE + c)) != 0 {
                    min_r = min_r.min(r);
                    min_c = min_c.min(c);
                    max_r = max_r.max(r);
                    max_c = max_c.max(c);
                }
            }
        }
        let mut normalized = 0;
        for r in min_r..=max_r {
            for c in min_c..=max_c {
                if mask & (1u64 << (r * STRIDE + c)) != 0 {
                    normalized |= 1u64 << ((r - min_r) * STRIDE + c - min_c);
                }
            }
        }
        (normalized, max_c - min_c + 1, max_r - min_r + 1)
    }

    fn coordinate_rotate90(mask: Mask, w: usize, h: usize) -> (Mask, usize, usize) {
        let mut rotated = 0;
        for r in 0..h {
            for c in 0..w {
                if mask & (1u64 << (r * STRIDE + c)) != 0 {
                    rotated |= 1u64 << (c * STRIDE + h - 1 - r);
                }
            }
        }
        (rotated, h, w)
    }

    fn assert_rotations_match_coordinate_oracle(mask: Mask) {
        let (normalized, w, h) = coordinate_normalize(mask);
        assert_eq!(normalize_translation(mask), (normalized, w, h));

        let mut current = (normalized, w, h);
        let mut expected = current;
        for turn in 0..4 {
            if turn > 0 {
                current = coordinate_rotate90(current.0, current.1, current.2);
            }
            if current.0 < expected.0 {
                expected = current;
            }
            assert_eq!(
                rotate90(current.0, current.1, current.2),
                coordinate_rotate90(current.0, current.1, current.2)
            );
            let twice = coordinate_rotate90(current.0, current.1, current.2);
            let twice = coordinate_rotate90(twice.0, twice.1, twice.2);
            assert_eq!(rotate180(current.0, current.1, current.2), twice.0);
        }
        assert_eq!(
            compute_canonical(normalized, w, h),
            expected,
            "mask={mask:#018x}, bbox={w}x{h}"
        );
    }

    #[test]
    fn test_canonical_exhaustive_4x4_coordinate_oracle() {
        // 全部非空 4×4 位图：含细长、方形、对称和非对称形状。
        for raw in 1u32..(1 << 16) {
            let mut mask = 0u64;
            for cell in 0..16 {
                if raw & (1 << cell) != 0 {
                    mask |= 1u64 << ((cell / 4) * STRIDE + cell % 4);
                }
            }
            assert_rotations_match_coordinate_oracle(mask);
        }
    }

    #[test]
    fn test_canonical_stride8_boundary_coordinate_oracle() {
        // 最高位、整行、整列及 8×8 方形均触及 u64 位宽边界。
        for mask in [
            1u64 | (1u64 << 7),
            1u64 | (1u64 << 56),
            1u64 | (1u64 << 7) | (1u64 << 56) | (1u64 << 63),
            u64::MAX,
            0x8040_2010_0804_0201,
        ] {
            assert_rotations_match_coordinate_oracle(mask);
        }
    }

    fn coordinate_grow(mask: Mask, w: usize, h: usize, r: i32, c: i32) -> (Mask, usize, usize) {
        let mut cells = Vec::new();
        for row in 0..h {
            for col in 0..w {
                if mask & (1u64 << (row * STRIDE + col)) != 0 {
                    cells.push((row as i32, col as i32));
                }
            }
        }
        cells.push((r, c));
        let min_row = cells.iter().map(|cell| cell.0).min().unwrap();
        let min_col = cells.iter().map(|cell| cell.1).min().unwrap();
        let max_row = cells.iter().map(|cell| cell.0).max().unwrap();
        let max_col = cells.iter().map(|cell| cell.1).max().unwrap();
        let mut grown = 0;
        for (row, col) in cells {
            grown |= 1u64 << (((row - min_row) as usize) * STRIDE + (col - min_col) as usize);
        }
        (
            grown,
            (max_col - min_col + 1) as usize,
            (max_row - min_row + 1) as usize,
        )
    }

    fn coordinate_connected(mask: Mask, w: usize, h: usize) -> bool {
        let mut reached = 1u64 << mask.trailing_zeros();
        loop {
            let mut next = reached;
            for row in 0..h {
                for col in 0..w {
                    let bit = 1u64 << (row * STRIDE + col);
                    if mask & bit == 0 || reached & bit != 0 {
                        continue;
                    }
                    let neighbors = [
                        (row > 0).then_some(bit >> STRIDE),
                        (row + 1 < h).then_some(bit << STRIDE),
                        (col > 0).then_some(bit >> 1),
                        (col + 1 < w).then_some(bit << 1),
                    ];
                    if neighbors
                        .into_iter()
                        .flatten()
                        .any(|neighbor| reached & neighbor != 0)
                    {
                        next |= bit;
                    }
                }
            }
            if next == reached {
                return reached == mask;
            }
            reached = next;
        }
    }

    #[test]
    fn test_parent_rotation_growth_exhaustive_4x4_coordinate_oracle() {
        let mut seen = HashSet::new();
        let (mut connected_parents, mut growth_edges) = (0, 0);
        for raw in 1u32..(1 << 16) {
            let mut mask = 0u64;
            for cell in 0..16 {
                if raw & (1 << cell) != 0 {
                    mask |= 1u64 << ((cell / 4) * STRIDE + cell % 4);
                }
            }
            let (mask, w, h) = coordinate_normalize(mask);
            if !seen.insert(mask) || !coordinate_connected(mask, w, h) {
                continue;
            }
            connected_parents += 1;
            let rotations = ParentRotations::new(mask, w, h);
            for r in -1..=h as i32 {
                for c in -1..=w as i32 {
                    let grown = coordinate_grow(mask, w, h, r, c);
                    if grown.1 > 4 || grown.2 > 4 || grown.0.count_ones() == mask.count_ones() {
                        continue;
                    }
                    let mut adjacent = false;
                    for (dr, dc) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let nr = r + dr;
                        let nc = c + dc;
                        if nr >= 0
                            && nr < h as i32
                            && nc >= 0
                            && nc < w as i32
                            && mask & (1u64 << (nr as usize * STRIDE + nc as usize)) != 0
                        {
                            adjacent = true;
                        }
                    }
                    if !adjacent {
                        continue;
                    }

                    let g = Growth::new(w, h, r, c);
                    let mut expected = grown;
                    for turn in 0..4 {
                        let actual = match turn {
                            0 => rotations.child0(g),
                            1 => rotations.child90(g),
                            2 => rotations.child180(g),
                            _ => rotations.child270(g),
                        };
                        assert_eq!(
                            actual, expected.0,
                            "mask={mask:#018x}, grow=({r},{c}), turn={turn}"
                        );
                        expected = coordinate_rotate90(expected.0, expected.1, expected.2);
                    }
                    let mut orbit = grown;
                    let mut best = orbit;
                    for _ in 0..3 {
                        orbit = coordinate_rotate90(orbit.0, orbit.1, orbit.2);
                        if orbit.0 < best.0 {
                            best = orbit;
                        }
                    }
                    assert_eq!(
                        rotations.grow_canonical(w, h, r, c),
                        best,
                        "mask={mask:#018x}, grow=({r},{c})"
                    );
                    growth_edges += 1;
                }
            }
        }
        assert_eq!(connected_parents, 9472);
        assert_eq!(growth_edges, 54213);
    }
}
