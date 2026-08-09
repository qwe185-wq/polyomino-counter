//! Fixed mask 去重验证工具
//!
//! 两阶段算法：
//!   1. 分片：按 hash 前缀将 mask 分配到 N 个临时文件
//!   2. 验证：对每个分片并行排序，检查相邻重复
//!
//! 用法:
//!   dedup_check <输入.bin>
//!   dedup_check <输入.zip>     (自动调用 7-zip 解压)
//!   dedup_check --shards 512 <输入.bin>

use clap::Parser;
use rayon::prelude::*;
use rustc_hash::FxHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

// ================================================================
// CLI
// ================================================================

#[derive(Parser, Debug)]
#[command(name = "dedup_check")]
struct Args {
    /// 输入文件 (.bin 或 .zip)
    input: PathBuf,

    /// 分片数量 (默认 256，2的幂)
    #[arg(long, default_value_t = 256)]
    shards: usize,

    /// 临时目录 (默认系统临时目录)
    #[arg(long)]
    tmp_dir: Option<PathBuf>,

    /// 详细输出
    #[arg(short, long)]
    verbose: bool,
}

// ================================================================
// 主逻辑
// ================================================================

fn main() {
    let args = Args::parse();

    // 验证分片数是 2 的幂
    if !args.shards.is_power_of_two() || args.shards == 0 {
        eprintln!("错误: --shards 必须是 2 的幂");
        return;
    }

    let shard_mask = (args.shards - 1) as u64;

    // 确定输入文件（如果是 .zip 则先解压）
    let bin_paths: Vec<PathBuf> = if args.input.extension().map_or(false, |e| e == "zip") {
        let tmp = args
            .tmp_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("dedup_check"));
        fs::create_dir_all(&tmp).unwrap();
        let extract_dir = tmp.join("extracted");
        fs::create_dir_all(&extract_dir).unwrap();

        println!("解压 {} ...", args.input.display());
        let zip_exe = "C:/Program Files/7-Zip/7z.exe";
        let status = Command::new(zip_exe)
            .args([
                "x",
                "-y",
                &format!("-o{}", extract_dir.display()),
                args.input.to_str().unwrap(),
            ])
            .status()
            .expect("7-zip 执行失败");
        if !status.success() {
            eprintln!("解压失败");
            return;
        }
        // 找到所有 .bin 文件
        let mut bins: Vec<PathBuf> = find_bin_files(&extract_dir);
        bins.sort();
        if bins.is_empty() {
            eprintln!("解压后未找到 .bin 文件");
            return;
        }
        bins
    } else if args.input.is_dir() {
        let mut bins: Vec<PathBuf> = find_bin_files(&args.input);
        bins.sort();
        if bins.is_empty() {
            eprintln!("目录中未找到 .bin 文件: {}", args.input.display());
            return;
        }
        bins
    } else {
        vec![args.input.clone()]
    };

    // 验证所有文件存在
    for p in &bin_paths {
        if !p.exists() {
            eprintln!("错误: 文件不存在: {}", p.display());
            return;
        }
    }

    let total_file_size: u64 = bin_paths.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
    let n_masks = total_file_size / 8;
    for p in &bin_paths {
        let sz = fs::metadata(p).unwrap().len();
        if sz % 8 != 0 {
            eprintln!(
                "警告: {} 大小 {} 不是 8 的倍数，忽略末尾字节",
                p.display(),
                sz % 8
            );
        }
    }

    println!(
        "输入: {} 个文件 ({:.2} GB, {} masks)",
        bin_paths.len(),
        total_file_size as f64 / 1e9,
        n_masks
    );
    println!("分片: {} 个", args.shards);

    let total_start = Instant::now();

    // === 阶段 1: 分片 ===
    let tmp_base = args
        .tmp_dir
        .unwrap_or_else(|| std::env::temp_dir().join("dedup_check"));
    fs::create_dir_all(&tmp_base).unwrap();

    let shard_files: Vec<PathBuf> = (0..args.shards)
        .map(|i| tmp_base.join(format!("shard_{:04x}.bin", i)))
        .collect();

    println!("\n── 阶段 1: 分片 ──");
    let phase1_start = Instant::now();

    {
        let mut shard_writers: Vec<BufWriter<File>> = shard_files
            .iter()
            .map(|p| BufWriter::with_capacity(1024 * 1024, File::create(p).unwrap()))
            .collect();

        // 每个分片一个计数器
        let mut shard_counts = vec![0u64; args.shards];

        let mut total_processed: u64 = 0;

        for (fi, bin_path) in bin_paths.iter().enumerate() {
            let fsize = fs::metadata(bin_path).unwrap().len();
            if args.verbose && bin_paths.len() > 1 {
                eprintln!("  [{}/{}] {} ({:.1} MB)", fi + 1, bin_paths.len(),
                    bin_path.file_name().unwrap().to_string_lossy(), fsize as f64 / 1e6);
            }

            let mut reader = BufReader::with_capacity(64 * 1024 * 1024, File::open(bin_path).unwrap());
            let mut buf = vec![0u8; 64 * 1024 * 1024];

            loop {
                let n = reader.read(&mut buf).unwrap();
                if n == 0 { break; }
                let chunk = &buf[..n - (n % 8)];
                let _n_masks_in_chunk = chunk.len() / 8;

                let chunk_shards: Vec<Vec<u8>> = chunk
                    .par_chunks(8)
                    .fold(
                        || (0..args.shards).map(|_| Vec::with_capacity(1024)).collect::<Vec<Vec<u8>>>(),
                        |mut local, mask_bytes| {
                            let mask = u64::from_le_bytes(mask_bytes.try_into().unwrap());
                            let mut hasher = FxHasher::default();
                            mask.hash(&mut hasher);
                            let h = hasher.finish();
                            let shard = (h & shard_mask) as usize;
                            local[shard].extend_from_slice(mask_bytes);
                            local
                        },
                    )
                    .reduce(
                        || (0..args.shards).map(|_| Vec::new()).collect(),
                        |mut a, b| { for i in 0..args.shards { a[i].extend_from_slice(&b[i]); } a },
                    );

                for i in 0..args.shards {
                    if !chunk_shards[i].is_empty() {
                        shard_writers[i].write_all(&chunk_shards[i]).unwrap();
                        shard_counts[i] += (chunk_shards[i].len() / 8) as u64;
                    }
                }

                total_processed += chunk.len() as u64;
                if args.verbose && bin_paths.len() == 1 {
                    eprintln!("  progress: {:.1}% ({:.2} GB)",
                        total_processed as f64 / total_file_size as f64 * 100.0,
                        total_processed as f64 / 1e9);
                }
            }
        }

        // 关闭所有 writer
        for w in shard_writers.iter_mut() {
            w.flush().unwrap();
        }
        drop(shard_writers);

        let phase1_elapsed = phase1_start.elapsed();
        println!(
            "  分片完成: {:.2}s ({:.0} GB/s)",
            phase1_elapsed.as_secs_f64(),
            total_file_size as f64 / phase1_elapsed.as_secs_f64() / 1e9
        );

        if args.verbose {
            let non_empty = shard_counts.iter().filter(|&&c| c > 0).count();
            let max_count = shard_counts.iter().max().unwrap();
            let min_count = shard_counts.iter().filter(|&&c| c > 0).min().unwrap_or(&0);
            println!("  非空分片: {} / {}", non_empty, args.shards);
            println!("  分片大小: min={} max={}", min_count, max_count);
        }
    }

    // === 阶段 2: 验证每个分片内的重复 ===
    println!("\n── 阶段 2: 分片内去重验证 ──");
    let phase2_start = Instant::now();

    let duplicates: Vec<(u64, usize)> = shard_files
        .par_iter()
        .enumerate()
        .filter_map(|(shard_idx, shard_path)| {
            let shard_size = fs::metadata(shard_path).unwrap().len();
            if shard_size == 0 {
                return None;
            }

            let n = shard_size as usize / 8;
            let mut masks = Vec::with_capacity(n);

            let mut reader =
                BufReader::with_capacity(8 * 1024 * 1024, File::open(shard_path).unwrap());
            let mut buf = [0u8; 8];
            while reader.read_exact(&mut buf).is_ok() {
                masks.push(u64::from_le_bytes(buf));
            }

            // 排序
            masks.par_sort_unstable();

            // 扫描相邻重复
            let mut dups: Vec<(u64, usize)> = Vec::new();
            for w in masks.windows(2) {
                if w[0] == w[1] {
                    dups.push((w[0], shard_idx));
                }
            }

            // 清理分片文件
            let _ = fs::remove_file(shard_path);

            if dups.is_empty() {
                None
            } else {
                Some(dups)
            }
        })
        .flatten()
        .collect();

    // 清理临时目录
    let _ = fs::remove_dir_all(&tmp_base);

    let phase2_elapsed = phase2_start.elapsed();
    let total_elapsed = total_start.elapsed();

    println!("  验证完成: {:.2}s", phase2_elapsed.as_secs_f64());
    println!("\n──────────────────────────────────────");
    if duplicates.is_empty() {
        println!("  ✓ 未发现重复 — {} masks 全部唯一", n_masks);
    } else {
        println!("  ✗ 发现 {} 个重复!", duplicates.len());
        for &(mask, shard) in duplicates.iter().take(100) {
            println!("    重复 mask: 0x{:016x} (分片 {})", mask, shard);
        }
        if duplicates.len() > 100 {
            println!("    ... 还有 {} 个重复未列出", duplicates.len() - 100);
        }
    }
    println!("  总耗时: {:.2}s", total_elapsed.as_secs_f64());
    println!("──────────────────────────────────────");

    if !duplicates.is_empty() {
        std::process::exit(1);
    }
}

/// 递归查找 .bin 文件
fn find_bin_files(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.extend(find_bin_files(&path));
            } else if path.extension().map_or(false, |e| e == "bin") {
                result.push(path);
            }
        }
    }
    result
}
