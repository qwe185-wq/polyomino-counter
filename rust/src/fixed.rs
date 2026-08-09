//! Fixed Polyomino 枚举引擎（Redelmeier 1981 生长法）+ 内联对称检测
//!
//! ## 核心优化
//!
//! 1. **仅翻译归一化** — Burnside 引理将 One-sided 问题转为 Fixed，消除 4× 旋转计算
//! 2. **分片并发哈希集** — 每代内去重，4096 分片消除临界区争用
//! 3. **前沿位掩码** — u16 位掩码替代 bool 数组 memset，减少内存带宽
//! 4. **rayon 并行** — work-stealing 调度替代 OpenMP dynamic
//! 5. **内联对称检测** — 发现新 Fixed 形状时立即检测 90°/180° 对称性
//!
//! ## 返回值
//!
//! (fixed_results, sym90_results, sym180_results)
//! Burnside: One-sided = (Fixed + 2·Sym90 + Sym180) / 4

use crate::bit_utils::*;
use crate::export::ExportManager;
use crate::hashset::ShardedHashSet;
use crate::symmetric::{has_symmetry_180, has_symmetry_90, SymmetryStats};
use crate::types::*;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;

// ================================================================
// Fixed Polyomino 枚举（含内联对称检测）
// ================================================================

/// 运行 Fixed polyomino 枚举 + 对称性检测
///
/// 返回 (fixed, sym90, sym180) 三组 RoomCount。
/// 如果 `export` 为 Some，则在枚举过程中将 Fixed mask 写入磁盘。
pub fn enumerate_fixed_with_symmetry(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
) -> (Vec<RoomCount>, Vec<RoomCount>, Vec<RoomCount>) {
    assert!(max_n <= MAX_N, "max_n must be <= {}", MAX_N);

    let max_cells = max_n * max_n;

    // Fixed 统计原子变量
    let total_stats: Arc<Vec<AtomicU64>> =
        Arc::new((0..max_n).map(|_| AtomicU64::new(1)).collect());
    let nohole_stats: Arc<Vec<AtomicU64>> =
        Arc::new((0..max_n).map(|_| AtomicU64::new(1)).collect());
    let hole_stats: Arc<Vec<AtomicU64>> =
        Arc::new((0..max_n).map(|_| AtomicU64::new(0)).collect());

    // Sym90 统计
    let sym90_stats = Arc::new(SymmetryStats::new(max_n));
    // Sym180 统计
    let sym180_stats = Arc::new(SymmetryStats::new(max_n));

    // size=1: 单格有所有对称性
    {
        sym90_stats.record(1, 1, false, max_n);
        sym180_stats.record(1, 1, false, max_n);
    }

    let mut cur_masks: Vec<Mask> = vec![1u64]; // single cell at (0,0)

    // 导出种子形状 (size=1, monomino = Fixed = One-sided)
    if let Some(ref exp) = export {
        let seed_data: Vec<(Mask, usize, bool)> = cur_masks
            .iter()
            .map(|&mask| {
                let (w, h) = mask_extent(mask);
                (mask, w.max(h), false)
            })
            .collect();
        if let Err(e) = exp.write_batch(&seed_data) {
            eprintln!("  [导出] 种子写入错误: {}", e);
        }
    }

    for size in 1..max_cells {
        let gen_start = std::time::Instant::now();
        let n_shapes = cur_masks.len();

        if n_shapes == 0 {
            break;
        }

        // 预分配哈希集: 每形状约生成 2-3 个候选，唯一率 ~80-90%
        // 用 n_shapes * 2 作为容量估算（实测≈1.19×，留余量）
        let dedup_cap = (n_shapes * 2).max(1024);
        let dedup = Arc::new(ShardedHashSet::new(dedup_cap));

        let export_enabled = export.is_some();

        // One-sided 去重: Fixed ≈ 4× One-sided，容量取 Fixed 的 1/3
        let os_dedup = if export_enabled {
            Some(Arc::new(ShardedHashSet::new((dedup_cap / 3).max(1024))))
        } else {
            None
        };

        // 每线程 fold 缓冲初始容量: 本代平均产出 = n_shapes/size*3 / n_threads
        let n_threads = rayon::current_num_threads();
        let fold_cap = (n_shapes / n_threads / 4).max(1024);

        // 流式导出通道：避免攒大量数据到内存
        let (exp_tx, exp_rx) = if export_enabled {
            let (tx, rx) = sync_channel::<Vec<(Mask, usize, bool)>>(64);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // 后台写线程
        let writer_handle = if let (Some(ref exp), Some(rx)) = (&export, exp_rx) {
            let exp = Arc::clone(exp);
            Some(std::thread::spawn(move || {
                for batch in rx {
                    if let Err(e) = exp.write_batch(&batch) {
                        eprintln!("  [导出] 写入错误: {}", e);
                    }
                }
            }))
        } else {
            None
        };

        let (next_masks, mut export_remnants): (Vec<Mask>, Vec<(Mask, usize, bool)>) = cur_masks
            .par_iter()
            .with_min_len(if n_shapes < 500 { n_shapes } else { 16 })
            .fold(
                || {
                    (
                        Vec::<Mask>::with_capacity(fold_cap),
                        if export_enabled {
                            Vec::with_capacity(fold_cap.min(100_000))
                        } else {
                            Vec::new()
                        },
                    )
                },
                |(mut local_new, mut local_exp), &pmask| {
                    // 提取坐标
                    let mut cells_r = [0usize; MAX_CELLS];
                    let mut cells_c = [0usize; MAX_CELLS];
                    let mut cc = 0usize;
                    {
                        let mut m = pmask;
                        while m != 0 {
                            let bit = m.trailing_zeros() as usize;
                            cells_r[cc] = bit >> STRIDE_SHIFT;
                            cells_c[cc] = bit & (STRIDE - 1);
                            cc += 1;
                            m &= m - 1;
                        }
                    }

                    let (pw, ph) = mask_extent(pmask);
                    let box_at_max = pw == max_n && ph == max_n;

                    let frontier = compute_frontier_inline(
                        box_at_max, max_n, &cells_r, &cells_c, cc, pw, ph,
                    );

                    for (nr_i, nc_i) in frontier {
                        let (new_mask, _raw_w, _raw_h) =
                            grow_mask_inline(pmask, pw, ph, nr_i, nc_i);
                        let (canonical, can_w, can_h) = normalize_translation(new_mask);

                        if !dedup.check_and_insert(canonical) {
                            continue;
                        }
                        local_new.push(canonical);

                        let has_hole = poly_has_hole(canonical, can_w, can_h);
                        let md = can_w.max(can_h);

                        // One-sided 去重 + 导出
                        if let Some(ref os_hs) = os_dedup {
                            let (os_canon, os_w, os_h) =
                                compute_canonical(canonical, can_w, can_h);
                            if os_hs.check_and_insert(os_canon) {
                                // 新 One-sided: 用 One-sided canonical 的信息导出
                                let os_has_hole = poly_has_hole(os_canon, os_w, os_h);
                                let os_md = os_w.max(os_h);
                                local_exp.push((os_canon, os_md, os_has_hole));
                                if local_exp.len() >= 100_000 {
                                    if let Some(ref tx) = exp_tx {
                                        let batch = std::mem::replace(
                                            &mut local_exp,
                                            Vec::with_capacity(100_000),
                                        );
                                        let _ = tx.send(batch);
                                    }
                                }
                            }
                        }
                        for n in md..=max_n {
                            let i = n - 1;
                            total_stats[i].fetch_add(1, Ordering::Relaxed);
                            if has_hole {
                                hole_stats[i].fetch_add(1, Ordering::Relaxed);
                            } else {
                                nohole_stats[i].fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        if has_symmetry_90(canonical, can_w, can_h) {
                            sym90_stats.record(can_w, can_h, has_hole, max_n);
                        }
                        if has_symmetry_180(canonical, can_w, can_h) {
                            sym180_stats.record(can_w, can_h, has_hole, max_n);
                        }
                    }

                    (local_new, local_exp)
                },
            )
            .reduce(
                || (Vec::new(), Vec::new()),
                |(mut a_m, mut a_e), (b_m, b_e)| {
                    a_m.extend(b_m);
                    a_e.extend(b_e);
                    (a_m, a_e)
                },
            );

        let next_count = next_masks.len();
        let gen_elapsed = gen_start.elapsed();

        // 流式导出：发送剩余批次 + 等待写线程
        if let (Some(tx), Some(handle)) = (exp_tx, writer_handle) {
            let write_start = std::time::Instant::now();
            // 发送各线程残余的 export 数据
            if !export_remnants.is_empty() {
                let batch = std::mem::replace(
                    &mut export_remnants,
                    Vec::new(),
                );
                let _ = tx.send(batch);
            }
            drop(tx); // 关闭通道
            let _ = handle.join(); // 等待写线程完成
            if verbose {
                eprintln!(
                    "  [导出] 格={:2} 写出={} 耗时={:.3}s",
                    size,
                    next_count,
                    write_start.elapsed().as_secs_f64(),
                );
            }
        }

        if verbose {
            eprintln!(
                "  [Fixed] 格={:2} 本代={:>8} 新代={:>8} 耗时={:.3}s 速率={:.0}K/s",
                size,
                n_shapes,
                next_count,
                gen_elapsed.as_secs_f64(),
                if gen_elapsed.as_secs_f64() > 0.0 {
                    n_shapes as f64 / gen_elapsed.as_secs_f64() / 1000.0
                } else {
                    0.0
                }
            );
        }

        cur_masks = next_masks;
        if cur_masks.is_empty() {
            break;
        }
    }

    // 汇总 Fixed 结果
    let mut fixed = Vec::with_capacity(max_n);
    for n in 1..=max_n {
        let i = n - 1;
        fixed.push(RoomCount {
            n,
            total: total_stats[i].load(Ordering::Relaxed),
            no_hole: nohole_stats[i].load(Ordering::Relaxed),
            has_hole: hole_stats[i].load(Ordering::Relaxed),
        });
    }

    let sym90 = sym90_stats.to_room_counts(max_n);
    let sym180 = sym180_stats.to_room_counts(max_n);

    (fixed, sym90, sym180)
}

// ================================================================
// 兼容旧接口
// ================================================================

/// 仅 Fixed 枚举（向后兼容）
pub fn enumerate_fixed(max_n: usize, verbose: bool) -> Vec<RoomCount> {
    let (fixed, _, _) = enumerate_fixed_with_symmetry(max_n, verbose, None);
    fixed
}

// ================================================================
// 内联前沿计算
// ================================================================

#[inline(always)]
fn compute_frontier_inline(
    box_at_max: bool,
    max_n: usize,
    cells_r: &[usize],
    cells_c: &[usize],
    cc: usize,
    pw: usize,
    ph: usize,
) -> smallvec::SmallVec<[(i32, i32); 256]> {
    let mut occ_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];
    for i in 0..cc {
        occ_rows[cells_r[i] + 1] |= 1u16 << (cells_c[i] + 1);
    }

    let mut inf_rows: [u16; GRID_PAD] = [0u16; GRID_PAD];
    let mut frontier = smallvec::SmallVec::new();

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

            if (occ_rows[nri] >> nci) & 1 != 0 {
                continue;
            }
            if (inf_rows[nri] >> nci) & 1 != 0 {
                continue;
            }

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
            frontier.push((nr, nc));
        }
    }

    frontier
}

/// 内联 GROW 操作
#[inline(always)]
fn grow_mask_inline(
    pmask: Mask,
    pw: usize,
    ph: usize,
    nr: i32,
    nc: i32,
) -> (Mask, usize, usize) {
    let sr = if nr < 0 { 1usize } else { 0usize };
    let sc = if nc < 0 { 1usize } else { 0usize };

    let mut new_mask: Mask = 0;
    for r in 0..ph {
        let row = (pmask >> (r * STRIDE)) & ((1u64 << pw) - 1);
        new_mask |= (row << sc) << ((r + sr) * STRIDE);
    }
    let nr_abs = (nr + sr as i32) as usize;
    let nc_abs = (nc + sc as i32) as usize;
    new_mask |= 1u64 << (nr_abs * STRIDE + nc_abs);

    let raw_w = if nc < 0 {
        pw + 1
    } else if nc as usize >= pw {
        nc as usize + 1
    } else {
        pw
    };
    let raw_h = if nr < 0 {
        ph + 1
    } else if nr as usize >= ph {
        nr as usize + 1
    } else {
        ph
    };

    (new_mask, raw_w, raw_h)
}

// ================================================================
// 测试
// ================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_n1() {
        let (fixed, sym90, sym180) = enumerate_fixed_with_symmetry(1, false, None);
        assert_eq!(fixed[0].total, 1);
        // Monomino has both symmetries
        assert_eq!(sym90[0].total, 1);
        assert_eq!(sym180[0].total, 1);
        // Burnside: (1 + 2*1 + 1) / 4 = 1 ✓
    }

    #[test]
    fn test_fixed_n2_burnside() {
        let (fixed, sym90, sym180) = enumerate_fixed_with_symmetry(2, false, None);

        // n=2 Fixed should be 8
        assert_eq!(fixed[1].total, 8, "Fixed n=2 should be 8");

        // Sym90: monomino + 2×2 square = 2
        assert_eq!(sym90[1].total, 2, "Sym90 n=2 should be 2");

        // Sym180: monomino + 2 dominoes + square = 4
        assert_eq!(sym180[1].total, 4, "Sym180 n=2 should be 4");

        // Burnside: (8 + 2*2 + 4) / 4 = 16/4 = 4
        let sum = fixed[1].total + 2 * sym90[1].total + sym180[1].total;
        assert_eq!(sum % 4, 0, "Burnside sum must be divisible by 4");
        assert_eq!(sum / 4, 4, "One-sided n=2 should be 4");
    }
}
