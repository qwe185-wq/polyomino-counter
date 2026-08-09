//! 多联骨牌房间计数 — Rust 实现
//!
//! ## 算法
//!
//! 使用 Burnside 引理将 One-sided polyomino 问题分解为:
//! 1. **Fixed polyomino 枚举** (Redelmeier 1981 生长法)
//! 2. **内联对称检测** — 发现新形状时检测 90°/180° 旋转对称性
//! 3. **Burnside 组合**: One-sided = (Fixed + 2·Sym90 + Sym180) / 4
//!
//! ## 用法
//!
//! ```bash
//! cargo run --release -- [n] [--verbose] [--jensen]
//! ```

// mimalloc 全局分配器 — 减少碎片，优化并发分配性能
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod bit_utils;
mod burnside;
mod fixed;
mod hashset;
mod jensen;
mod symmetric;
mod types;

use crate::burnside::apply_burnside;
use crate::fixed::enumerate_fixed_with_symmetry;
use crate::jensen::enumerate_jensen;
use crate::types::*;
use clap::Parser;
use std::time::Instant;

/// 多联骨牌房间计数 — Burnside 引理 + Fixed Polyomino 枚举
#[derive(Parser, Debug)]
#[command(name = "room-count")]
#[command(version = "0.1.0")]
#[command(about = "枚举 n×n 网格中 One-sided polyomino（房间形状）")]
struct Args {
    /// 网格尺寸 (1..6)
    #[arg(default_value_t = MAX_N)]
    n: usize,

    /// 输出详情
    #[arg(short, long)]
    verbose: bool,

    /// 同时运行 Jensen 转移矩阵法验证
    #[arg(long)]
    jensen: bool,
}

fn main() {
    let args = Args::parse();
    let max_n = args.n.clamp(1, MAX_N);

    println!("═══════════════════════════════════════════════════════════");
    println!("  多联骨牌房间计数 — Rust 实现");
    println!("  Burnside 引理 + Fixed Polyomino (Redelmeier 1981)");
    println!("  n×n 正方形网格 (n={})", max_n);
    println!("  并行: rayon (work-stealing)");
    println!("  去重: 分片并发哈希集 (1024 shards + mimalloc)");
    println!("  对称: 内联 90°/180° 旋转检测");
    println!("═══════════════════════════════════════════════════════════");
    println!();

    let total_start = Instant::now();

    // ── Fixed 枚举 + 内联对称检测 ──
    println!("── Fixed Polyomino 枚举 + 对称性检测 ──");
    let fixed_start = Instant::now();
    let (fixed_results, sym90_results, sym180_results) =
        enumerate_fixed_with_symmetry(max_n, args.verbose);
    let fixed_elapsed = fixed_start.elapsed();
    println!(
        "  枚举完成，耗时 {:.3}s",
        fixed_elapsed.as_secs_f64()
    );

    // ── Burnside 引理组合 ──
    println!();
    println!("── Burnside 引理 — One-sided = (Fixed + 2·Sym90 + Sym180)/4 ──");
    let one_sided = apply_burnside(&fixed_results, &sym90_results, &sym180_results);

    let total_elapsed = total_start.elapsed();

    // ── 输出结果 ──
    println!();
    println!("  ╔══════════════════════════════════════════════════════╗");
    println!("  ║         One-sided Polyomino 枚举结果                  ║");
    println!("  ╠═════╤══════════╤══════════╤══════════╤═══════════════╣");
    println!("  ║  n  │  总房间数  │  无洞     │  有洞     │  验证        ║");
    println!("  ╟─────┼──────────┼──────────┼──────────┼───────────────╢");

    let known: &[(usize, u64, u64, u64)] = &[
        (1, 1, 1, 0),
        (2, 4, 4, 0),
        (3, 46, 44, 2),
        (4, 2404, 1899, 505),
        (5, 520818, 267976, 252842),
    ];

    let mut all_verified = true;
    for r in &one_sided {
        let (exp_total, exp_no, exp_hole) = if r.n <= known.len() {
            let k = &known[r.n - 1];
            (k.1, k.2, k.3)
        } else {
            (0, 0, 0)
        };

        let verified = if r.n <= known.len() {
            r.total == exp_total && r.no_hole == exp_no && r.has_hole == exp_hole
        } else {
            true
        };

        if !verified {
            all_verified = false;
        }

        let status = if r.n > known.len() {
            "待验证"
        } else if verified {
            "✓"
        } else {
            "✗"
        };

        println!(
            "  ║ {:-3} │ {:>8} │ {:>8} │ {:>8} │ {:>11} ║",
            r.n, r.total, r.no_hole, r.has_hole, status
        );
    }
    println!("  ╚═════╧══════════╧══════════╧══════════╧═══════════════╝");

    // 分解明细
    println!();
    println!("  ── 分解明细 ──");
    for i in 0..max_n {
        let f = &fixed_results[i];
        let s90 = &sym90_results[i];
        let s180 = &sym180_results[i];
        let os = &one_sided[i];

        let sum = f.total + 2 * s90.total + s180.total;
        let remainder = sum % 4;
        let flag = if remainder == 0 { "✓" } else { "⚠ NON-INT" };
        println!(
            "  n={}: Fixed={}  Sym90={}  Sym180={}  |  \
             sum={} → OS={}  [{} sum%4={}]",
            f.n, f.total, s90.total, s180.total, sum, os.total, flag, remainder
        );
    }

    println!();
    println!("  ⏱ 总耗时: {:.3}s", total_elapsed.as_secs_f64());

    // Jensen 验证（可选）
    if args.jensen {
        println!();
        println!("── Jensen 转移矩阵验证 ──");
        let jensen_start = Instant::now();
        let jensen_results = enumerate_jensen(max_n, args.verbose);
        let jensen_elapsed = jensen_start.elapsed();
        println!("  Jensen Fixed 结果:");
        for r in &jensen_results {
            println!("    n={}: total={}", r.n, r.total);
        }
        println!("  Jensen 耗时: {:.3}s", jensen_elapsed.as_secs_f64());
    }

    if all_verified && max_n <= 5 {
        println!();
        println!("  ✓ 所有已知结果验证通过！");
    } else if !all_verified {
        println!();
        println!("  ⚠ 结果与已知值不一致，需进一步调试。");
    }
}
