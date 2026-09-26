//! 精确计数使用前沿 DP；完整数据集使用前沿状态图回溯。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
mod bit_utils;
mod burnside;
mod count_cache;
mod dynamic;
mod dynamic_bitset;
mod dynamic_export;
mod dynamic_frontier;
mod dynamic_transfer;
mod export;
mod fixed;
mod frontier_export;
mod hashset;
mod redelmeier_export;
mod symmetric;
mod symmetric_quotient;
mod symmetric_transfer;
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
    let n = value
        .parse::<usize>()
        .map_err(|_| "n必须为正整数".to_owned())?;
    dynamic::layout(n).map_err(|error| error.to_string())?;
    Ok(n)
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
    /// 输出纯计数的分阶段/逐层统计到 stderr
    #[arg(long, conflicts_with = "export")]
    profile_count: bool,
    /// 复用已完成的精确计数分项；不保存未完成的前沿状态
    #[arg(long, conflicts_with = "export")]
    count_cache_dir: Option<PathBuf>,
    /// 小内存计数分项的并行线程数；大型 DP 单独运行
    #[arg(long, conflicts_with = "export")]
    count_threads: Option<usize>,
    /// 并行调度的总内存预算 MiB（硬限制由外部运行器执行）
    #[arg(long, conflicts_with = "export")]
    count_memory_mib: Option<u64>,
    /// 对称固定集内核：auto 自动选择；frontier/quotient 用于对照
    #[arg(long, value_enum, conflicts_with = "export")]
    symmetry_engine: Option<dynamic_transfer::SymmetryEngine>,
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
    if args.count_threads == Some(0) || args.count_memory_mib == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "计数线程数和内存预算必须为正数",
        ));
    }
    let count_options = args.profile_count
        || args.count_cache_dir.is_some()
        || args.count_threads.is_some()
        || args.count_memory_mib.is_some()
        || args.symmetry_engine.is_some();
    if count_options
        && (args.export || !matches!(args.algorithm, Algorithm::Auto | Algorithm::Transfer))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "计数选项只支持纯计数 auto/transfer",
        ));
    }
    if args.profile_count {
        std::env::set_var("ROOM_COUNT_PROFILE", "1");
    }
    if args.n > MAX_N || count_options {
        return run_dynamic(args);
    }
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

fn run_dynamic(args: Args) -> io::Result<()> {
    let start = Instant::now();
    let algorithm = match args.algorithm {
        Algorithm::Auto if args.export => Algorithm::Frontier,
        Algorithm::Auto => Algorithm::Transfer,
        algorithm => algorithm,
    };
    if !matches!(algorithm, Algorithm::Transfer | Algorithm::Frontier) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "bfs/canonical/redelmeier为n≤6的固定位图参考算法；动态尺寸请使用auto、transfer或frontier"));
    }
    if args.export && algorithm == Algorithm::Transfer {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "前沿DP只计数；导出请使用frontier或auto",
        ));
    }
    let mode = if args.no_compress {
        dynamic_export::Mode::Raw
    } else if args.compression_backend == CompressionBackend::Native {
        dynamic_export::Mode::Native(args.compression_level)
    } else {
        dynamic_export::Mode::SevenZip(args.compression_level)
    };
    let mut output = if args.export {
        Some(dynamic_export::Export::new(&args.export_dir, args.n, mode)?)
    } else {
        None
    };
    let title = if algorithm == Algorithm::Transfer {
        "动态前沿DP + 旋转轨道 + 任意精度Burnside"
    } else {
        "动态前沿状态图回溯导出"
    };
    println!("多联骨牌房间计数 — {title}\nn={}", args.n);
    let compute_start = Instant::now();
    let results = if algorithm == Algorithm::Transfer {
        dynamic_transfer::enumerate_transfer_with_options(
            args.n,
            args.verbose,
            &dynamic_transfer::CountOptions {
                cache_dir: args.count_cache_dir.clone(),
                symmetry_engine: args.symmetry_engine.unwrap_or_default(),
                threads: args.count_threads.unwrap_or(1),
                memory_budget_bytes: args
                    .count_memory_mib
                    .unwrap_or(4096)
                    .checked_mul(1024 * 1024)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "内存预算溢出"))?,
            },
        )?
    } else {
        dynamic_frontier::enumerate_frontier(args.n, args.verbose, &mut |mask, md, hole| {
            if let Some(output) = &mut output {
                output.write(mask, md, hole)
            } else {
                Ok(())
            }
        })?
    };
    let elapsed = compute_start.elapsed();
    if results.len() != args.n || !dynamic::verify_results(&results) {
        return Err(io::Error::other("动态计数一致性/已知结果校验失败"));
    }
    for result in &results {
        println!("{result}");
    }
    println!("算法耗时: {:.6}s", elapsed.as_secs_f64());
    if let Some(output) = &mut output {
        let finish_start = Instant::now();
        output.finish(&results.last().unwrap().total)?;
        let label = match mode {
            dynamic_export::Mode::Raw => "导出收尾耗时",
            dynamic_export::Mode::Native(_) => "ZIP收尾耗时",
            dynamic_export::Mode::SevenZip(_) => "压缩耗时",
        };
        println!("{label}: {:.6}s", finish_start.elapsed().as_secs_f64());
        if matches!(mode, dynamic_export::Mode::Native(_)) {
            println!("流式ZIP压缩已计入算法耗时");
        }
        println!(
            "导出目录: {}（v3动态位图；通过dataset.json读取）",
            args.export_dir.display()
        );
    }
    println!("总耗时: {:.6}s", start.elapsed().as_secs_f64());
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
    #[test]
    fn accepts_dynamic_dimensions_without_starting_enumeration() {
        for n in ["7", "8", "9", "17", "100"] {
            assert_eq!(
                Args::try_parse_from(["room-count", n])
                    .unwrap()
                    .n
                    .to_string(),
                n
            );
        }
        for n in ["0", "-1", "18446744073709551615"] {
            assert!(Args::try_parse_from(["room-count", n]).is_err());
        }
    }
}
