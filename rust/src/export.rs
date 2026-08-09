//! Fixed mask 二进制导出模块
//!
//! 将枚举过程中发现的 Fixed polyomino 以原始 u64 mask 格式写入磁盘，
//! 按 bounding box max dimension (md) 和有无洞分类存储。
//! 每 10M 个 mask 切分为一个独立文件，zip 后可按块随机解压。
//!
//! ## 目录结构
//!
//! ```text
//! output/
//! ├── no_holes/
//! │   ├── n01_fixed/shapes_000001.bin …  →  n01_fixed.zip
//! │   ├── n02_fixed/shapes_000001.bin …  →  n02_fixed.zip
//! │   └── ...
//! ├── with_holes/
//! │   └── (同上)
//! └── all_fixed/
//!     └── shapes_000001.bin … → all_fixed.zip
//! ```
//!
//! ## 二进制格式
//!
//! 每个 shape: 8 字节 u64 little-endian (规范化位图, stride=8, 左上角对齐)
//! 每个 chunk: CHUNK_SIZE 个 mask (最后一个可能不满)。

use crate::bit_utils::{mask_extent, poly_has_hole};
use crate::types::*;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// 每块 mask 数量，10M = 80 MB/块，zip 内独立压缩条目
pub const CHUNK_SIZE: u64 = 10_000_000;

/// 单个输出槽位（一个 md × hole 组合）
struct Slot {
    dir: PathBuf,
    chunk_idx: u32,   // 从 1 开始
    count: u64,       // 当前块内已写 mask 数
    writer: BufWriter<File>,
    has_data: bool,
}

impl Slot {
    fn new(dir: PathBuf) -> std::io::Result<Self> {
        let file = File::create(dir.join("shapes_000001.bin"))?;
        Ok(Self {
            dir,
            chunk_idx: 1,
            count: 0,
            writer: BufWriter::with_capacity(8 * 1024 * 1024, file),
            has_data: false,
        })
    }

    fn write_mask(&mut self, bytes: &[u8; 8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.count += 1;
        self.has_data = true;

        if self.count >= CHUNK_SIZE {
            self.rotate()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.writer.flush()?;
        self.chunk_idx += 1;
        self.count = 0;
        let name = format!("shapes_{:06}.bin", self.chunk_idx);
        let file = File::create(self.dir.join(name))?;
        self.writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// 管理所有导出文件句柄
pub struct ExportManager {
    base_dir: PathBuf,
    /// 12 个 slot: [no_hole md1..6, with_holes md1..6]
    slots: Mutex<Vec<Slot>>,
    /// all_fixed slot (None after close)
    all_slot: Mutex<Option<Slot>>,
}

impl ExportManager {
    pub fn new(base_dir: &Path) -> std::io::Result<Self> {
        let mut slots = Vec::with_capacity(12);

        for &has_hole in &[false, true] {
            let category = if has_hole { "with_holes" } else { "no_holes" };
            for md in 1..=MAX_N {
                let dir = base_dir.join(category).join(format!("n{:02}_fixed", md));
                fs::create_dir_all(&dir)?;
                slots.push(Slot::new(dir)?);
            }
        }

        let all_dir = base_dir.join("all_fixed");
        fs::create_dir_all(&all_dir)?;
        let all_slot = Some(Slot::new(all_dir)?);

        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            slots: Mutex::new(slots),
            all_slot: Mutex::new(all_slot),
        })
    }

    pub fn write_batch(&self, batch: &[(Mask, usize, bool)]) -> std::io::Result<()> {
        let mut slots = self.slots.lock().unwrap();
        let mut all_slot = self.all_slot.lock().unwrap();

        for &(mask, md, has_hole) in batch {
            let idx = (md - 1) + if has_hole { MAX_N } else { 0 };
            let bytes = mask.to_le_bytes();
            slots[idx].write_mask(&bytes)?;
            if let Some(ref mut s) = *all_slot {
                s.write_mask(&bytes)?;
            }
        }

        Ok(())
    }

    pub fn flush_all(&self) -> std::io::Result<()> {
        let mut slots = self.slots.lock().unwrap();
        for s in slots.iter_mut() {
            s.flush()?;
        }
        let mut all_slot = self.all_slot.lock().unwrap();
        if let Some(ref mut s) = *all_slot {
            s.flush()?;
        }
        Ok(())
    }

    /// 关闭所有文件句柄（为 7-zip 压缩做准备）
    fn close_all(&self) {
        let mut slots = self.slots.lock().unwrap();
        slots.clear();
        let mut all_slot = self.all_slot.lock().unwrap();
        *all_slot = None;
    }

    /// 调用 7-zip 压缩所有输出目录
    pub fn compress_7z(&self) -> std::io::Result<()> {
        self.flush_all()?;

        // 获取 has_data 快照
        let has_data: Vec<bool> = {
            let slots = self.slots.lock().unwrap();
            slots.iter().map(|s| s.has_data).collect()
        };

        // 关闭文件句柄，确保 Windows 上 7z 可读取
        self.close_all();

        let zip_exe = "C:/Program Files/7-Zip/7z.exe";

        for &has_hole in &[false, true] {
            let category = if has_hole { "with_holes" } else { "no_holes" };
            for md in 1..=MAX_N {
                let idx = (md - 1) + if has_hole { MAX_N } else { 0 };
                if idx >= has_data.len() || !has_data[idx] {
                    continue;
                }

                let sub_dir = format!("n{:02}_fixed", md);
                let target_dir = self.base_dir.join(category).join(&sub_dir);

                if !target_dir.exists() {
                    continue;
                }

                eprintln!("  [7z] compress {}/{} ...", category, sub_dir);

                let status = Command::new(zip_exe)
                    .current_dir(self.base_dir.join(category))
                    .args([
                        "a", "-tzip", "-mx=9", "-bso0",
                        format!("{}.zip", sub_dir).as_str(),
                        sub_dir.as_str(),
                    ])
                    .status()?;

                if status.success() {
                    let _ = fs::remove_dir_all(&target_dir);
                } else {
                    eprintln!("  [7z] warn: {} compress failed", sub_dir);
                }
            }
        }

        // all_fixed
        let all_dir = self.base_dir.join("all_fixed");
        if all_dir.exists() {
            eprintln!("  [7z] compress all_fixed ...");
            let status = Command::new(zip_exe)
                .current_dir(&self.base_dir)
                .args(["a", "-tzip", "-mx=9", "-bso0", "all_fixed.zip", "all_fixed"])
                .status()?;
            if status.success() {
                let _ = fs::remove_dir_all(&all_dir);
            }
        }

        eprintln!("  [7z] compress done");
        Ok(())
    }
}

/// 从 mask 计算导出的元数据：(md, has_hole)
#[inline]
pub fn compute_meta(mask: Mask) -> (usize, bool) {
    let (w, h) = mask_extent(mask);
    let md = w.max(h);
    let has_hole = poly_has_hole(mask, w, h);
    (md, has_hole)
}
