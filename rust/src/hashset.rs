//! 按代规模自适应的分片哈希集；每片串行插入，片间可并行。

use parking_lot::Mutex;
use rustc_hash::FxHashSet;
use std::sync::Arc;

/// 分片数量上限（实际数量按代规模取2的幂）。
const SHARD_BITS: usize = 10;
const SHARD_COUNT: usize = 1 << SHARD_BITS;

/// 单个分片
struct Shard {
    set: Mutex<FxHashSet<u64>>,
}

impl Shard {
    fn new(capacity: usize) -> Self {
        // 预分配但限制上限: 每分片最多 128K 条目 (~1MB)，超大代让哈希集自行扩展
        // 平衡预分配收益 (减少 rehash) 与峰值内存
        let cap = capacity.min(131_072);
        Self {
            set: Mutex::new(FxHashSet::with_capacity_and_hasher(
                cap,
                rustc_hash::FxBuildHasher,
            )),
        }
    }
}

/// 分片并发哈希集
///
/// `Arc` 包裹以支持跨线程共享，`Send + Sync` 由编译器保证。
pub struct ShardedHashSet {
    shards: Vec<Arc<Shard>>,
}

impl ShardedHashSet {
    /// 创建新的分片哈希集
    ///
    /// `total_capacity`: 预估总容量，均匀分配到各分片。
    pub fn new(total_capacity: usize) -> Self {
        let count = total_capacity
            .div_ceil(256)
            .next_power_of_two()
            .clamp(1, SHARD_COUNT);
        let per_shard = total_capacity.div_ceil(count);
        let mut shards = Vec::with_capacity(count);
        for _ in 0..count {
            shards.push(Arc::new(Shard::new(per_shard)));
        }
        Self { shards }
    }

    /// splitmix64 变体 — 高质量 64 位哈希
    #[inline]
    fn hash(key: u64) -> u64 {
        let mut x = key;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
        x ^ (x >> 31)
    }

    /// 获取 key 对应的分片索引
    #[inline]
    fn shard_idx(&self, key: u64) -> usize {
        (Self::hash(key) as usize) & (self.shards.len() - 1)
    }

    /// 锁内查询，仅测试和辅助接口使用。
    ///
    /// 读写均使用同一个分片锁。
    #[inline]
    pub fn contains(&self, key: u64) -> bool {
        let idx = self.shard_idx(key);
        self.shards[idx].set.lock().contains(&key)
    }

    /// 插入 key（仅锁单个分片）
    ///
    /// 返回 `true` 表示新插入，`false` 表示已存在。
    #[inline]
    pub fn insert(&self, key: u64) -> bool {
        let idx = self.shard_idx(key);
        self.shards[idx].set.lock().insert(key)
    }

    /// 获取元素总数（遍历所有分片，仅用于统计）
    pub fn total_count(&self) -> usize {
        self.shards.iter().map(|s| s.set.lock().len()).sum()
    }

    /// 批量插入 4 个旋转方向（用于偶尔需要的 One-sided 规范化）
    #[inline]
    pub fn insert_orientations(&self, orients: &[u64; 4]) {
        for &o in orients.iter() {
            self.insert(o);
        }
    }

    /// 检查并插入 — 先查后插的原子化操作
    ///
    /// 在同一分片锁内完成查+插，避免 TOCTOU 竞态。
    /// 返回 `true` 表示是新 key 并已插入。
    #[inline]
    pub fn check_and_insert(&self, key: u64) -> bool {
        let idx = self.shard_idx(key);
        let mut set = self.shards[idx].set.lock();
        set.insert(key)
    }
}

impl Clone for ShardedHashSet {
    fn clone(&self) -> Self {
        Self {
            shards: self.shards.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_basic_operations() {
        let hs = ShardedHashSet::new(1024);
        assert!(hs.insert(42));
        assert!(hs.contains(42));
        assert!(!hs.insert(42)); // 重复插入失败
        assert!(!hs.contains(99));
    }

    #[test]
    fn test_concurrent_inserts() {
        let hs = Arc::new(ShardedHashSet::new(100000));
        let mut handles = vec![];

        for t in 0..8 {
            let hs = hs.clone();
            handles.push(thread::spawn(move || {
                let base = t * 10000;
                let mut inserted = 0usize;
                for i in 0..10000u64 {
                    if hs.insert(base as u64 + i) {
                        inserted += 1;
                    }
                }
                inserted
            }));
        }

        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 80000);
        assert_eq!(hs.total_count(), 80000);
    }

    #[test]
    fn test_concurrent_check_and_insert() {
        let hs = Arc::new(ShardedHashSet::new(1024));
        let mut handles = vec![];

        // All threads try to insert the same keys
        for _ in 0..8 {
            let hs = hs.clone();
            handles.push(thread::spawn(move || {
                let mut count = 0usize;
                for i in 0..1000u64 {
                    if hs.check_and_insert(i) {
                        count += 1;
                    }
                }
                count
            }));
        }

        let results: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        // Only one set of insertions should succeed
        let total: usize = results.iter().sum();
        assert_eq!(hs.total_count(), 1000);
        // 同一个key恰好一次成功，不能仅检查下界。
        assert_eq!(total, 1000);
    }
}
