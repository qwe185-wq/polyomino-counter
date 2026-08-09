//! 多联骨牌房间计数 — Rust 实现
//!
//! ## 算法
//!
//! 1. **Fixed BFS + Burnside** (主算法): 逐代生成 + 内联对称检测 + 分片哈希
//! 2. **Redelmeier DFS** (WIP): Untried Set + 半平面约束 (n=1-2 已验证)

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod bit_utils;
mod burnside;
mod fixed;
mod hashset;
mod jensen;
mod redelmeier;
mod symmetric;
mod types;

use crate::burnside::apply_burnside;
use crate::fixed::enumerate_fixed_with_symmetry;
use crate::types::*;
use clap::Parser;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "room-count")]
struct Args {
    #[arg(default_value_t = MAX_N)]
    n: usize,
    #[arg(short, long)]
    verbose: bool,
    /// 使用 Redelmeier DFS (实验性)
    #[arg(long)]
    dfs: bool,
    /// 使用 Jensen 转移矩阵法
    #[arg(long)]
    jensen: bool,
}

fn main() {
    let args = Args::parse();
    let max_n = args.n.clamp(1, MAX_N);

    println!("═══════════════════════════════════════════════════════════");
    if args.dfs {
        println!("  多联骨牌房间计数 — Redelmeier DFS (实验性)");
    } else {
        println!("  多联骨牌房间计数 — Fixed BFS + Burnside 引理");
    }
    println!("  n={}  mimalloc  1024分片并发哈希", max_n);
    println!("═══════════════════════════════════════════════════════════\n");

    let total_start = Instant::now();

    let results = if args.jensen {
        println!("── Jensen 转移矩阵法 ──");
        crate::jensen::enumerate_jensen(max_n, args.verbose)
    } else if args.dfs {
        println!("── Redelmeier DFS 枚举 ──");
        crate::redelmeier::enumerate_redelmeier(max_n, args.verbose)
    } else {
        println!("── Fixed BFS 枚举 + 对称性检测 ──");
        let (fixed, sym90, sym180) =
            enumerate_fixed_with_symmetry(max_n, args.verbose);
        let os = apply_burnside(&fixed, &sym90, &sym180);

        if args.verbose {
            println!("\n  Fixed + Sym 分解:");
            for i in 0..max_n {
                let f = &fixed[i]; let s90 = &sym90[i]; let s180 = &sym180[i];
                let sum = f.total + 2*s90.total + s180.total;
                println!("  n={}: Fixed={} Sym90={} Sym180={} sum={} sum%4={} OS={}",
                         f.n, f.total, s90.total, s180.total, sum, sum%4, os[i].total);
            }
        }
        os
    };

    let total_elapsed = total_start.elapsed();

    println!();
    println!("  ╔══════════════════════════════════════════════════════╗");
    println!("  ║         One-sided Polyomino 枚举结果                  ║");
    println!("  ╠═════╤══════════╤══════════╤══════════╤═══════════════╣");
    println!("  ║  n  │  总房间数  │  无洞     │  有洞     │  验证        ║");
    println!("  ╟─────┼──────────┼──────────┼──────────┼───────────────╢");

    let known: &[(usize, u64, u64, u64)] = &[
        (1, 1, 1, 0), (2, 4, 4, 0), (3, 46, 44, 2),
        (4, 2404, 1899, 505), (5, 520818, 267976, 252842),
    ];

    let mut all_ok = true;
    for r in &results {
        let (e_t, e_n, e_h) = if r.n <= known.len() {
            (known[r.n-1].1, known[r.n-1].2, known[r.n-1].3)
        } else { (0,0,0) };
        let ok = r.n > known.len() || (r.total == e_t && r.no_hole == e_n && r.has_hole == e_h);
        if !ok { all_ok = false; }
        println!("  ║ {:-3} │ {:>8} │ {:>8} │ {:>8} │ {:>11} ║",
                 r.n, r.total, r.no_hole, r.has_hole,
                 if r.n > known.len() { "新结果" } else if ok { "✓" } else { "✗" });
    }
    println!("  ╚═════╧══════════╧══════════╧══════════╧═══════════════╝");

    println!("\n  ⏱ 总耗时: {:.3}s", total_elapsed.as_secs_f64());
    if all_ok && max_n <= 5 { println!("\n  ✓ 所有已知结果验证通过！"); }
}
