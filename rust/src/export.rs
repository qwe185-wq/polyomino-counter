//! 有界流式导出：按实际有数据的槽懒分配，保护既有数据集。
use crate::types::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

pub const CHUNK_SIZE: u64 = 10_000_000;
const BUFFER_SIZE: usize = 256 * 1024;

struct Slot {
    dir: PathBuf,
    writer: Option<BufWriter<File>>,
    chunk: u32,
    count: u64,
    has_data: bool,
}
impl Slot {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writer: None,
            chunk: 1,
            count: 0,
            has_data: false,
        }
    }
    fn write(&mut self, bytes: &[u8; 8]) -> io::Result<()> {
        if self.count == CHUNK_SIZE {
            self.flush()?;
            self.writer = None;
            self.chunk += 1;
            self.count = 0;
        }
        if self.writer.is_none() {
            fs::create_dir_all(&self.dir)?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.dir.join(format!("shapes_{:06}.bin", self.chunk)))?;
            self.writer = Some(BufWriter::with_capacity(BUFFER_SIZE, file));
        }
        self.writer.as_mut().unwrap().write_all(bytes)?;
        self.count += 1;
        self.has_data = true;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.flush()?;
        }
        Ok(())
    }
}

struct Output {
    slots: Vec<Slot>,
    all: Slot,
    closed: bool,
}
pub struct ExportManager {
    base_dir: PathBuf,
    output: Mutex<Output>,
}
impl ExportManager {
    pub fn new(base_dir: &Path) -> io::Result<Self> {
        if base_dir.exists() && fs::read_dir(base_dir)?.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "导出目录非空；请为新数据集指定新目录，保留旧global_index",
            ));
        }
        fs::create_dir_all(base_dir)?;
        let slots = ["no_holes", "with_holes"]
            .into_iter()
            .flat_map(|category| {
                (1..=MAX_N).map(move |md| {
                    Slot::new(base_dir.join(category).join(format!("n{md:02}_fixed")))
                })
            })
            .collect();
        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            output: Mutex::new(Output {
                slots,
                all: Slot::new(base_dir.join("all_fixed")),
                closed: false,
            }),
        })
    }

    pub fn write_batch(&self, batch: &[(Mask, usize, bool)]) -> io::Result<()> {
        let mut output = self
            .output
            .lock()
            .map_err(|_| io::Error::other("导出状态异常"))?;
        if output.closed {
            return Err(io::Error::other("导出文件已关闭"));
        }
        for &(mask, md, hole) in batch {
            if !(1..=MAX_N).contains(&md) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "包围盒越界"));
            }
            let index = md - 1 + if hole { MAX_N } else { 0 };
            let bytes = mask.to_le_bytes();
            output.slots[index].write(&bytes)?;
            output.all.write(&bytes)?;
        }
        Ok(())
    }

    pub fn flush_all(&self) -> io::Result<()> {
        let mut output = self
            .output
            .lock()
            .map_err(|_| io::Error::other("导出状态异常"))?;
        for slot in &mut output.slots {
            slot.flush()?;
        }
        output.all.flush()
    }

    pub fn compress_7z(&self, level: u8) -> io::Result<()> {
        if level > 9 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "压缩等级必须在0..9",
            ));
        }
        self.flush_all()?;
        let dirs = {
            let mut output = self
                .output
                .lock()
                .map_err(|_| io::Error::other("导出状态异常"))?;
            output.closed = true;
            let mut dirs = Vec::new();
            for slot in &mut output.slots {
                slot.writer = None;
                if slot.has_data {
                    dirs.push(slot.dir.clone());
                }
            }
            output.all.writer = None;
            if output.all.has_data {
                dirs.push(output.all.dir.clone());
            }
            dirs
        };
        let zip = std::env::var_os("ROOM_COUNT_7Z").unwrap_or_else(|| {
            let windows = Path::new("C:/Program Files/7-Zip/7z.exe");
            if windows.exists() {
                windows.as_os_str().to_owned()
            } else {
                "7z".into()
            }
        });
        for dir in dirs {
            let name = dir
                .file_name()
                .ok_or_else(|| io::Error::other("导出目录无文件名"))?;
            let archive = format!("{}.zip", name.to_string_lossy());
            let status = Command::new(&zip)
                .current_dir(dir.parent().unwrap())
                .args(["a", "-tzip", "-bso0"])
                .arg(format!("-mx={level}"))
                .arg(&archive)
                .arg(name)
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "压缩失败: {archive} ({status})；原始文件已保留"
                )));
            }
            // 仅删除本次新建且压缩成功的数据目录，不触及已有数据集。
            let root = self.base_dir.canonicalize()?;
            let target = dir.canonicalize()?;
            if !target.starts_with(&root) || target == root {
                return Err(io::Error::other("压缩清理路径越界"));
            }
            fs::remove_dir_all(target)?;
        }
        Ok(())
    }
}
