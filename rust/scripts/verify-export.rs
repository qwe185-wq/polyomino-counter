//! 独立裸导出验收器。仅使用标准库；scratch 必须是新目录或空目录。
//! 编译：rustc -O verify_exact.rs -o verify_exact.exe
//! 用法：verify_exact <dataset-root> <scratch-new-dir> <n>，n=1..6。

use std::convert::TryInto;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;

const BUCKETS: usize = 256;
const WRITER_BUFFER: usize = 64 * 1024; // 合计 16 MiB
const INPUT_BUFFER: usize = 1024 * 1024;
const MAX_BUCKET_BYTES: u64 = 20 * 1024 * 1024; // 四线程至多 80 MiB 的桶向量
const MASK_BITS: u64 = (1u64 << 48) - 1;
const COL0: u64 = 0x0101_0101_0101_0101;
const COL7: u64 = 0x8080_8080_8080_8080;
const TOTAL: [u64; 7] = [0, 1, 4, 46, 2404, 520818, 410964612];
const NO_HOLES: [u64; 7] = [0, 1, 4, 44, 1899, 267976, 112877832];
const WITH_HOLES: [u64; 7] = [0, 0, 0, 2, 505, 252842, 298086780];

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Default)]
struct Counts([[u64; 7]; 2]);

impl Counts {
    fn add(&mut self, other: &Self) {
        for hole in 0..2 {
            for md in 1..=6 {
                self.0[hole][md] += other.0[hole][md];
            }
        }
    }
}

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message.into()).into())
}

fn hash_mask(mut x: u64) -> usize {
    // 固定、可重现的混合函数只决定磁盘桶，不承担去重正确性。
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x as usize & (BUCKETS - 1)
}

fn bucket_path(scratch: &Path, i: usize) -> PathBuf {
    scratch.join(format!("bucket_{i:03}.bin"))
}

fn prepare_scratch(scratch: &Path) -> Result<Vec<BufWriter<File>>> {
    if scratch.exists() {
        if !scratch.is_dir() || fs::read_dir(scratch)?.next().is_some() {
            return fail(format!("scratch 必须为空目录：{}", scratch.display()));
        }
    } else {
        fs::create_dir(scratch)?;
    }
    let mut writers = Vec::with_capacity(BUCKETS);
    for i in 0..BUCKETS {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(bucket_path(scratch, i))?;
        writers.push(BufWriter::with_capacity(WRITER_BUFFER, file));
    }
    Ok(writers)
}

fn shape_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        return fail(format!("形状目录不是目录：{}", dir.display()));
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let number = name
            .strip_prefix("shapes_")
            .and_then(|s| s.strip_suffix(".bin"))
            .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u32>().ok());
        let Some(number) = number else {
            return fail(format!("形状目录含非预期条目：{}", entry.path().display()));
        };
        if !entry.file_type()?.is_file() {
            return fail(format!("形状条目不是普通文件：{}", entry.path().display()));
        }
        entries.push((number, entry.path()));
    }
    entries.sort_unstable_by_key(|(i, _)| *i);
    for (index, (number, _)) in entries.iter().enumerate() {
        if *number != index as u32 + 1 {
            return fail(format!("分块编号不连续：{}", dir.display()));
        }
    }
    Ok(entries.into_iter().map(|(_, path)| path).collect())
}

fn check_layout(dataset: &Path, max_n: usize) -> Result<()> {
    for entry in fs::read_dir(dataset)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "dataset.json" && entry.file_type()?.is_file() {
            continue;
        }
        let expected = if name == "no_holes" {
            NO_HOLES
        } else if name == "with_holes" {
            WITH_HOLES
        } else {
            return fail(format!("数据集含未列出条目：{}", entry.path().display()));
        };
        if !entry.file_type()?.is_dir() {
            return fail(format!("分类不是普通目录：{}", entry.path().display()));
        }
        for stream in fs::read_dir(entry.path())? {
            let stream = stream?;
            let allowed = (1..=max_n).any(|md| {
                expected[md] > expected[md - 1]
                    && stream.file_name() == format!("n{md:02}_fixed").as_str()
            });
            if !allowed || !stream.file_type()?.is_dir() {
                return fail(format!("分类含未列出流：{}", stream.path().display()));
            }
        }
    }
    Ok(())
}

fn distribute_file(
    path: &Path,
    md: usize,
    hole: usize,
    writers: &mut [BufWriter<File>],
) -> Result<u64> {
    let expected_bytes = fs::metadata(path)?.len();
    if expected_bytes % 8 != 0 {
        return fail(format!(
            "文件含残缺记录（尾部不足 8 字节）：{}",
            path.display()
        ));
    }
    let mut file = File::open(path)?;
    let mut block = vec![0u8; INPUT_BUFFER + 7];
    let mut pending = 0usize;
    let mut bytes = 0u64;
    let mut count = 0u64;
    loop {
        let read = file.read(&mut block[pending..INPUT_BUFFER + pending])?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        let full = (pending + read) & !7;
        for chunk in block[..full].chunks_exact(8) {
            let mask = u64::from_le_bytes(chunk.try_into().unwrap());
            // 原始数据不能占用标签位，且 n<=6 的最高有效格为 bit45。
            if mask & !MASK_BITS != 0 {
                return fail(format!(
                    "掩码占用标签位：{}，记录 {}",
                    path.display(),
                    count + 1
                ));
            }
            let tagged = mask | ((md as u64) << 60) | ((hole as u64) << 63);
            writers[hash_mask(mask)].write_all(&tagged.to_le_bytes())?;
            count += 1;
        }
        pending = pending + read - full;
        if pending != 0 {
            block.copy_within(full..full + pending, 0);
        }
    }
    if pending != 0 || bytes != expected_bytes {
        return fail(format!(
            "文件读数与元数据不符或尾部残缺：{}",
            path.display()
        ));
    }
    Ok(count)
}

#[inline]
fn neighbors(x: u64) -> u64 {
    ((x & !COL0) >> 1) | ((x & !COL7) << 1) | (x >> 8) | (x << 8)
}

#[inline]
fn flood(seed: u64, allowed: u64) -> u64 {
    let mut reached = seed;
    loop {
        let next = reached | (neighbors(reached) & allowed);
        if next == reached {
            return reached;
        }
        reached = next;
    }
}

struct Rotations {
    // 逐行坐标生成的顺时针 90° 贡献表，不借用生产位运算。
    quarter: [[[u64; 64]; 6]; 6],
    reverse: [[u8; 64]; 6],
}

impl Rotations {
    fn new() -> Self {
        let mut result = Self {
            quarter: [[[0; 64]; 6]; 6],
            reverse: [[0; 64]; 6],
        };
        for h in 1..=6 {
            for r in 0..h {
                for byte in 0..64usize {
                    let mut out = 0u64;
                    for c in 0..6 {
                        if byte & (1 << c) != 0 {
                            out |= 1u64 << (8 * c + h - 1 - r);
                        }
                    }
                    result.quarter[h - 1][r][byte] = out;
                }
            }
        }
        for w in 1..=6 {
            for byte in 0..64usize {
                let mut out = 0u8;
                for c in 0..w {
                    if byte & (1 << c) != 0 {
                        out |= 1 << (w - 1 - c);
                    }
                }
                result.reverse[w - 1][byte] = out;
            }
        }
        result
    }

    #[inline]
    fn half(&self, mask: u64, w: usize, h: usize) -> u64 {
        let mut out = 0u64;
        for r in 0..h {
            let byte = ((mask >> (8 * r)) & 63) as usize;
            out |= (self.reverse[w - 1][byte] as u64) << (8 * (h - 1 - r));
        }
        out
    }

    #[inline]
    fn quarter(&self, mask: u64, h: usize) -> u64 {
        let mut out = 0u64;
        for r in 0..h {
            out |= self.quarter[h - 1][r][((mask >> (8 * r)) & 63) as usize];
        }
        out
    }
}

fn validate(packed: u64, max_n: usize, rotations: &Rotations) -> Result<(usize, usize, u64)> {
    let mask = packed & MASK_BITS;
    let md = ((packed >> 60) & 7) as usize;
    let hole = (packed >> 63) as usize;
    if mask == 0 || md == 0 || md > max_n {
        return fail(format!("空形状或错误边长标签：{packed:016x}"));
    }
    let mut columns = 0u8;
    for r in 0..6 {
        columns |= (mask >> (8 * r)) as u8;
    }
    if mask.trailing_zeros() >= 8 || columns & 1 == 0 {
        return fail(format!("形状没有平移归一化：{packed:016x}"));
    }
    let h = ((63 - mask.leading_zeros()) / 8 + 1) as usize;
    let w = (8 - columns.leading_zeros()) as usize;
    if h > max_n || w > max_n || h.max(w) != md {
        return fail(format!(
            "形状越界或包围盒标签错误：{packed:016x}，bbox={w}x{h}"
        ));
    }
    if flood(mask & mask.wrapping_neg(), mask) != mask {
        return fail(format!("前景非四连通：{packed:016x}"));
    }
    if w < h || rotations.half(mask, w, h) < mask {
        return fail(format!("不是四旋转最小代表：{packed:016x}"));
    }
    if w == h {
        let quarter = rotations.quarter(mask, h);
        if quarter < mask || rotations.half(quarter, h, w) < mask {
            return fail(format!("不是四旋转最小代表：{packed:016x}"));
        }
    }
    // 整个 8x8 边框都连通到外部；形状平移一格后，未到达的背景恰为洞。
    let background = !(mask << 9);
    let actual_hole = usize::from(flood(1, background) != background);
    if actual_hole != hole {
        return fail(format!("洞分类错误：{packed:016x}，实际={actual_hole}"));
    }
    Ok((hole, md, mask))
}

fn verify_bucket(path: PathBuf, max_n: usize) -> Result<Counts> {
    let size = fs::metadata(&path)?.len();
    if size % 8 != 0 || size > MAX_BUCKET_BYTES {
        return fail(format!(
            "磁盘桶大小无效或超过 20 MiB 上限：{} ({size} B)",
            path.display()
        ));
    }
    let mut file = File::open(&path)?;
    let mut values = Vec::new();
    values.try_reserve_exact((size / 8) as usize)?;
    let mut bytes = [0u8; 64 * 1024];
    let mut actual = 0u64;
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        actual += n as u64;
        if n % 8 != 0 {
            // 不假设 read() 总在记录边界返回：剩余字节用 read_exact 补足。
            let full = n & !7;
            for chunk in bytes[..full].chunks_exact(8) {
                values.push(u64::from_le_bytes(chunk.try_into().unwrap()));
            }
            let mut tail = [0u8; 8];
            tail[..n - full].copy_from_slice(&bytes[full..n]);
            file.read_exact(&mut tail[n - full..])?;
            actual += (8 - (n - full)) as u64;
            values.push(u64::from_le_bytes(tail));
        } else {
            for chunk in bytes[..n].chunks_exact(8) {
                values.push(u64::from_le_bytes(chunk.try_into().unwrap()));
            }
        }
    }
    if actual != size || values.len() as u64 != size / 8 {
        return fail(format!("磁盘桶短读或读数变化：{}", path.display()));
    }
    values.sort_unstable_by_key(|x| x & MASK_BITS);
    let rotations = Rotations::new();
    let mut counts = Counts::default();
    let mut previous = None;
    for packed in values {
        let (hole, md, mask) = validate(packed, max_n, &rotations)?;
        if previous == Some(mask) {
            return fail(format!("全局重复掩码：{mask:016x}，桶 {}", path.display()));
        }
        previous = Some(mask);
        counts.0[hole][md] += 1;
    }
    Ok(counts)
}

fn run(dataset: &Path, scratch: &Path, max_n: usize) -> Result<()> {
    if !(1..=6).contains(&max_n) {
        return fail("n 必须在 1..=6");
    }
    if !dataset.is_dir() {
        return fail(format!("数据集目录不存在：{}", dataset.display()));
    }
    check_layout(dataset, max_n)?;
    let mut writers = prepare_scratch(scratch)?;
    let mut distributed = 0u64;
    for (hole, category) in ["no_holes", "with_holes"].iter().enumerate() {
        for md in 1..=max_n {
            let dir = dataset.join(category).join(format!("n{md:02}_fixed"));
            for path in shape_files(&dir)? {
                distributed += distribute_file(&path, md, hole, &mut writers)?;
            }
        }
    }
    for writer in &mut writers {
        writer.flush()?;
    }
    drop(writers); // 所有输入写句柄关闭后才能开始排序。
    eprintln!("已分桶 {distributed} 条；开始核验 256 个桶（最多四线程）。");
    let mut total = Counts::default();
    for start in (0..BUCKETS).step_by(4) {
        let batch = thread::scope(|scope| -> Result<Counts> {
            let mut handles = Vec::new();
            for i in start..(start + 4).min(BUCKETS) {
                handles.push(scope.spawn(move || verify_bucket(bucket_path(scratch, i), max_n)));
            }
            let mut combined = Counts::default();
            for handle in handles {
                combined.add(
                    &handle
                        .join()
                        .map_err(|_| io::Error::other("桶核验线程崩溃"))??,
                );
            }
            Ok(combined)
        })?;
        total.add(&batch);
        if (start + 4) % 64 == 0 {
            eprintln!("已核验 {} / 256 桶", start + 4);
        }
    }
    let mut counted = 0u64;
    for md in 1..=max_n {
        let expected_no = NO_HOLES[md] - NO_HOLES[md - 1];
        let expected_yes = WITH_HOLES[md] - WITH_HOLES[md - 1];
        let expected_total = TOTAL[md] - TOTAL[md - 1];
        if expected_no + expected_yes != expected_total
            || total.0[0][md] != expected_no
            || total.0[1][md] != expected_yes
        {
            return fail(format!("n{md:02} 分类数错误：实际无洞={} 有洞={}，预期无洞={expected_no} 有洞={expected_yes}", total.0[0][md], total.0[1][md]));
        }
        counted += total.0[0][md] + total.0[1][md];
    }
    if counted != distributed || counted != TOTAL[max_n] {
        return fail(format!(
            "记录总数不符：分桶={distributed}，验证={counted}，预期={}",
            TOTAL[max_n]
        ));
    }
    println!(
        "PASS n={max_n} records={counted} no_holes={} with_holes={} buckets=256 exact_keys=true",
        NO_HOLES[max_n], WITH_HOLES[max_n]
    );
    Ok(())
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 4 {
        eprintln!("用法：verify_exact <dataset-root> <scratch-new-dir> <n:1..6>");
        std::process::exit(2);
    }
    let Some(n) = args[3].to_string_lossy().parse::<usize>().ok() else {
        eprintln!("n 必须是 1..=6 的整数");
        std::process::exit(2);
    };
    if let Err(error) = run(Path::new(&args[1]), Path::new(&args[2]), n) {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}
