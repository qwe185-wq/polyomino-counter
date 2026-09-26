//! 动态尺寸路线的共享类型；不以机器整数截断形状或计数。
use num_bigint::BigUint;
use std::{fmt, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomCount {
    pub n: usize,
    pub total: BigUint,
    pub no_hole: BigUint,
    pub has_hole: BigUint,
}

impl fmt::Display for RoomCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "n={}: total={}, no_hole={}, has_hole={}",
            self.n, self.total, self.no_hole, self.has_hole
        )
    }
}

/// 只拒绝零和机器地址/尺寸算术不能表示的输入，不设经验资源门槛。
pub fn layout(n: usize) -> io::Result<(usize, usize)> {
    let error = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "n必须为正整数，且位图尺寸不能溢出机器地址范围",
        )
    };
    if n == 0 {
        return Err(error());
    }
    let stride = n.max(8);
    let bits = n.checked_mul(stride).ok_or_else(error)?;
    let padded = n
        .checked_add(2)
        .and_then(|x| x.checked_mul(x))
        .ok_or_else(error)?;
    if padded > isize::MAX as usize {
        return Err(error());
    }
    Ok((stride, bits.div_ceil(8).max(8)))
}

pub fn verify_results(results: &[RoomCount]) -> bool {
    let known = [
        (1u64, 1u64, 0u64),
        (4, 4, 0),
        (46, 44, 2),
        (2404, 1899, 505),
        (520818, 267976, 252842),
        (410964612, 112877832, 298086780),
        (1185652433093, 144608553854, 1041043879239),
    ];
    results.iter().enumerate().all(|(i, r)| {
        r.n == i + 1
            && r.total == &r.no_hole + &r.has_hole
            && known.get(i).is_none_or(|&(total, no_hole, has_hole)| {
                r.total == BigUint::from(total)
                    && r.no_hole == BigUint::from(no_hole)
                    && r.has_hole == BigUint::from(has_hole)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dynamic_sizes_and_counts_cross_machine_word() {
        assert_eq!(layout(7).unwrap(), (8, 8));
        assert_eq!(layout(8).unwrap(), (8, 8));
        assert_eq!(layout(9).unwrap(), (9, 11));
        assert_eq!(layout(17).unwrap(), (17, 37));
        assert!(layout(0).is_err());
        assert!(layout(usize::MAX).is_err());
        let count = BigUint::from(u64::MAX) + 1u32;
        assert_eq!(count.to_string(), "18446744073709551616");
    }
}
