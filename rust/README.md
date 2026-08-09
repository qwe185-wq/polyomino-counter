# room-count — Rust 实现

**Burnside 引理 + Redelmeier 生长法** — n×n 网格 One-sided Polyomino 房间计数

## 算法

| 组件 | 方法 | 说明 |
|------|------|------|
| **核心枚举** | Redelmeier (1981) 生长法 | 逐格扩展 Fixed polyomino |
| **对称性** | 内联 90°/180° 旋转检测 | 发现新形状立即检测，零额外遍 |
| **去重** | 分片并发 FxHashSet (32 shards) | parking_lot RwLock + mimalloc |
| **并行** | Rayon work-stealing | 替代 C 版 OpenMP dynamic |
| **Burnside** | One-sided = (Fixed + 2·Sym90 + Sym180) / 4 |

## 编译运行

```bash
# 安装 Rust: https://rustup.rs

# 编译 (发布模式)
cd rust
cargo build --release

# 运行 n=5 (验证)
./target/release/room-count.exe 5 --verbose

# 运行 n=6 (约 10 分钟, 需 >8GB RAM)
./target/release/room-count.exe 6 --verbose

# Jensen 验证 (可选)
./target/release/room-count.exe 5 --jensen
```

## 验证结果

| n | 总房间数 | 无洞 | 有洞 | 验证 |
|---|---------|------|------|------|
| 1 | 1 | 1 | 0 | ✓ |
| 2 | 4 | 4 | 0 | ✓ |
| 3 | 46 | 44 | 2 | ✓ |
| 4 | 2,404 | 1,899 | 505 | ✓ |
| 5 | 520,818 | 267,976 | 252,842 | ✓ |
| **6** | **410,964,612** | **112,877,832** | **298,086,780** | **新结果** |

### n=6 分解

```
Fixed(n=6)   = 1,643,823,600
Sym90(n=6)   =           202
Sym180(n=6)  =        34,444
Burnside sum = 1,643,858,448  (sum%4=0 ✓)
One-sided    =   410,964,612
```

## 性能

| n | 耗时 | 峰值内存 | 速率 |
|---|------|----------|------|
| 5 | 0.22s | ~50 MB | ~20M shapes/s |
| 6 | 329s (5.5min) | ~8 GB | ~5M shapes/s |

## 与 C 版本对比

| 方面 | C (main) | Rust |
|------|----------|------|
| 算法 | One-sided 直接枚举 + orient_hs | Burnside → Fixed + 对称检测 |
| 规范化 | 4 旋转 + 4 平移 per 候选 | 仅 1 平移 per 候选 |
| 并发 | 单一 `omp critical` (69% 等锁) | 1024 分片 RwLock (<3% 争用) |
| 分配器 | 系统 malloc | mimalloc (减少碎片) |
| n=5 | 0.59s | 0.22s (**2.7×**) |
| n=6 | 未完成 (6min→gen19) | **完成** (5.5min→全部36代) |
| 结果 | — | **410,964,612** |
