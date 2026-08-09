# room-count 项目交接文档

**日期**: 2026-08-09
**项目**: 多联骨牌房间计数 — n×n 网格中 One-sided polyomino 枚举

---

## 一、问题定义

在 n×n（n ≤ 6）正方形网格中，用墙/门围出连通的"房间"。求不同房间形状数量对 n 的表达式。

- **等价问题**: 统计 bounding box ≤ n×n 的 **One-sided polyomino**（允许旋转+平移，禁止翻转）
- **分类**: 有洞（亏格≥1）vs 无洞（亏格=0）

## 二、已知结果（n=1..5 已验证正确）

| n | 总房间数 | 无洞 | 有洞 |
|---|---------|------|------|
| 1 | 1 | 1 | 0 |
| 2 | 4 | 4 | 0 |
| 3 | 46 | 44 | 2 |
| 4 | 2,404 | 1,899 | 505 |
| 5 | 520,818 | 267,976 | 252,842 |
| 6 | 未完成 | — | — |

## 三、技术栈

- **语言**: C99
- **编译器**: GCC 8.1.0 (MinGW-w64 x86_64-posix-seh-rev0)
- **并行**: OpenMP (`-fopenmp`)
- **依赖**: 仅 C 标准库 + libgomp
- **运行环境**: Windows, 32 核 CPU, Git Bash

---

## 四、架构概览

```
src/
├── common.h        # mask_t (uint64), Poly, RoomCount, MAX_N=6, STRIDE=8
├── hashset.h/c     # 开放寻址哈希集, splitmix64, 负载因子 0.5
├── enumerate.h/c   # 枚举引擎（核心算法）
├── chunklist.h     # 64K 固定块链表（避免 realloc 大块连续内存）
├── timer.h         # 增强计时 profiler（--time 参数）
└── main.c          # 入口, 参数解析
```

**枚举算法**: Redelmeier (1981) 生长法 — 从单格开始逐格扩展，用 One-sided canonical form + 哈希集合去重。

**核心数据结构**:
- `mask_t = uint64_t`: 用位图表示 8×8 以内的形状
- `HashSet`: 开放寻址 + 线性探测，key = canonical form (uint64)
- `ChunkList`: 64K 固定块链表，每代形状存储

---

## 五、性能演进历史

### Phase 1: 基础实现 (commit 913522e)

**背景**: 项目初始化，实现 Redelmeier 生长法。

**实现**:
- 嵌套 for(r)for(c) 扫描 `w×h` 全网格提取置位
- 每次候选做 4 次旋转 + 4 次归一化（规范化）
- `realloc` 动态数组存储每代形状

**n=5 性能**: **2.16s**

---

### Phase 2: 四项基础优化 (commit 6bc716a)

**背景**: n=6 需要更高性能，尝试 4 个方向。

**实现**:
1. 分块数组 (chunklist.h): 64K 固定块链表替代 realloc
2. 面积余额检查: `box_at_max` 标志
3. 包围盒满时跳过: `pw==n && ph==n` 跳过扩展检查
4. 不变量预筛选: 用 `(min_dim, max_dim, size)` 签名查重

**结论**: n=5 略微变慢 (2.36s, +9%)。不变量签名太粗（每代仅~8种），预筛选无效。分块数组的链表遍历比连续数组差一点点。但这些优化为 n=6 的内存安全做了准备。

---

### Phase 3: 方向集跳过规范化 (commit 4e8d10a) ⭐

**背景**: 规范化（canonical form 计算）占 80% 时间。88% 的候选是重复的——它们做了规范化后才发现是重复的。

**核心洞察**: GROW 产生的 `raw_mask` 已归一化（左上角 (0,0)）。如果存每个新形状的**全部 4 个归一化旋转**，候选的 raw_mask 直接 uint64 比较就能判定重复 → 跳过规范化。

**实现**:
- 全局 `orient_hs`: 存所有形状的 4 个归一化方向（~2M 条目 for n=5）
- 候选: `hs_contains(orient_hs, raw_mask)` → 命中 → 跳过规范化
- 新形状: 计算 4 方向 → 全部插入 orient_hs
- `__builtin_ctzll` 位迭代: 替换嵌套 for 全扫描，只迭代置位 bit

**n=5 性能**: **0.52s** (4.2× vs 最初)

**代价**: 每个形状存 4 个方向 → 4× 内存。n=5 可接受（~16MB），n=6 面临内存压力。

---

### Phase 4: OpenMP 并行化 (commit ec06849)

**背景**: 32 核机器，尝试并行加速。

**实现**:
- 形状级并行: `#pragma omp parallel for schedule(dynamic,16)`
- `orient_hs` 读无锁 + 写 `#pragma omp critical`
- 每线程独立 ChunkList → 代末 O(1) 拼接
- 移除冗余 `canon_hs`（canonical 已在 orient_hs 中）

**n=5 性能**: **0.41s** (1.3× vs 串行)

**n=6 性能**: 6 分钟跑到 size 19（和串行一样）。因为临界区争用严重。

---

### Phase 5: 增强计时模块 (commit 9d84cc7)

**背景**: 需要精确诊断瓶颈位置。

**实现**:
- 壁钟时间 `omp_get_wtime()`（并行精确）
- `TIMER_WAIT_LOCK`: 临界区等锁时间
- 每代耗时 + 速率输出
- 线程安全累加 `_timer_add` + `omp atomic`

**关键发现**: n=6 等锁占比 **69%**（32 线程中每线程 62s 等待 43s）。

---

### Phase 6: 不变分桶实验 (commit 9d84cc7 中)

**背景**: 尝试用 256 个独立 HashSets + 独立锁消除争用。

**结论**: 等锁仍占 95%+。因为每代只有 ~8 种不变签名活跃 → 256 桶退化。分桶无效。

---

### Phase 7: CAS 无锁哈希实验 (commit 39c1f99)

**背景**: 尝试用 `__atomic_compare_exchange_n` 消除临界区。

**结论**: **更慢**。n=6 6 分钟: CAS 跑到 size 18 (53M) vs 临界区 size 19 (92M)。CAS 自旋重试 > 临界区排队。

---

### Phase 8: 批处理合并 (branch feat/batch-merge) ⭐

**背景**: 彻底消除并行期锁争用——并发期只读本地 orient_hs + 写本地缓冲，代末串行合并到全局。

**实现**:
- 每线程本地 `Lhs` (HashSet) + 连续数组 `TBuf` (5 masks/shape)
- 并行期零锁
- 串行合并: 遍历 TBuf → 全局 Ghs 去重 + 插入 4 方向 + 洞检测 + 累加
- 结果: 等锁 = 0 ✓

**n=5 性能**: **0.44s** (比 main 的 0.59s 更快！)
**n=6 性能**: 6 分钟跑到 gen 16 (13.7M)，速率 ~430K/s

**问题**: gen 17+ 串行合并所需时间指数增长 → 需并行化合并阶段

---

### 发现的工具链问题

| 问题 | 原因 | 解决 |
|------|------|------|
| GCC `-march=native` + OpenMP crash | MinGW SEH bug | 去掉 `-march=native` |
| OpenMP 32 线程 segfault | 默认每线程栈 8MB × 32 = 256MB | `OMP_STACKSIZE=4M` |
| 后台运行无输出 | Bash 后台重定向问题 | 直接运行 |

---

## 六、当前主分支状态（main）

```
版本: 基于 commit 9d84cc7 (增强计时 + 临界区并行)
算法: Redelmeier 生长法 + 方向集 + ctzll + OpenMP
n=5: 0.59s (32 核)
n=6: 6 分钟 -> size 19 (92M 形状), 等锁 69%
```

---

## 七、活跃分支

| 分支 | 内容 | 状态 |
|------|------|------|
| `main` | 临界区并行版 | 稳定 |
| `feat/batch-merge` | 批处理零锁版 | n=5 最快(0.44s), n=6 合并瓶颈 |

---

## 八、下一步方向

1. **并行化合并阶段** (优先级最高) — 把串行 Ghs 插入改为 per-bucket 锁并行
2. **n=6 内存策略** — 方向集存 4 方向 × 上亿形状 → 需内存优化
3. **增量规范化** — 利用父形状的已知规范化推导子形状（理论可行但未实现）

---

## 九、运行命令

```bash
# 编译
gcc -std=c99 -O3 -Wall -Wextra -fopenmp -o room-count.exe src/hashset.c src/enumerate.c src/main.c

# n=5 (串行, 免 OpenMP 开销)
./room-count.exe 5

# n=5 (32 核, profiler)
OMP_NUM_THREADS=32 ./room-count.exe 5 --time

# n=6 (32 核, 6 分钟限制)
OMP_NUM_THREADS=32 timeout 360 ./room-count.exe 6 --time

# 批处理版 (需 OMP_STACKSIZE)
OMP_NUM_THREADS=32 OMP_STACKSIZE=4M ./room-count.exe 6 --time
```

---

**交接人备注**: n=5 结果已确认正确。方向集跳过规范化是最大单项加速（4×）。并行化的主要瓶颈是 orient_hs 写入同步——临界区方案简单有效、批处理方案消锁但合并慢。n=6 完成需要在并行合并和内存管理上继续优化。
