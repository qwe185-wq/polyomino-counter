//! 真正的 Redelmeier 固定多联骨牌遍历，逐个导出旋转最小代表。
//!
//! 根是最高一行最左占格；根右侧及更低行可用，更低行允许负列。
//! 每层的待试格和禁用历史按值保存，子调用不会改写父层的回溯状态。

use crate::bit_utils::{compute_canonical, poly_has_hole};
use crate::export::ExportManager;
use crate::types::{Mask, RoomCount, MAX_N, STRIDE};
use std::io;
use std::sync::Arc;
use std::time::Instant;

const EXPORT_BATCH: usize = 8192;
type Bins = [[u64; 2]; MAX_N];

#[derive(Clone, Copy)]
struct Shape {
    occupied: u128,
    min_col: i8,
    max_col: i8,
    max_row: u8,
    size: u8,
}

struct Walker {
    limit: usize,
    pitch: usize,
    offset: i8,
    bins: Bins,
    fixed_visited: u64,
    batch: Vec<(Mask, usize, bool)>,
    export: Option<Arc<ExportManager>>,
}

impl Walker {
    #[inline]
    fn position(&self, index: usize) -> (usize, i8) {
        (index / self.pitch, (index % self.pitch) as i8 - self.offset)
    }

    #[inline]
    fn bit_at(&self, row: usize, col: i8) -> u128 {
        1u128 << (row * self.pitch + (col + self.offset) as usize)
    }

    #[inline]
    fn fits(&self, shape: Shape, row: usize, col: i8) -> bool {
        row < self.limit
            && shape.max_row.max(row as u8) as usize + 1 <= self.limit
            && (shape.max_col.max(col) - shape.min_col.min(col)) as usize + 1 <= self.limit
    }

    #[inline]
    fn neighbors(&self, row: usize, col: i8) -> u128 {
        let mut result = 0;
        if row > 0 && (row > 1 || col >= 0) {
            result |= self.bit_at(row - 1, col);
        }
        if row + 1 < self.limit {
            result |= self.bit_at(row + 1, col);
        }
        if col > -self.offset && (row > 0 || col > 0) {
            result |= self.bit_at(row, col - 1);
        }
        if col < self.offset {
            result |= self.bit_at(row, col + 1);
        }
        result
    }

    #[inline]
    fn normalized(&self, shape: Shape) -> Mask {
        let mut occupied = shape.occupied;
        let mut result = 0u64;
        while occupied != 0 {
            let index = occupied.trailing_zeros() as usize;
            occupied &= occupied - 1;
            let (row, col) = self.position(index);
            let column = (col - shape.min_col) as usize;
            result |= 1u64 << (row * STRIDE + column);
        }
        result
    }

    fn record(&mut self, shape: Shape) -> io::Result<()> {
        self.fixed_visited += 1;
        let width = (shape.max_col - shape.min_col + 1) as usize;
        let height = shape.max_row as usize + 1;
        let mask = self.normalized(shape);
        if compute_canonical(mask, width, height).0 != mask {
            return Ok(());
        }
        let hole = poly_has_hole(mask, width, height);
        let md = width.max(height);
        self.bins[md - 1][usize::from(hole)] += 1;
        if self.export.is_some() {
            self.batch.push((mask, md, hole));
            if self.batch.len() == EXPORT_BATCH {
                self.flush_batch()?;
            }
        }
        Ok(())
    }

    fn flush_batch(&mut self) -> io::Result<()> {
        if !self.batch.is_empty() {
            if let Some(writer) = &self.export {
                writer.write_batch(&self.batch)?;
            }
            self.batch.clear();
        }
        Ok(())
    }

    /// `forbidden` 是本层以前弹出的候选，以及祖先层留下的禁用历史。
    /// 弹出的格子仅在同层后续兄弟分支禁用；子层按值继承并独立扩展。
    fn search(&mut self, shape: Shape, mut untried: u128, mut forbidden: u128) -> io::Result<()> {
        while untried != 0 {
            let index = untried.trailing_zeros() as usize;
            let bit = 1u128 << index;
            untried &= !bit;
            let (row, col) = self.position(index);
            if !self.fits(shape, row, col) {
                forbidden |= bit;
                continue;
            }
            let child = Shape {
                occupied: shape.occupied | bit,
                min_col: shape.min_col.min(col),
                max_col: shape.max_col.max(col),
                max_row: shape.max_row.max(row as u8),
                size: shape.size + 1,
            };
            self.record(child)?;
            if (child.size as usize) < self.limit * self.limit {
                let exposed = self.neighbors(row, col) & !(child.occupied | untried | forbidden);
                self.search(child, untried | exposed, forbidden)?;
            }
            forbidden |= bit;
        }
        Ok(())
    }

    fn counts(&self) -> Vec<RoomCount> {
        let (mut no_hole, mut has_hole) = (0, 0);
        (0..self.limit)
            .map(|i| {
                no_hole += self.bins[i][0];
                has_hole += self.bins[i][1];
                RoomCount {
                    n: i + 1,
                    total: no_hole + has_hole,
                    no_hole,
                    has_hole,
                }
            })
            .collect()
    }
}

/// 单次遍历所有面积；内存仅含递归栈、固定宽度位图与有限导出批次。
pub fn enumerate_redelmeier(
    max_n: usize,
    verbose: bool,
    export: Option<Arc<ExportManager>>,
) -> io::Result<Vec<RoomCount>> {
    assert!((1..=MAX_N).contains(&max_n));
    let started = Instant::now();
    let pitch = max_n * 2 - 1;
    let offset = max_n as i8 - 1;
    let root = Shape {
        occupied: 1u128 << offset,
        min_col: 0,
        max_col: 0,
        max_row: 0,
        size: 1,
    };
    let mut walker = Walker {
        limit: max_n,
        pitch,
        offset,
        bins: [[0; 2]; MAX_N],
        fixed_visited: 0,
        batch: Vec::with_capacity(if export.is_some() { EXPORT_BATCH } else { 0 }),
        export,
    };
    walker.record(root)?;
    if max_n > 1 {
        let untried = walker.neighbors(0, 0);
        walker.search(root, untried, 0)?;
    }
    walker.flush_batch()?;
    if let Some(writer) = &walker.export {
        writer.flush_all()?;
    }
    if verbose {
        eprintln!(
            "  [Redelmeier] fixed={} one-sided={} {:.3}s",
            walker.fixed_visited,
            walker.counts().last().unwrap().total,
            started.elapsed().as_secs_f64()
        );
    }
    Ok(walker.counts())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    fn masks(dir: &Path) -> Vec<Mask> {
        let mut result = Vec::new();
        if dir.exists() {
            for entry in fs::read_dir(dir).unwrap() {
                let bytes = fs::read(entry.unwrap().path()).unwrap();
                assert_eq!(bytes.len() % 8, 0);
                result.extend(
                    bytes
                        .chunks_exact(8)
                        .map(|b| Mask::from_le_bytes(b.try_into().unwrap())),
                );
            }
        }
        result
    }

    #[test]
    fn known_counts_through_four() {
        let counts = enumerate_redelmeier(4, false, None).unwrap();
        let expected = [(1, 1, 0), (4, 4, 0), (46, 44, 2), (2404, 1899, 505)];
        for (actual, expected) in counts.iter().zip(expected) {
            assert_eq!((actual.total, actual.no_hole, actual.has_hole), expected);
        }
    }

    #[test]
    fn exported_four_by_four_matches_canonical_bfs_set() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "room-count-redelmeier-{}-{unique}",
            std::process::id()
        ));
        let red_dir = dir.join("redelmeier");
        let bfs_dir = dir.join("bfs");
        let red = Arc::new(ExportManager::new(&red_dir).unwrap());
        let bfs = Arc::new(ExportManager::new(&bfs_dir).unwrap());
        enumerate_redelmeier(4, false, Some(red.clone())).unwrap();
        crate::fixed::enumerate_canonical(4, false, Some(bfs.clone())).unwrap();
        drop((red, bfs));
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        for category in ["no_holes", "with_holes"] {
            for md in 1..=4 {
                let slot = format!("n{md:02}_fixed");
                let red_masks = masks(&red_dir.join(category).join(&slot));
                let bfs_masks = masks(&bfs_dir.join(category).join(&slot));
                assert_eq!(
                    red_masks.iter().copied().collect::<HashSet<_>>(),
                    bfs_masks.iter().copied().collect(),
                    "类别={category} 包围盒={md}"
                );
                actual.extend(red_masks);
                expected.extend(bfs_masks);
            }
        }
        assert_eq!(actual.len(), 2404);
        assert_eq!(
            actual.iter().copied().collect::<HashSet<_>>().len(),
            actual.len()
        );
        let cross = (1u64 << 1) | (1u64 << 8) | (1u64 << 9) | (1u64 << 10) | (1u64 << 17);
        let ring = (1u64 << 0)
            | (1u64 << 1)
            | (1u64 << 2)
            | (1u64 << 8)
            | (1u64 << 10)
            | (1u64 << 16)
            | (1u64 << 17)
            | (1u64 << 18);
        let square = (1u64 << 0) | (1u64 << 1) | (1u64 << 8) | (1u64 << 9);
        assert!(actual.contains(&cross), "最高行无角格的十字形必须生成");
        assert!(actual.contains(&ring), "单格洞必须生成");
        assert!(actual.contains(&square), "旋转对称形状必须只写一次");
        assert_eq!(
            actual.into_iter().collect::<HashSet<_>>(),
            expected.into_iter().collect()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn root_allows_lower_negative_columns() {
        let walker = Walker {
            limit: 4,
            pitch: 7,
            offset: 3,
            bins: [[0; 2]; MAX_N],
            fixed_visited: 0,
            batch: Vec::new(),
            export: None,
        };
        let below = walker.bit_at(1, 0);
        assert_ne!(walker.neighbors(1, 0) & walker.bit_at(1, -1), 0);
        assert_eq!(walker.neighbors(0, 0) & walker.bit_at(0, -1), 0);
        assert_ne!(walker.neighbors(0, 0) & below, 0);
    }
}
