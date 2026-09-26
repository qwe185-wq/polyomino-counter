//! 单个旋转固定集的有界性能测量入口，不导出形状。
#![allow(dead_code)]
#[path = "../count_cache.rs"]
mod count_cache;
#[path = "../dynamic.rs"]
mod dynamic;
#[path = "../dynamic_bitset.rs"]
mod dynamic_bitset;
#[path = "../dynamic_transfer.rs"]
mod dynamic_transfer;
#[path = "../symmetric_quotient.rs"]
mod symmetric_quotient;
#[path = "../symmetric_transfer.rs"]
mod symmetric_transfer;
#[cfg(test)]
#[path = "../transfer.rs"]
mod transfer;
#[cfg(test)]
#[path = "../types.rs"]
mod types;

use std::{env, io, time::Instant};

fn main() -> io::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: symmetry_probe WIDTH HEIGHT half|quarter [frontier|quotient|gray|auto]",
        )
    };
    if args.len() != 3 && args.len() != 4 {
        return Err(invalid());
    }
    let width = args[0].parse::<usize>().map_err(|_| invalid())?;
    let height = args[1].parse::<usize>().map_err(|_| invalid())?;
    let quarter = match args[2].as_str() {
        "half" => false,
        "quarter" => true,
        _ => return Err(invalid()),
    };
    let start = Instant::now();
    use dynamic_transfer::SymmetryEngine;
    let engine = match args.get(3).map(String::as_str).unwrap_or("frontier") {
        "frontier" => SymmetryEngine::Frontier,
        "quotient" => SymmetryEngine::Quotient,
        "gray" => SymmetryEngine::Gray,
        "auto" => SymmetryEngine::Auto,
        _ => return Err(invalid()),
    };
    let (no_hole, has_hole) = dynamic_transfer::probe_bbox(width, height, quarter, engine)?;
    let elapsed = start.elapsed();
    println!("no_hole={no_hole}, has_hole={has_hole}");
    println!("algorithm_seconds={:.9}", elapsed.as_secs_f64());
    Ok(())
}
