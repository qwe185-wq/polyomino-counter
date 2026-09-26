//! 任意尺寸网格的多字位图。每行末尾和末行均为空，使横向移位不会跨行相接，
//! 并容纳最末格向右、向下扩展后的边与顶点。

use std::io::{Error, ErrorKind, Result};

fn overflow(what: &str) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("{what} 超出 usize 可表示范围"),
    )
}

fn zeroed_words(len: usize) -> Result<Vec<u64>> {
    let mut words = Vec::new();
    words
        .try_reserve_exact(len)
        .map_err(|error| Error::other(error.to_string()))?;
    words.resize(len, 0);
    Ok(words)
}

#[inline]
fn shifted_left(words: &[u64], index: usize, shift: usize) -> u64 {
    let whole = shift / 64;
    let partial = shift % 64;
    if index < whole {
        return 0;
    }
    let source = index - whole;
    let mut value = words[source] << partial;
    if partial != 0 && source != 0 {
        value |= words[source - 1] >> (64 - partial);
    }
    value
}

#[inline]
fn shifted_right(words: &[u64], index: usize, shift: usize) -> u64 {
    let whole = shift / 64;
    let partial = shift % 64;
    let Some(source) = index.checked_add(whole) else {
        return 0;
    };
    let Some(&base) = words.get(source) else {
        return 0;
    };
    let mut value = base >> partial;
    if partial != 0 {
        if let Some(&carry) = source.checked_add(1).and_then(|i| words.get(i)) {
            value |= carry << (64 - partial);
        }
    }
    value
}

/// Gray 码枚举中可反复翻转旋转轨道的位图。`is_connected` 复用内部缓冲区。
pub(crate) struct DynamicGrid {
    width: usize,
    height: usize,
    stride: usize,
    mask: Vec<u64>,
    seen: Vec<u64>,
    frontier: Vec<u64>,
    next: Vec<u64>,
    occupied: usize,
    top_count: usize,
    left_count: usize,
}

impl DynamicGrid {
    pub(crate) fn new(width: usize, height: usize) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "网格宽高必须为正整数"));
        }
        let stride = width.checked_add(1).ok_or_else(|| overflow("网格行距"))?;
        let rows = height.checked_add(1).ok_or_else(|| overflow("网格行数"))?;
        let bits = stride
            .checked_mul(rows)
            .ok_or_else(|| overflow("位图尺寸"))?;
        width
            .checked_mul(height)
            .ok_or_else(|| overflow("网格面积"))?;
        let words = bits / 64 + usize::from(bits % 64 != 0);
        words
            .checked_mul(std::mem::size_of::<u64>())
            .ok_or_else(|| overflow("位图字节数"))?;
        Ok(Self {
            width,
            height,
            stride,
            mask: zeroed_words(words)?,
            seen: zeroed_words(words)?,
            frontier: zeroed_words(words)?,
            next: zeroed_words(words)?,
            occupied: 0,
            top_count: 0,
            left_count: 0,
        })
    }

    /// `cells` 是本次旋转轨道中互不重复的紧凑行主序索引。
    pub(crate) fn toggle_cells(&mut self, cells: &[usize]) {
        for &cell in cells {
            let row = cell / self.width;
            let col = cell % self.width;
            assert!(row < self.height, "轨道格点超出网格");
            let bit = row * self.stride + col;
            let word = &mut self.mask[bit / 64];
            let flag = 1u64 << (bit % 64);
            let was_set = *word & flag != 0;
            *word ^= flag;
            if was_set {
                self.occupied -= 1;
                if row == 0 {
                    self.top_count -= 1;
                }
                if col == 0 {
                    self.left_count -= 1;
                }
            } else {
                self.occupied += 1;
                if row == 0 {
                    self.top_count += 1;
                }
                if col == 0 {
                    self.left_count += 1;
                }
            }
        }
    }

    /// 调用方只对旋转不变的位图使用本判定：四分之一转只需触及上边，
    /// 半转还需触及左边，其余边由旋转对称性保证。
    pub(crate) fn touches_required_boundary(&self, quarter: bool) -> bool {
        self.top_count != 0 && (quarter || self.left_count != 0)
    }

    /// 对前沿做整字四邻域扩散；行末空位使左右位移不会产生跨行伪邻接。
    pub(crate) fn is_connected(&mut self) -> bool {
        let Some((start_word, &value)) = self.mask.iter().enumerate().find(|(_, word)| **word != 0)
        else {
            return false;
        };
        self.seen.fill(0);
        self.frontier.fill(0);
        let start = value & value.wrapping_neg();
        self.seen[start_word] = start;
        self.frontier[start_word] = start;
        loop {
            let mut has_next = false;
            for index in 0..self.mask.len() {
                let adjacent = shifted_left(&self.frontier, index, 1)
                    | shifted_right(&self.frontier, index, 1)
                    | shifted_left(&self.frontier, index, self.stride)
                    | shifted_right(&self.frontier, index, self.stride);
                let fresh = adjacent & self.mask[index] & !self.seen[index];
                self.next[index] = fresh;
                self.seen[index] |= fresh;
                has_next |= fresh != 0;
            }
            if !has_next {
                break;
            }
            std::mem::swap(&mut self.frontier, &mut self.next);
        }
        self.seen == self.mask
    }

    /// 对连通的非空图形，用 Euler 特征数 χ=V-E+F 判定是否有洞。
    pub(crate) fn has_hole(&self) -> bool {
        let mut faces = 0u128;
        let mut edges = 0u128;
        let mut vertices = 0u128;
        for index in 0..self.mask.len() {
            let face = self.mask[index];
            let right = shifted_left(&self.mask, index, 1);
            let below = shifted_left(&self.mask, index, self.stride);
            let diagonal = shifted_left(&self.mask, index, self.stride + 1);
            faces += u128::from(face.count_ones());
            edges += u128::from((face | right).count_ones());
            edges += u128::from((face | below).count_ones());
            vertices += u128::from((face | right | below | diagonal).count_ones());
        }
        edges + 1 > vertices + faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flood(grid: &[bool], width: usize, height: usize, start: usize) -> usize {
        let mut seen = vec![false; grid.len()];
        let mut pending = vec![start];
        seen[start] = true;
        let mut count = 0;
        while let Some(cell) = pending.pop() {
            count += 1;
            let (row, col) = (cell / width, cell % width);
            for (dr, dc) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                let nr = row as isize + dr;
                let nc = col as isize + dc;
                if nr >= 0 && nc >= 0 && nr < height as isize && nc < width as isize {
                    let neighbor = nr as usize * width + nc as usize;
                    if grid[neighbor] && !seen[neighbor] {
                        seen[neighbor] = true;
                        pending.push(neighbor);
                    }
                }
            }
        }
        count
    }

    fn reference_connected(grid: &[bool], width: usize, height: usize) -> bool {
        let Some(start) = grid.iter().position(|&cell| cell) else {
            return false;
        };
        flood(grid, width, height, start) == grid.iter().filter(|&&cell| cell).count()
    }

    fn reference_hole(grid: &[bool], width: usize, height: usize) -> bool {
        let outer_width = width + 2;
        let outer_height = height + 2;
        let mut background = vec![true; outer_width * outer_height];
        for row in 0..height {
            for col in 0..width {
                background[(row + 1) * outer_width + col + 1] = !grid[row * width + col];
            }
        }
        flood(&background, outer_width, outer_height, 0)
            != background.iter().filter(|&&cell| cell).count()
    }

    #[test]
    fn multiword_geometry_matches_coordinate_flood() {
        let mut random = 0x9e37_79b9_7f4a_7c15u64;
        for (width, height) in [
            (8, 9),
            (9, 8),
            (11, 17),
            (17, 11),
            (63, 3),
            (64, 3),
            (65, 3),
            (1, 129),
            (2, 97),
            (3, 65),
        ] {
            let mut bitmap = DynamicGrid::new(width, height).unwrap();
            let mut grid = vec![false; width * height];
            for sample in 0..96 {
                for cell in 0..grid.len() {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    let wanted = match sample {
                        0 => true,
                        1 => cell == grid.len() - 1,
                        2 => cell % width == width - 1 || cell % width == 0,
                        _ => random % 5 != 0,
                    };
                    if grid[cell] != wanted {
                        bitmap.toggle_cells(&[cell]);
                        grid[cell] = wanted;
                    }
                }
                assert_eq!(
                    bitmap.is_connected(),
                    reference_connected(&grid, width, height),
                    "连通性: {width}×{height}, 样本 {sample}"
                );
                if reference_connected(&grid, width, height) {
                    assert_eq!(
                        bitmap.has_hole(),
                        reference_hole(&grid, width, height),
                        "空洞: {width}×{height}, 样本 {sample}"
                    );
                }
            }
        }
    }

    #[test]
    fn boundary_and_cross_word_ring() {
        let (width, height) = (65, 4);
        let mut bitmap = DynamicGrid::new(width, height).unwrap();
        assert!(!bitmap.is_connected());
        assert!(!bitmap.touches_required_boundary(false));
        let ring: Vec<usize> = (0..height)
            .flat_map(|row| {
                (0..width).filter_map(move |col| {
                    (row == 0 || row == height - 1 || col == 0 || col == width - 1)
                        .then_some(row * width + col)
                })
            })
            .collect();
        bitmap.toggle_cells(&ring);
        assert!(bitmap.touches_required_boundary(false));
        assert!(bitmap.touches_required_boundary(true));
        assert!(bitmap.is_connected());
        assert!(bitmap.has_hole());
        bitmap.toggle_cells(&[width / 2]);
        assert!(bitmap.is_connected());
        assert!(!bitmap.has_hole());
        bitmap.toggle_cells(&[width / 2]);
        bitmap.toggle_cells(&ring);
        assert!(!bitmap.is_connected());
        assert!(!bitmap.touches_required_boundary(false));
    }

    #[test]
    fn checked_layout_rejects_overflow() {
        assert!(DynamicGrid::new(0, 1).is_err());
        assert!(DynamicGrid::new(1, 0).is_err());
        assert!(DynamicGrid::new(usize::MAX, 1).is_err());
        assert!(DynamicGrid::new(1, usize::MAX).is_err());
        assert!(DynamicGrid::new(usize::MAX / 2, 3).is_err());
    }
}
