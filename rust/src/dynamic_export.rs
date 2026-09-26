//! 动态位图的单份分类输出。v3显式记录步长、记录宽度及十进制大整数计数。
use crate::dynamic;
use num_bigint::BigUint;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const BUFFER: usize = 256 * 1024;
const CHUNK_RECORDS: u64 = 10_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Raw,
    Native(u8),
    SevenZip(u8),
}

struct Slot {
    dir: PathBuf,
    raw: Option<BufWriter<File>>,
    zip: Option<ZipWriter<File>>,
    buffer: Vec<u8>,
    chunk: BigUint,
    in_chunk: u64,
    total: BigUint,
}

impl Slot {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            raw: None,
            zip: None,
            buffer: Vec::new(),
            chunk: 1u32.into(),
            in_chunk: 0,
            total: BigUint::default(),
        }
    }
    fn chunk_name(&self) -> String {
        format!("shapes_{:0>6}.bin", self.chunk.to_string())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(raw) = &mut self.raw {
            raw.flush()?;
        }
        if !self.buffer.is_empty() {
            self.zip.as_mut().unwrap().write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
    fn write(&mut self, record: &[u8], mode: Mode, chunk_records: u64) -> io::Result<()> {
        if self.in_chunk == chunk_records {
            self.flush()?;
            self.raw = None;
            self.in_chunk = 0;
            self.chunk += 1u32;
        }
        let name = self.chunk_name();
        if let Mode::Native(level) = mode {
            if self.zip.is_none() {
                fs::create_dir_all(self.dir.parent().unwrap())?;
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(self.dir.with_extension("zip.partial"))?;
                self.zip = Some(ZipWriter::new(file));
            }
            if self.in_chunk == 0 {
                let options = SimpleFileOptions::default()
                    .large_file(true)
                    .compression_method(if level == 0 {
                        CompressionMethod::Stored
                    } else {
                        CompressionMethod::Deflated
                    })
                    .compression_level(if level == 0 { None } else { Some(level as i64) });
                let path = format!(
                    "{}/{}",
                    self.dir.file_name().unwrap().to_string_lossy(),
                    name
                );
                self.zip
                    .as_mut()
                    .unwrap()
                    .start_file(path, options)
                    .map_err(io::Error::other)?;
            }
            // 记录宽度不一定整除缓冲区，使用>=而非固定8字节路线的==。
            if record.len() >= BUFFER {
                self.flush()?;
                self.zip.as_mut().unwrap().write_all(record)?;
            } else {
                self.buffer.extend_from_slice(record);
                if self.buffer.len() >= BUFFER {
                    self.flush()?;
                }
            }
        } else {
            if self.raw.is_none() {
                fs::create_dir_all(&self.dir)?;
                self.raw = Some(BufWriter::with_capacity(
                    BUFFER,
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(self.dir.join(name))?,
                ));
            }
            self.raw.as_mut().unwrap().write_all(record)?;
        }
        self.in_chunk += 1;
        self.total += 1u32;
        Ok(())
    }
}

pub struct Export {
    root: PathBuf,
    n: usize,
    stride: usize,
    record_bytes: usize,
    mode: Mode,
    slots: BTreeMap<(bool, usize), Slot>,
    closed: bool,
}

impl Export {
    pub fn new(root: &Path, n: usize, mode: Mode) -> io::Result<Self> {
        let (stride, record_bytes) = dynamic::layout(n)?;
        if matches!(mode, Mode::Native(level) | Mode::SevenZip(level) if level > 9) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "压缩等级必须在0..9",
            ));
        }
        if root.exists() && fs::read_dir(root)?.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "导出目录非空；请指定新目录",
            ));
        }
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.to_owned(),
            n,
            stride,
            record_bytes,
            mode,
            slots: BTreeMap::new(),
            closed: false,
        })
    }

    pub fn write(&mut self, mask: &BigUint, md: usize, hole: bool) -> io::Result<()> {
        if self.closed {
            return Err(io::Error::other("导出已经关闭"));
        }
        if md == 0
            || md > self.n
            || mask == &BigUint::default()
            || mask.bits() > (self.n * self.stride) as u64
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "形状为空或超出数据集尺寸",
            ));
        }
        let mut record = mask.to_bytes_le();
        record.resize(self.record_bytes, 0);
        let root = &self.root;
        let slot = self.slots.entry((hole, md)).or_insert_with(|| {
            Slot::new(
                root.join(if hole { "with_holes" } else { "no_holes" })
                    .join(format!("n{md:02}_fixed")),
            )
        });
        slot.write(&record, self.mode, CHUNK_RECORDS)
    }

    /// 文件与清单只有全部成功后才被发布为完整数据集。
    pub fn finish(&mut self, expected: &BigUint) -> io::Result<()> {
        if self.closed {
            return Err(io::Error::other("导出已经关闭"));
        }
        self.closed = true;
        let total: BigUint = self.slots.values().map(|slot| &slot.total).sum();
        if &total != expected {
            return Err(io::Error::other("导出条数与计数不一致"));
        }
        for slot in self.slots.values_mut() {
            slot.flush()?;
            slot.raw = None;
            if let Some(zip) = slot.zip.take() {
                zip.finish().map_err(io::Error::other)?.sync_all()?;
            }
        }
        match self.mode {
            Mode::Native(_) => {
                for slot in self.slots.values() {
                    if slot.dir.with_extension("zip").exists() {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "目标ZIP已存在",
                        ));
                    }
                }
                for slot in self.slots.values() {
                    fs::rename(
                        slot.dir.with_extension("zip.partial"),
                        slot.dir.with_extension("zip"),
                    )?;
                }
            }
            Mode::SevenZip(level) => self.compress_7z(level)?,
            Mode::Raw => (),
        }
        let extension = if self.mode == Mode::Raw { "" } else { ".zip" };
        let streams = self.slots.iter().map(|(&(hole, md), slot)| {
            let category = if hole { "with_holes" } else { "no_holes" };
            format!("    {{\"path\": \"{category}/n{md:02}_fixed{extension}\", \"max_dimension\": {md}, \"has_hole\": {hole}, \"count\": \"{}\"}}", slot.total)
        }).collect::<Vec<_>>().join(",\n");
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let manifest = format!(concat!("{{\n  \"format_version\": 3,\n  \"dataset_id\": \"{}-{}\",\n",
            "  \"max_n\": {},\n  \"stride\": {},\n  \"record_bytes\": {},\n  \"count\": \"{}\",\n",
            "  \"equivalence\": \"one-sided\",\n  \"encoding\": \"uint-le-fixed\",\n",
            "  \"representative\": \"minimum-rotation\",\n  \"category_dimension\": \"exact-max-bounding-box\",\n",
            "  \"storage_layout\": \"classified-single-copy\",\n  \"ordering\": \"category-major-parallel-unspecified\",\n",
            "  \"streams\": [\n{}\n  ],\n  \"complete\": true\n}}\n"), id, std::process::id(), self.n, self.stride, self.record_bytes, total, streams);
        let partial = self.root.join("dataset.json.partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        file.write_all(manifest.as_bytes())?;
        file.sync_all()?;
        drop(file);
        let target = self.root.join("dataset.json");
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "数据集清单已存在",
            ));
        }
        fs::rename(partial, target)
    }

    fn compress_7z(&self, level: u8) -> io::Result<()> {
        let executable = std::env::var_os("ROOM_COUNT_7Z").unwrap_or_else(|| {
            let path = Path::new("C:/Program Files/7-Zip/7z.exe");
            if path.exists() {
                path.as_os_str().to_owned()
            } else {
                "7z".into()
            }
        });
        for slot in self.slots.values() {
            if slot.dir.with_extension("zip").exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "目标ZIP已存在",
                ));
            }
            let name = slot.dir.file_name().unwrap();
            let status = Command::new(&executable)
                .current_dir(slot.dir.parent().unwrap())
                .args(["a", "-tzip", "-bso0"])
                .arg(format!("-mx={level}"))
                .arg(format!("{}.zip", name.to_string_lossy()))
                .arg(name)
                .status()?;
            if !status.success() {
                return Err(io::Error::other("7-Zip压缩失败，裸数据已保留"));
            }
            let root = self.root.canonicalize()?;
            let target = slot.dir.canonicalize()?;
            if !target.starts_with(&root) || target == root {
                return Err(io::Error::other("清理路径越界"));
            }
            fs::remove_dir_all(target)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    fn temp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "room-count-dynamic-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn raw_and_zip_preserve_bit_above_64_and_decimal_manifest() {
        let mask = (BigUint::from(1u32) << 80usize) | BigUint::from(1u32);
        for mode in [Mode::Raw, Mode::Native(0), Mode::Native(1)] {
            let root = temp("wide");
            let mut output = Export::new(&root, 9, mode).unwrap();
            output.write(&mask, 9, false).unwrap();
            output.finish(&1u32.into()).unwrap();
            let manifest = fs::read_to_string(root.join("dataset.json")).unwrap();
            assert!(manifest.contains("\"record_bytes\": 11"));
            assert!(manifest.contains("\"count\": \"1\""));
            let bytes = if mode == Mode::Raw {
                fs::read(root.join("no_holes/n09_fixed/shapes_000001.bin")).unwrap()
            } else {
                let mut zip =
                    zip::ZipArchive::new(File::open(root.join("no_holes/n09_fixed.zip")).unwrap())
                        .unwrap();
                let mut bytes = Vec::new();
                zip.by_index(0).unwrap().read_to_end(&mut bytes).unwrap();
                bytes
            };
            assert_eq!(bytes.len(), 11);
            assert_eq!(BigUint::from_bytes_le(&bytes), mask);
            assert!(output.write(&mask, 9, false).is_err());
            assert!(Export::new(&root, 9, mode).is_err());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn wide_zip_chunks_and_finish_failure() {
        let root = temp("chunks");
        let mut slot = Slot::new(root.join("no_holes/n09_fixed"));
        for _ in 0..5 {
            slot.write(&[1; 11], Mode::Native(1), 2).unwrap();
        }
        slot.flush().unwrap();
        slot.zip.take().unwrap().finish().unwrap();
        let mut archive =
            zip::ZipArchive::new(File::open(slot.dir.with_extension("zip.partial")).unwrap())
                .unwrap();
        assert_eq!(archive.len(), 3);
        for (i, length) in [(0, 22), (1, 22), (2, 11)] {
            assert_eq!(archive.by_index(i).unwrap().size(), length);
        }
        drop(archive);
        let mut output = Export::new(&root.join("failed"), 9, Mode::Native(1)).unwrap();
        output.write(&1u32.into(), 1, false).unwrap();
        assert!(output.finish(&2u32.into()).is_err());
        assert!(!root.join("failed/dataset.json").exists());
        drop(output);
        fs::remove_dir_all(root).unwrap();
    }
}
