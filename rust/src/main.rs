//! 精确计数使用前沿 DP；完整数据集使用前沿状态图回溯。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
mod bit_utils;
mod burnside;
mod export;
mod fixed;
mod frontier_export;
mod hashset;
mod redelmeier_export;
mod symmetric;
mod transfer;
mod types;
#[cfg(test)]
mod validation;

use clap::{Parser, ValueEnum};
use std::{io, path::PathBuf, process::ExitCode, sync::Arc, time::Instant};
use types::MAX_N;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Algorithm {
    Auto,
    Transfer,
    Bfs,
    Canonical,
    Frontier,
    Redelmeier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum CompressionBackend {
    Native,
    #[value(name = "7z")]
    SevenZip,
}

fn parse_n(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=MAX_N).contains(n))
        .ok_or_else(|| format!("n 必须在1..={MAX_N}范围内"))
}

#[derive(Parser, Debug)]
#[command(
    name = "room-count",
    about = "精确统计 one-sided 房间形状；n 必须显式指定"
)]
struct Args {
    #[arg(value_parser=parse_n)]
    n: usize,
    #[arg(short, long)]
    verbose: bool,
    /// auto：纯计数使用前沿DP，导出使用前沿状态图回溯
    #[arg(long, value_enum, default_value = "auto")]
    algorithm: Algorithm,
    /// 兼容旧参数，使用已经验证的新前沿DP
    #[arg(long,hide=true,conflicts_with_all=["export","algorithm"])]
    jensen: bool,
    /// 导出全部one-sided最小旋转代表
    #[arg(long)]
    export: bool,
    /// 必须为不存在或空目录，拒绝覆盖已有数据集
    #[arg(long, default_value = "output")]
    export_dir: PathBuf,
    /// 保留裸bin，跳过ZIP压缩
    #[arg(long, requires = "export")]
    no_compress: bool,
    /// ZIP压缩等级0..9，默认1优先吞吐
    #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u8).range(0..=9))]
    compression_level: u8,
    /// native：流式ZIP；7z：先写裸文件再调用外部7-Zip
    #[arg(long, value_enum, default_value = "native")]
    compression_backend: CompressionBackend,
}

fn run(args: Args) -> io::Result<()> {
    let total_start = Instant::now();
    let algorithm = match args.algorithm {
        Algorithm::Auto if args.export => Algorithm::Frontier,
        Algorithm::Auto => Algorithm::Transfer,
        algorithm => algorithm,
    };
    if args.export && algorithm == Algorithm::Transfer {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "前沿DP只计数；导出请使用 --algorithm frontier 或 auto",
        ));
    }
    let title = match algorithm {
        Algorithm::Transfer => "前沿 DP + 对称轨道 + Burnside",
        Algorithm::Canonical => "并行 One-sided BFS + 轨道计数",
        Algorithm::Frontier => "前沿状态图回溯导出",
        Algorithm::Redelmeier => "Redelmeier 逐形状遍历",
        _ => "并行 Fixed BFS + Burnside",
    };
    println!("多联骨牌房间计数 — {title}\nn={}", args.n);
    let manager = if args.export {
        Some(Arc::new(
            if !args.no_compress && args.compression_backend == CompressionBackend::Native {
                export::ExportManager::new_zip(&args.export_dir, args.compression_level)?
            } else {
                export::ExportManager::new(&args.export_dir)?
            },
        ))
    } else {
        None
    };
    let compute_start = Instant::now();
    let results = match algorithm {
        Algorithm::Transfer => transfer::enumerate_transfer(args.n, args.verbose),
        Algorithm::Frontier => {
            frontier_export::enumerate_frontier(args.n, args.verbose, manager.clone())?
        }
        Algorithm::Redelmeier => {
            redelmeier_export::enumerate_redelmeier(args.n, args.verbose, manager.clone())?
        }
        Algorithm::Bfs | Algorithm::Canonical => {
            let (fixed, s90, s180) = if algorithm == Algorithm::Canonical {
                fixed::enumerate_canonical(args.n, args.verbose, manager.clone())?
            } else {
                fixed::enumerate_fixed_with_symmetry(args.n, args.verbose, manager.clone())?
            };
            if args.verbose {
                for ((f, a), b) in fixed.iter().zip(&s90).zip(&s180) {
                    eprintln!(
                        "  n={} Fixed={} Sym90={} Sym180={}",
                        f.n, f.total, a.total, b.total
                    );
                }
            }
            burnside::apply_burnside(&fixed, &s90, &s180)
        }
        Algorithm::Auto => unreachable!(),
    };
    let compute_elapsed = compute_start.elapsed();
    if !burnside::verify_results(&results) {
        return Err(io::Error::other("已知计数校验失败"));
    }
    for r in &results {
        println!("{r}");
    }
    println!("算法耗时: {:.6}s", compute_elapsed.as_secs_f64());
    if let Some(manager) = manager {
        if !args.no_compress {
            let compress_start = Instant::now();
            match args.compression_backend {
                CompressionBackend::Native => {
                    manager.finish_zip()?;
                    println!(
                        "ZIP收尾耗时: {:.6}s（流式压缩已计入算法耗时）",
                        compress_start.elapsed().as_secs_f64()
                    );
                }
                CompressionBackend::SevenZip => {
                    manager.compress_7z(args.compression_level)?;
                    println!("压缩耗时: {:.6}s", compress_start.elapsed().as_secs_f64());
                }
            }
        }
        let count = results.last().unwrap().total;
        let streams = manager.stream_manifest(!args.no_compress, count)?;
        let dataset_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let manifest = format!(
            concat!("{{\n  \"format_version\": 2,\n  \"dataset_id\": \"{}-{}\",\n",
            "  \"max_n\": {},\n  \"count\": {},\n  \"equivalence\": \"one-sided\",\n",
            "  \"encoding\": \"u64-le-stride8\",\n  \"representative\": \"minimum-rotation\",\n",
            "  \"category_dimension\": \"exact-max-bounding-box\",\n",
            "  \"storage_layout\": \"classified-single-copy\",\n",
            "  \"ordering\": \"category-major-parallel-unspecified\",\n",
            "  \"streams\": [\n{}\n  ],\n  \"complete\": true\n}}\n"),
            dataset_id,
            std::process::id(),
            args.n,
            count,
            streams
        );
        std::fs::write(args.export_dir.join("dataset.json"), manifest)?;
        println!(
            "导出目录: {}（新数据集，旧 global_index 不可复用）",
            args.export_dir.display()
        );
    }
    println!("总耗时: {:.6}s", total_start.elapsed().as_secs_f64());
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn no_implicit_large_run() {
        assert!(Args::try_parse_from(["room-count"]).is_err());
    }
}
