#[path = "../transfer.rs"]
mod transfer;
#[path = "../types.rs"]
mod types;

fn main() {
    // 探针不提供默认的大规模计算入口。
    let start = std::time::Instant::now();
    let results = transfer::enumerate_transfer(5, false);
    let elapsed = start.elapsed();
    for result in results {
        println!("{result}");
    }
    eprintln!("enumeration_ms={:.3}", elapsed.as_secs_f64() * 1000.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sided_counts_through_five() {
        let expected = [
            (1, 1, 0),
            (4, 4, 0),
            (46, 44, 2),
            (2404, 1899, 505),
            (520818, 267976, 252842),
        ];
        let actual = transfer::enumerate_transfer(5, false);
        assert_eq!(actual.len(), expected.len());
        for (result, &(total, no_hole, has_hole)) in actual.iter().zip(expected.iter()) {
            assert_eq!(
                (result.total, result.no_hole, result.has_hole),
                (total, no_hole, has_hole),
                "n={}",
                result.n
            );
        }
    }
}
