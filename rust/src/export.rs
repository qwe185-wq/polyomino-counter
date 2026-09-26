//! 有界流式导出：按实际有数据的槽懒分配，保护既有数据集。
use crate::types::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub const CHUNK_SIZE: u64 = 10_000_000;
const BUFFER_SIZE: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Raw,
    Zip(u8),
}

struct Slot {
    dir: PathBuf,
    mode: OutputMode,
    writer: Option<BufWriter<File>>,
    zip: Option<ZipWriter<File>>,
    zip_buffer: Vec<u8>,
    chunk: u32,
    count: u64,
    total: u64,
    has_data: bool,
}
impl Slot {
    fn new(dir: PathBuf, mode: OutputMode) -> Self {
        Self {
            dir,
            mode,
            writer: None,
            zip: None,
            zip_buffer: Vec::new(),
            chunk: 1,
            count: 0,
            total: 0,
            has_data: false,
        }
    }
    fn archive_path(&self) -> PathBuf {
        self.dir.with_extension("zip")
    }
    fn partial_path(&self) -> PathBuf {
        self.dir.with_extension("zip.partial")
    }

    fn zip_options(level: u8) -> SimpleFileOptions {
        let method = if level == 0 {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(if level == 0 { None } else { Some(level.into()) })
    }

    fn start_zip_chunk(&mut self, level: u8) -> io::Result<()> {
        if self.zip.is_none() {
            fs::create_dir_all(self.dir.parent().unwrap())?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.partial_path())?;
            self.zip = Some(ZipWriter::new(file));
            self.zip_buffer = Vec::with_capacity(BUFFER_SIZE);
        }
        let name = format!(
            "{}/shapes_{:06}.bin",
            self.dir.file_name().unwrap().to_string_lossy(),
            self.chunk
        );
        self.zip
            .as_mut()
            .unwrap()
            .start_file(name, Self::zip_options(level))
            .map_err(io::Error::other)
    }

    fn write(&mut self, bytes: &[u8; 8], chunk_size: u64) -> io::Result<()> {
        if self.count == chunk_size {
            self.flush()?;
            if self.mode == OutputMode::Raw {
                self.writer = None;
            }
            self.chunk += 1;
            self.count = 0;
        }
        match self.mode {
            OutputMode::Raw => {
                if self.writer.is_none() {
                    fs::create_dir_all(&self.dir)?;
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(self.dir.join(format!("shapes_{:06}.bin", self.chunk)))?;
                    self.writer = Some(BufWriter::with_capacity(BUFFER_SIZE, file));
                }
                self.writer.as_mut().unwrap().write_all(bytes)?;
            }
            OutputMode::Zip(level) => {
                if self.count == 0 {
                    self.start_zip_chunk(level)?;
                }
                self.zip_buffer.extend_from_slice(bytes);
                if self.zip_buffer.len() == BUFFER_SIZE {
                    self.flush()?;
                }
            }
        }
        self.count += 1;
        self.total += 1;
        self.has_data = true;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.flush()?;
        }
        if !self.zip_buffer.is_empty() {
            self.zip.as_mut().unwrap().write_all(&self.zip_buffer)?;
            self.zip_buffer.clear();
        }
        Ok(())
    }

    fn finish_zip(&mut self) -> io::Result<()> {
        self.flush()?;
        if let Some(zip) = self.zip.take() {
            let file = zip.finish().map_err(io::Error::other)?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn publish_zip(&self) -> io::Result<()> {
        if self.has_data {
            fs::rename(self.partial_path(), self.archive_path())?;
        }
        Ok(())
    }
}

struct Output {
    slots: Vec<Slot>,
    mode: OutputMode,
    closed: bool,
}
pub struct ExportManager {
    base_dir: PathBuf,
    output: Mutex<Output>,
}
impl ExportManager {
    pub fn new(base_dir: &Path) -> io::Result<Self> {
        Self::new_with_mode(base_dir, OutputMode::Raw)
    }

    pub fn new_zip(base_dir: &Path, level: u8) -> io::Result<Self> {
        if level > 9 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "压缩等级必须在0..9",
            ));
        }
        Self::new_with_mode(base_dir, OutputMode::Zip(level))
    }

    fn new_with_mode(base_dir: &Path, mode: OutputMode) -> io::Result<Self> {
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
                    Slot::new(
                        base_dir.join(category).join(format!("n{md:02}_fixed")),
                        mode,
                    )
                })
            })
            .collect();
        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            output: Mutex::new(Output {
                slots,
                mode,
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
            output.slots[index].write(&bytes, CHUNK_SIZE)?;
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
        Ok(())
    }

    pub fn finish_zip(&self) -> io::Result<()> {
        let mut output = self
            .output
            .lock()
            .map_err(|_| io::Error::other("导出状态异常"))?;
        if !matches!(output.mode, OutputMode::Zip(_)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "当前导出不是ZIP模式",
            ));
        }
        if output.closed {
            return Err(io::Error::other("导出文件已关闭"));
        }
        output.closed = true;
        // 所有 ZIP 都完成后再发布正式文件，失败时保留 .partial 供检查。
        for slot in &mut output.slots {
            slot.finish_zip()?;
        }
        for slot in &output.slots {
            if slot.has_data && slot.archive_path().exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "目标ZIP已存在",
                ));
            }
        }
        for slot in &output.slots {
            slot.publish_zip()?;
        }
        Ok(())
    }

    /// 每个记录只在一个分类流中；清单顺序定义本数据集的逻辑全集顺序。
    pub fn stream_manifest(&self, compressed: bool, expected_total: u64) -> io::Result<String> {
        let output = self
            .output
            .lock()
            .map_err(|_| io::Error::other("导出状态异常"))?;
        let total: u64 = output.slots.iter().map(|slot| slot.total).sum();
        if total != expected_total {
            return Err(io::Error::other(format!(
                "导出记录数不符: {total} != {expected_total}"
            )));
        }
        let streams: Vec<String> = output.slots.iter().enumerate()
            .filter(|(_, slot)| slot.has_data)
            .map(|(i, slot)| {
                let md = i % MAX_N + 1;
                let hole = i >= MAX_N;
                let category = if hole { "with_holes" } else { "no_holes" };
                let extension = if compressed { ".zip" } else { "" };
                format!("    {{\"path\": \"{category}/n{md:02}_fixed{extension}\", \"max_dimension\": {md}, \"has_hole\": {hole}, \"count\": {}}}", slot.total)
            }).collect();
        Ok(streams.join(",\n"))
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
            if output.mode != OutputMode::Raw {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "当前导出不是裸文件模式",
                ));
            }
            output.closed = true;
            let mut dirs = Vec::new();
            for slot in &mut output.slots {
                slot.writer = None;
                if slot.has_data {
                    dirs.push(slot.dir.clone());
                }
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

#[cfg(test)]
mod zip_tests {
    use super::*;
    use std::io::Read;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "room-count-native-zip-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn native_zip_keeps_chunk_boundaries_and_reads_back() {
        let temp = TestDir::new();
        let dir = temp.0.join("no_holes").join("n01_fixed");
        let mut slot = Slot::new(dir.clone(), OutputMode::Zip(1));
        for mask in 1u64..=5 {
            slot.write(&mask.to_le_bytes(), 2).unwrap();
        }
        slot.finish_zip().unwrap();
        let partial = dir.with_extension("zip.partial");
        assert!(partial.is_file());
        assert!(!dir.with_extension("zip").exists());
        slot.publish_zip().unwrap();
        let file = File::open(dir.with_extension("zip")).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.len(), 3);
        for (chunk, expected) in [(1, vec![1, 2]), (2, vec![3, 4]), (3, vec![5])] {
            let name = format!("n01_fixed/shapes_{chunk:06}.bin");
            let mut bytes = Vec::new();
            archive
                .by_name(&name)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            let actual: Vec<u64> = bytes
                .chunks_exact(8)
                .map(|part| u64::from_le_bytes(part.try_into().unwrap()))
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn native_zip_finish_failure_keeps_partial_and_no_manifest() {
        let temp = TestDir::new();
        let manager = ExportManager::new_zip(&temp.0.join("dataset"), 0).unwrap();
        manager.write_batch(&[(1, 1, false)]).unwrap();
        let stream = temp.0.join("dataset/no_holes/n01_fixed");
        fs::create_dir(stream.with_extension("zip")).unwrap();
        assert!(manager.finish_zip().is_err());
        assert!(stream.with_extension("zip.partial").is_file());
        assert!(!temp.0.join("dataset/dataset.json").exists());
        assert!(manager.write_batch(&[(2, 1, false)]).is_err());
    }

    #[test]
    fn native_zip_zero_level_is_stored_and_manager_closes() {
        let temp = TestDir::new();
        let dataset = temp.0.join("dataset");
        let manager = ExportManager::new_zip(&dataset, 0).unwrap();
        manager
            .write_batch(&[(1, 1, false), (2, 2, false)])
            .unwrap();
        manager.finish_zip().unwrap();
        assert!(manager.write_batch(&[(3, 1, false)]).is_err());
        let file = File::open(dataset.join("no_holes/n01_fixed.zip")).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let entry = archive.by_name("n01_fixed/shapes_000001.bin").unwrap();
        assert_eq!(entry.compression(), CompressionMethod::Stored);
        assert_eq!(
            manager
                .stream_manifest(true, 2)
                .unwrap()
                .matches("\"path\"")
                .count(),
            2
        );
        assert!(!dataset.join("no_holes/n01_fixed.zip.partial").exists());
        assert!(ExportManager::new_zip(&dataset, 1).is_err());
        assert!(ExportManager::new_zip(&temp.0.join("invalid"), 10).is_err());
    }
}
