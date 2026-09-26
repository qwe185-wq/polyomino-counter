//! Burnside 引理组合器 — 将 Fixed + Sym 组合为 One-sided 结果
//!
//! ## 公式
//!
//! One-sided(n) = (Fixed(n) + 2·Sym90(n) + Sym180(n)) / 4
//!
//! ## 验证
//!
//! 已知 OEIS 序列:
//! - A001168: Fixed polyomino (平移不同视为不同)
//! - A000988: One-sided polyomino (允许旋转+平移，禁止翻转)
//! - A001933: One-sided polyomino with holes
//!
//! 对于 bounding box ≤ n×n 的约束:
//! - 枚举 Fixed 时限制 max(w,h) ≤ n
//! - 枚举 Sym90/Sym180 时也限制 max(w,h) ≤ n
//! - 应用 Burnside 时按 size 分别计算

use crate::types::*;

/// 应用 Burnside 引理: 从 Fixed/Sym90/Sym180 组合 One-sided 结果
///
/// # 参数
/// - `fixed`: Fixed polyomino 枚举结果
/// - `sym90`: 90° 旋转对称形状枚举结果
/// - `sym180`: 180° 旋转对称形状枚举结果
///
/// # 返回
/// - One-sided RoomCount 数组
///
/// # 正确性约束
///
/// 结果必须是整数。验证 (fixed + 2*sym90 + sym180) % 4 == 0。
pub fn apply_burnside(
    fixed: &[RoomCount],
    sym90: &[RoomCount],
    sym180: &[RoomCount],
) -> Vec<RoomCount> {
    assert_eq!(fixed.len(), sym90.len());
    assert_eq!(fixed.len(), sym180.len());

    let mut results = Vec::with_capacity(fixed.len());

    for i in 0..fixed.len() {
        let n = fixed[i].n;

        // One-sided = (Fixed + 2·Sym90 + Sym180) / 4
        let total_num = fixed[i].total + 2 * sym90[i].total + sym180[i].total;
        let nohole_num = fixed[i].no_hole + 2 * sym90[i].no_hole + sym180[i].no_hole;
        let hole_num = fixed[i].has_hole + 2 * sym90[i].has_hole + sym180[i].has_hole;

        // 验证整数性
        assert!(
            total_num % 4 == 0,
            "Burnside non-integer: n={}, total={} (fixed={}, sym90={}, sym180={})",
            n,
            total_num,
            fixed[i].total,
            sym90[i].total,
            sym180[i].total
        );

        assert_eq!(nohole_num % 4, 0, "Burnside no-hole n={n}");
        assert_eq!(hole_num % 4, 0, "Burnside has-hole n={n}");
        assert_eq!(total_num, nohole_num + hole_num, "分类之和 n={n}");

        results.push(RoomCount {
            n,
            total: total_num / 4,
            no_hole: nohole_num / 4,
            has_hole: hole_num / 4,
        });
    }

    results
}

/// 验证 Burnside 结果与已知值一致
pub fn verify_results(results: &[RoomCount]) -> bool {
    // 已知值: One-sided polyomino with bounding box ≤ n×n
    let known: &[(usize, u64, u64, u64)] = &[
        // (n, total, no_hole, has_hole)
        (1, 1, 1, 0),
        (2, 4, 4, 0),
        (3, 46, 44, 2),
        (4, 2404, 1899, 505),
        (5, 520818, 267976, 252842),
        (6, 410964612, 112877832, 298086780),
    ];

    for &(n, total, no_hole, has_hole) in known {
        if n > results.len() {
            break;
        }
        let r = &results[n - 1];
        if r.total != total || r.no_hole != no_hole || r.has_hole != has_hole {
            eprintln!(
                "  [验证失败] n={}: 期望 total={} no_hole={} has_hole={}, \
                 实际 total={} no_hole={} has_hole={}",
                n, total, no_hole, has_hole, r.total, r.no_hole, r.has_hole
            );
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_count_guard_covers_six_without_enumerating() {
        let known = [
            (1, 1, 0),
            (4, 4, 0),
            (46, 44, 2),
            (2404, 1899, 505),
            (520818, 267976, 252842),
            (410964612, 112877832, 298086780),
        ];
        let mut counts: Vec<_> = known
            .into_iter()
            .enumerate()
            .map(|(i, (total, no_hole, has_hole))| RoomCount {
                n: i + 1,
                total,
                no_hole,
                has_hole,
            })
            .collect();
        assert!(verify_results(&counts));
        counts[5].has_hole -= 1;
        assert!(!verify_results(&counts));
    }

    #[test]
    fn test_burnside_formula_integer() {
        // 构造简单测试数据确保 Burnside 公式产生整数
        let fixed = vec![
            RoomCount {
                n: 1,
                total: 1,
                no_hole: 1,
                has_hole: 0,
            },
            RoomCount {
                n: 2,
                total: 10,
                no_hole: 10,
                has_hole: 0,
            },
        ];
        let sym90 = vec![
            RoomCount {
                n: 1,
                total: 1,
                no_hole: 1,
                has_hole: 0,
            },
            RoomCount {
                n: 2,
                total: 2,
                no_hole: 2,
                has_hole: 0,
            },
        ];
        let sym180 = vec![
            RoomCount {
                n: 1,
                total: 1,
                no_hole: 1,
                has_hole: 0,
            },
            RoomCount {
                n: 2,
                total: 2,
                no_hole: 2,
                has_hole: 0,
            },
        ];

        let results = apply_burnside(&fixed, &sym90, &sym180);

        // n=1: (1 + 2*1 + 1) / 4 = 1 ✓
        assert_eq!(results[0].total, 1);

        // n=2: (10 + 2*2 + 2) / 4 = 16/4 = 4
        assert_eq!(results[1].total, 4);
    }
}
