//! 核心类型定义 — 多联骨牌房间计数
//!
//! 问题: n×n 网格中 One-sided polyomino 枚举
//! 等价: 统计 bounding box ≤ n×n 的 One-sided polyomino

use std::fmt;

// ================================================================
// 编译期常量
// ================================================================

/// 最大网格尺寸
pub const MAX_N: usize = 6;

/// 最大格子数 (6×6=36)
pub const MAX_CELLS: usize = MAX_N * MAX_N;

/// 位图行步长（2的幂，方便位运算）
pub const STRIDE: usize = 8;
pub const STRIDE_SHIFT: usize = 3; // log2(STRIDE)

/// 归一化后坐标的安全边界（含旋转+padding余量）10×10
pub const GRID_PAD: usize = MAX_N + 4;

// ================================================================
// 位图类型
// ================================================================

/// 64位足够表示最多 8×8=64 个格子
pub type Mask = u64;

// ================================================================
// 多联骨牌结构体
// ================================================================

/// 规范化后的多联骨牌表示
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Poly {
    /// 规范化位图：行优先，stride=STRIDE，左上角对齐 (0,0)
    pub cells: Mask,
    /// 包围矩形宽度
    pub w: usize,
    /// 包围矩形高度
    pub h: usize,
    /// 格子数（popcount）
    pub size: usize,
}

impl Poly {
    /// 创建单格初始形状
    pub fn single_cell() -> Self {
        Self {
            cells: 1,
            w: 1,
            h: 1,
            size: 1,
        }
    }
}

// ================================================================
// 房间计数结果
// ================================================================

/// 单个 n 的枚举结果
#[derive(Debug, Clone)]
pub struct RoomCount {
    /// 网格尺寸 1..MAX_N
    pub n: usize,
    /// 总房间数（One-sided）
    pub total: u64,
    /// 无洞房间数（亏格=0，单连通）
    pub no_hole: u64,
    /// 有洞房间数（亏格≥1）
    pub has_hole: u64,
}

impl fmt::Display for RoomCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "n={}: total={}, no_hole={}, has_hole={}",
            self.n, self.total, self.no_hole, self.has_hole
        )
    }
}
