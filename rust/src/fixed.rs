//! 按面积 BFS 枚举 Fixed 形状；局部统计归并，流式导出旋转最小代表。
use crate::bit_utils::*;
use crate::export::ExportManager;
use crate::hashset::ShardedHashSet;
use crate::symmetric::{has_symmetry_180, has_symmetry_90};
use crate::types::*;
use rayon::prelude::*;
use std::io;
use std::sync::{mpsc::sync_channel, Arc};

type Bins = [[u64; 2]; MAX_N];
pub type FixedCounts = (Vec<RoomCount>, Vec<RoomCount>, Vec<RoomCount>);
const TASK_SIZE: usize = 512;
const EXPORT_BATCH: usize = 8192;

#[derive(Default)]
struct Local {
    masks: Vec<Mask>,
    fixed: Bins,
    sym90: Bins,
    sym180: Bins,
}

fn counts(bins: &Bins, max_n: usize) -> Vec<RoomCount> {
    let (mut no_hole, mut has_hole) = (0, 0);
    (0..max_n)
        .map(|i| {
            no_hole += bins[i][0];
            has_hole += bins[i][1];
            RoomCount {
                n: i + 1,
                total: no_hole + has_hole,
                no_hole,
                has_hole,
            }
        })
        .collect()
}

/// 返回 Fixed、90°、180° 累计计数。写盘失败必须交给调用方，不返回成功。
pub fn enumerate_fixed_with_symmetry(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
) -> io::Result<FixedCounts> {
    enumerate(max_n, verbose, export, false)
}

/// 直接枚举旋转等价类；用轨道大小还原 Fixed 与旋转不变计数。
pub fn enumerate_canonical(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
) -> io::Result<FixedCounts> {
    enumerate(max_n, verbose, export, true)
}

fn enumerate(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
    one_sided: bool,
) -> io::Result<FixedCounts> {
    assert!((1..=MAX_N).contains(&max_n));
    let mut fixed = [[0; 2]; MAX_N];
    let mut sym90 = fixed;
    let mut sym180 = fixed;
    fixed[0][0] = 1;
    sym90[0][0] = 1;
    sym180[0][0] = 1;
    let mut current = vec![vec![1u64]];
    let mut n_shapes = 1usize;

    // 含 padding 的 8×8 位板，宽/高达到上限时裁掉外扩前沿。
    let mut allowed = [[0u64; MAX_N + 1]; MAX_N + 1];
    for (w, widths) in allowed.iter_mut().enumerate().take(max_n + 1).skip(1) {
        for (h, mask) in widths.iter_mut().enumerate().take(max_n + 1).skip(1) {
            let c0 = usize::from(w == max_n);
            let c1 = if w == max_n { w } else { w + 1 };
            let r0 = usize::from(h == max_n);
            let r1 = if h == max_n { h } else { h + 1 };
            let row = ((1u64 << (c1 - c0 + 1)) - 1) << c0;
            for r in r0..=r1 {
                *mask |= row << (r * 8);
            }
        }
    }

    let (sender, writer) = if let Some(exp) = export {
        exp.write_batch(&[(1, 1, false)])?;
        let (tx, rx) = sync_channel::<Vec<(Mask, usize, bool)>>(8);
        let handle = std::thread::spawn(move || -> io::Result<()> {
            for batch in rx {
                exp.write_batch(&batch)?;
            }
            exp.flush_all()
        });
        (Some(tx), Some(handle))
    } else {
        (None, None)
    };

    let enumeration = (|| -> io::Result<()> {
        for size in 1..max_n * max_n {
            let start = std::time::Instant::now();
            let dedup = ShardedHashSet::new((n_shapes * 2).max(16));
            let locals: io::Result<Vec<Local>> = current
                .par_iter()
                .flat_map(|chunk| chunk.par_chunks(TASK_SIZE))
                .map(|parents| -> io::Result<Local> {
                    let mut local = Local {
                        masks: Vec::with_capacity(parents.len() * 2),
                        ..Local::default()
                    };
                    let mut output = Vec::new();
                    for &pmask in parents {
                        let (pw, ph) = mask_extent(pmask);
                        let parent_rotations =
                            one_sided.then(|| ParentRotations::new(pmask, pw, ph));
                        let padded = pmask << 9;
                        let mut frontier =
                            ((padded << 8) | (padded >> 8) | (padded << 1) | (padded >> 1))
                                & !padded
                                & allowed[pw][ph];
                        while frontier != 0 {
                            let bit = frontier.trailing_zeros();
                            frontier &= frontier - 1;
                            let r = (bit / 8) as i32 - 1;
                            let c = (bit % 8) as i32 - 1;
                            let (m, w, h) = if let Some(rotations) = parent_rotations {
                                rotations.grow_canonical(pw, ph, r, c)
                            } else {
                                grow_mask(pmask, pw, ph, r, c)
                            };
                            if !dedup.check_and_insert(m) {
                                continue;
                            }
                            local.masks.push(m);
                            let hole = poly_has_hole(m, w, h);
                            let md = w.max(h);
                            let s180 = has_symmetry_180(m, w, h);
                            let s90 = s180 && has_symmetry_90(m, w, h);
                            let weight = if !one_sided || s90 {
                                1
                            } else if s180 {
                                2
                            } else {
                                4
                            };
                            local.fixed[md - 1][usize::from(hole)] += weight;
                            if s180 {
                                local.sym180[md - 1][usize::from(hole)] += weight;
                            }
                            if s90 {
                                local.sym90[md - 1][usize::from(hole)] += weight;
                            }
                            if sender.is_some() && (one_sided || compute_canonical(m, w, h).0 == m)
                            {
                                output.push((m, md, hole));
                                if output.len() == EXPORT_BATCH {
                                    sender
                                        .as_ref()
                                        .unwrap()
                                        .send(std::mem::take(&mut output))
                                        .map_err(|_| io::Error::other("导出写线程已退出"))?;
                                }
                            }
                        }
                    }
                    if !output.is_empty() {
                        sender
                            .as_ref()
                            .unwrap()
                            .send(output)
                            .map_err(|_| io::Error::other("导出写线程已退出"))?;
                    }
                    Ok(local)
                })
                .collect();
            let locals = locals?;
            // 结果块直接成为下一代，无树形 Vec 复制。
            drop(dedup);
            current.clear();
            let previous_count = n_shapes;
            n_shapes = 0;
            for local in locals {
                for i in 0..max_n {
                    for hole in 0..2 {
                        fixed[i][hole] += local.fixed[i][hole];
                        sym90[i][hole] += local.sym90[i][hole];
                        sym180[i][hole] += local.sym180[i][hole];
                    }
                }
                n_shapes += local.masks.len();
                if !local.masks.is_empty() {
                    current.push(local.masks);
                }
            }
            if verbose {
                eprintln!(
                    "  [BFS] 面积={} 父代={} 新代={} {:.3}s",
                    size + 1,
                    previous_count,
                    n_shapes,
                    start.elapsed().as_secs_f64()
                );
            }
            if n_shapes == 0 {
                break;
            }
        }
        Ok(())
    })();
    drop(sender);
    if let Some(handle) = writer {
        handle
            .join()
            .map_err(|_| io::Error::other("导出写线程异常退出"))??;
    }
    enumeration?;
    Ok((
        counts(&fixed, max_n),
        counts(&sym90, max_n),
        counts(&sym180, max_n),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_small_symmetry_counts() {
        let (f, s90, s180) = enumerate_fixed_with_symmetry(2, false, None).unwrap();
        assert_eq!((f[0].total, s90[0].total, s180[0].total), (1, 1, 1));
        assert_eq!((f[1].total, s90[1].total, s180[1].total), (8, 2, 4));
    }
}
