//! 对称形状计数 — 在 Fixed 枚举中内联检测旋转对称性
//!
//! ## 原理
//!
//! 对每个生成的 Fixed polyomino，检测其是否具有 90° 或 180° 旋转对称性：
//!
//! - **90° 对称**: 形状绕原点旋转 90° 后，经平移归一化与原形状相同
//! - **180° 对称**: 形状绕原点旋转 180° 后，经平移归一化与原形状相同
//!
//! 如果 90° 对称 → 必然 180° 对称（旋转两次 90° = 180°）。
//!
//! ## 计数
//!
//! 对于 Burnside 公式:
//!   One-sided(n) = (Fixed(n) + 2·Sym90(n) + Sym180(n)) / 4
//!
//! Sym90(n): max(w,h) ≤ n 的具有 90° 对称性的 Fixed 形状数
//! Sym180(n): max(w,h) ≤ n 的具有 180° 对称性的 Fixed 形状数

use crate::bit_utils::*;
use crate::types::*;
use std::sync::atomic::{AtomicU64, Ordering};

// ================================================================
// 旋转对称性检测
// ================================================================

/// 检查 Fixed 形状是否具有 180° 旋转对称性
///
/// 将形状绕原点旋转 180°，再平移归一化，比较是否相等。
pub fn has_symmetry_180(cells: Mask, w: usize, h: usize) -> bool {
    cells == cells.reverse_bits() >> (64 - ((h - 1) * STRIDE + w))
}

/// 检查 Fixed 形状是否具有 90° 旋转对称性
///
/// 将形状绕原点旋转 90°，再平移归一化，比较是否相等。
pub fn has_symmetry_90(cells: Mask, w: usize, h: usize) -> bool {
    w == h && rotate90(cells, w, h).0 == cells
}

// ================================================================
// 对称性统计（从 Fixed 枚举结果派生）
// ================================================================

/// 对称性统计器 — 线程安全累加
pub struct SymmetryStats {
    pub total: Vec<AtomicU64>,
    pub no_hole: Vec<AtomicU64>,
    pub has_hole: Vec<AtomicU64>,
}

impl SymmetryStats {
    pub fn new(max_n: usize) -> Self {
        Self {
            total: (0..max_n).map(|_| AtomicU64::new(0)).collect(),
            no_hole: (0..max_n).map(|_| AtomicU64::new(0)).collect(),
            has_hole: (0..max_n).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// 记录一个对称形状
    #[inline]
    pub fn record(&self, w: usize, h: usize, has_hole_flag: bool, max_n: usize) {
        let md = w.max(h);
        for n in md..=max_n {
            let i = n - 1;
            self.total[i].fetch_add(1, Ordering::Relaxed);
            if has_hole_flag {
                self.has_hole[i].fetch_add(1, Ordering::Relaxed);
            } else {
                self.no_hole[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn to_room_counts(&self, max_n: usize) -> Vec<RoomCount> {
        let mut results = Vec::with_capacity(max_n);
        for n in 1..=max_n {
            let i = n - 1;
            results.push(RoomCount {
                n,
                total: self.total[i].load(Ordering::Relaxed),
                no_hole: self.no_hole[i].load(Ordering::Relaxed),
                has_hole: self.has_hole[i].load(Ordering::Relaxed),
            });
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monomino_symmetry() {
        // Single cell always has all symmetries
        let mask: Mask = 1;
        assert!(has_symmetry_180(mask, 1, 1));
        assert!(has_symmetry_90(mask, 1, 1));
    }

    #[test]
    fn test_square_symmetry() {
        // 2×2 square
        let mask: Mask = (1 << 0) | (1 << 1) | (1 << STRIDE) | (1 << (STRIDE + 1));
        // cells at (0,0),(0,1),(1,0),(1,1)
        assert!(has_symmetry_180(mask, 2, 2));
        assert!(has_symmetry_90(mask, 2, 2));
    }

    #[test]
    fn test_domino_symmetry() {
        // Horizontal domino
        let mask: Mask = (1 << 0) | (1 << 1); // cells at (0,0),(0,1)
        assert!(has_symmetry_180(mask, 2, 1));
        assert!(!has_symmetry_90(mask, 2, 1)); // w≠h, no 90° symmetry

        // Vertical domino
        let mask2: Mask = (1 << 0) | (1 << STRIDE); // cells at (0,0),(1,0)
        assert!(has_symmetry_180(mask2, 1, 2));
        assert!(!has_symmetry_90(mask2, 1, 2));
    }

    #[test]
    fn test_l_tromino_no_symmetry() {
        // L-tromino (cells at (0,0),(0,1),(1,0))
        let mask: Mask = (1 << 0) | (1 << 1) | (1 << STRIDE);
        assert!(!has_symmetry_180(mask, 2, 2));
        assert!(!has_symmetry_90(mask, 2, 2));
    }
}
