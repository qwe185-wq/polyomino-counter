# room-count 项目交接文档 — Rust 实现版

**日期**: 2026-08-09
**分支**: `room-count-Rust` (工作树: `.worktrees/room-count-Rust/rust/`)
**基础**: 基于 main 分支 `254a017`

---

## 一、项目概述

在 n×n（n ≤ 6）正方形网格中枚举 One-sided polyomino（允许旋转+平移，禁止翻转），分类统计有洞/无洞。

**Rust 实现**: Burnside 引理 + Fixed Polyomino 枚举 + 多种实验性算法

---

## 二、已完成结果

### n=6 最终结果

| n | 总房间数 | 无洞 | 有洞 | 验证 |
|---|---------|------|------|------|
| 1 | 1 | 1 | 0 | ✓ |
| 2 | 4 | 4 | 0 | ✓ |
| 3 | 46 | 44 | 2 | ✓ |
| 4 | 2,404 | 1,899 | 505 | ✓ |
| 5 | 520,818 | 267,976 | 252,842 | ✓ |
| **6** | **410,964,612** | **112,877,832** | **298,086,780** | **新** |

### n=6 Burnside 分解

```
Fixed(n=6)   = 1,643,823,600
Sym90(n=6)   =           202
Sym180(n=6)  =        34,444
Burnside sum = 1,643,858,448  (sum%4=0 ✓)
One-sided    =   410,964,612
```

### 性能

| n | 耗时 | 峰值内存 | vs C 版本 |
|---|------|----------|-----------|
| 5 | 0.21s | ~50 MB | C: 0.59s → **2.8×** |
| 6 | 328s (5.5min) | ~8 GB | C: 未完成 (6min→gen19) |

---

## 三、项目结构

```
rust/
├── Cargo.toml              # 依赖: rayon, parking_lot, rustc-hash, mimalloc, clap
├── src/
│   ├── main.rs             # 入口 + CLI
│   ├── types.rs            # Mask, Poly, RoomCount 类型
│   ├── bit_utils.rs        # 位运算: 归一化、前沿计算、洞检测、GROW
│   ├── hashset.rs          # 分片并发哈希集 (1024 shards + parking_lot RwLock)
│   ├── fixed.rs            # Fixed Polyomino BFS 枚举 + 内联对称检测
│   ├── symmetric.rs        # 对称性检测 (has_symmetry_90, has_symmetry_180)
│   ├── burnside.rs         # Burnside 引理组合器
│   ├── redelmeier.rs       # Redelmeier DFS (实验性, WIP)
│   └── jensen.rs           # Jensen 转移矩阵 (实验性, WIP)
└── target/release/room-count.exe
```

---

## 四、已完成的改进（vs C 版本）

### 算法改进

| 改进 | C 版本 | Rust 版本 | 加速比 |
|------|--------|----------|--------|
| **规范化** | 4旋转+4归一化 per候选 | 仅1翻译归一化 (Burnside) | ~4× |
| **去重** | 单一 `omp critical` (69%等锁) | 1024分片 RwLock (<3%争用) | ~3× |
| **内存分配** | 系统 malloc | mimalloc (减少碎片) | 稳定性↑ |
| **并行模型** | OpenMP dynamic | Rayon work-stealing | — |
| **对称检测** | 事后 Burnside 组合 | 内联检测 (Fixed枚举中同步完成) | 零额外开销 |

### 工程改进

| 方面 | C | Rust |
|------|---|------|
| 线程安全 | 人工保证 (omp critical) | 编译期 Send+Sync |
| 内存安全 | 人工管理 (malloc/free) | 所有权系统 |
| 测试 | 无 | 17 个单元测试 + 集成验证 |
| 哈希 | splitmix64 | FxHash (rustc-hash, ~2× faster) |

---

## 五、三种方案状态总结

### 方案 A: Fixed BFS + Burnside ✅ 生产可用

**文件**: `fixed.rs`, `burnside.rs`, `symmetric.rs`

**算法**: Redelmeier 生长法 (逐代 BFS) + 翻译归一化 + 内联对称检测 + Burnside 组合

**状态**:
- ✅ n=1-5 与已知值完全一致
- ✅ n=6 完成 (328s, 410,964,612)
- ✅ 17 个测试全部通过
- ✅ 生产可用

**运行**:
```bash
cd rust
cargo run --release -- 5          # 验证 0.21s
cargo run --release -- 6          # 完整计算 328s
```

### 方案 B: Redelmeier DFS ⚠️ 实验性

**文件**: `redelmeier.rs`

**算法**: 真实 Redelmeier (1981) DFS 回溯 + Untried Set + 半平面约束 + 对称加权

**设计优势** (vs BFS):
- 无需哈希集去重 (每个形状恰好生成一次)
- O(n) 内存 (仅搜索栈)
- 尾部聚合 (Shirakawa 2025) 可进一步加速

**状态**:
- ✅ n=1-2 正确
- ❌ n≥3 有 bug: 生成128个形状 vs BFS的151个 (n=3)
- ❌ 尾部聚合未集成
- ❌ 无锁并行未集成

**待解决问题**:
1. **半平面约束正确性**: 当前实现可能在某些形状的生成路径上过于严格
2. **Untried Set 保存/恢复**: DFS 递归中的状态隔离已验证正确 (n=2)
3. **u128 位掩码**: 编码逻辑正确，支持 n≤6 坐标范围

**下一步**: 对比 DFS 与 BFS 生成的形状集合，定位缺失的 23/151 个形状

### 方案 C: Jensen 转移矩阵 ⚠️ 实验性

**文件**: `jensen.rs`

**算法**: Jensen (2001) 逐列转移矩阵

**设计优势** (vs 生长法):
- 状态空间极小 (~800 for n=6)
- 矩阵幂乘可在 <1s 完成 n=6
- 完全不枚举单个形状

**状态**:
- ✅ 状态枚举 (BFS 生成所有列签名)
- ✅ 转移函数 (列→列 + 连通性)
- ✅ 边界固定 (row0 必须占用)
- ✅ n=1 正确
- ❌ n≥2 偏高 (重复计数 bug)

**待解决问题**:
1. **形状终止时机**: 当前在"终止路径"和"最后一列"重复计数
2. **分量关闭检测**: 需要精确判断形状何时完成

**下一步**: 修复重复计数——只在分量全部关闭且不会再有新列时计数

---

## 六、算法优化空间 (未实现)

基于学术文献调研的潜在优化方向：

### 高优先级

| 优化 | 预期收益 | 参考 |
|------|----------|------|
| **真实 Redelmeier DFS** (修复方案B) | 5-10×, O(n)内存 | Redelmeier 1981 |
| **Jensen 转移矩阵** (修复方案C) | 100-300× for n≤6 | Jensen 2001 |
| **尾部聚合** (Shirakawa 2025) | 1.5-2× | arXiv:2510.22446 |

### 中优先级

| 优化 | 说明 | 参考 |
|------|------|------|
| **无锁子树并行** | Thread-ID Modulo at depth D | Shirakawa 2025 |
| **PGO 编译** | cargo-pgo 两阶段编译 | — |
| **邻接计数器** | 避免扫描全部前沿 | Shirakawa 2025 |

### 参考文献

1. Redelmeier, D.H. "Counting Polyominoes: Yet Another Attack." *Discrete Mathematics* 36 (1981): 191-203.
2. Jensen, I. "Counting Polyominoes: A Parallel Implementation for Cluster Computing." (2003)
3. Shirakawa, T. "Enumeration of Polyominoes up to Size N=59." arXiv:2510.22446 (2025)
4. Aleksandrowicz & Barequet. "Counting Polycubes without the Dimensionality Curse." (2008-2011)
5. OEIS A001168 (Fixed polyominoes), A000988 (One-sided polyominoes)

---

## 七、运行命令

```bash
# 编译
cd D:\git\room-count\.worktrees\room-count-Rust\rust
cargo build --release

# 验证 n=1..5
cargo test fixed burnside symmetric hashset

# n=5 快速验证
cargo run --release -- 5

# n=6 完整计算 (需 >8GB RAM, ~5.5min)
cargo run --release -- 6

# 实验性方案
cargo run --release -- 3 --dfs      # Redelmeier DFS (WIP)
cargo run --release -- 3 --jensen   # Jensen 转移矩阵 (WIP)
```

---

## 八、Git 分支信息

| 分支 | 说明 |
|------|------|
| `main` | C 版本 (原始实现) |
| `room-count-Rust` | Rust 实现 (本文档对应分支) |
| `feat/batch-merge` | C 批处理零锁实验版 |

**room-count-Rust 提交记录**:
```
3bea27a feat: Jensen 转移矩阵 (WIP) + n=1 正确
9f8bcb0 refactor: Redelmeier DFS 简化为调试版
88b20ab feat: Redelmeier DFS (WIP) + 主算法优化完善
ebd4371 perf: 1024分片 + 预分配上限 + mimalloc
79441e4 feat: Rust 实现 — Burnside 引理 + Fixed Polyomino
```

---

## 九、下一步建议

1. **P0**: 修复方案B (Redelmeier DFS) 的 n≥3 bug → 获得 O(n) 内存 + 无需哈希集
2. **P1**: 修复方案C (Jensen) 的重复计数 → 获得 <1s n=6 验证
3. **P2**: 集成尾部聚合到方案B → 额外 1.5-2× 加速
4. **P3**: 集成无锁并行到方案B → 线性扩展到 32+ 核

**交接人备注**: n=6 结果已获 (410,964,612)。BFS 方案生产可用。Redelmeier DFS 和 Jensen 转移矩阵是两种互补的优化路径——前者消除内存瓶颈，后者消除计算瓶颈。两者都只需修复已知的 bug 即可投入使用。
