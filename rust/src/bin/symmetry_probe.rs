//! 单个旋转固定集的有界性能测量入口，不导出形状。
#[path = "../symmetric_transfer.rs"]
mod symmetric_transfer;

use std::{env, io, time::Instant};

fn main() -> io::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: symmetry_probe WIDTH HEIGHT half|quarter",
        )
    };
    if args.len() != 3 {
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
    let (no_hole, has_hole) = symmetric_transfer::count_fixed(width, height, quarter)?;
    println!("no_hole={no_hole}, has_hole={has_hole}");
    println!("algorithm_seconds={:.9}", start.elapsed().as_secs_f64());
    Ok(())
}
