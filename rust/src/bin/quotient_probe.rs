//! 保序电压商图原型的独立计时入口。
#[path = "../symmetric_quotient.rs"]
mod symmetric_quotient;

use std::{env, io, time::Instant};

fn main() -> io::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: quotient_probe WIDTH HEIGHT half|quarter",
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
    let (no_hole, has_hole) = symmetric_quotient::count_fixed(width, height, quarter)?;
    let algorithm_seconds = start.elapsed().as_secs_f64();
    println!("no_hole={no_hole}, has_hole={has_hole}");
    println!("algorithm_seconds={algorithm_seconds:.9}");
    Ok(())
}
