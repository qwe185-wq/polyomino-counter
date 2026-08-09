# room-count — 多联骨牌房间计数

统计 n×n 正方形网格中用墙和门围成的不同房间形状的数量。

## 问题描述

在 n×n（n ≤ 6）的正方形网格中：
- 每条边可以放置**墙**、**门**或**不放**
- 墙和门围成的封闭连通区域称为**房间**
- 房间必须有至少一个门，内部不能有门
- 整个网格只有一个房间
- 房间对方向与位置不敏感（**One-sided**：允许旋转+平移，禁止翻转）

问题等价于：统计所有能嵌入 n×n 网格的 **one-sided polyomino**，并分类为有洞/无洞。

## 实现

| 语言 | 算法 | n=5 | n=6 | 内存 |
|------|------|-----|-----|------|
| **C** | BFS + Burnside | 0.59s | 6min (未完成) | ~8GB |
| **Rust** | BFS + Burnside + Rayon | 0.21s | 328s | ~8GB |
| Rust (实验) | Redelmeier DFS | ~37s | — | O(n) 栈 |
| Rust (实验) | Jensen 转移矩阵 | <1ms | — | 极小 |
| **Rust + export** | BFS + One-sided 导出 + 7z | <1s | ~655s | ~9GB |

## 构建

### C 版本

```bash
cd C
make          # 编译（-O3 优化）
make run      # 编译并运行
make clean    # 清理
```

**依赖**：GCC（MinGW-w64 或 Linux GCC），仅标准库。

### Rust 版本

```bash
cd rust
cargo build --release             # 编译
cargo run --release -- 5          # 枚举 n=5
cargo run --release -- 6 --export # 枚举 + 导出 One-sided 二进制数据
cargo test --release              # 测试
```

**依赖**：Rust 工具链（cargo），依赖库见 `rust/Cargo.toml`。

### 导出工具

```bash
# 去重验证（两阶段：hash 分片 + rayon 并行排序）
cargo run --release --bin dedup_check -- output/all_fixed.zip

# 形状提取与可视化
python read_shapes.py output/all_fixed.zip --info              # 文件摘要（零解压）
python read_shapes.py output/all_fixed.zip --txt --limit 10    # 前 10 个形状
python read_shapes.py output/all_fixed.zip --ascii --from 1000 --to 1005  # 指定范围
```

## 项目结构

```
room-count/
├── C/                    # C 实现
│   ├── Makefile
│   ├── docs/HANDOFF.md
│   └── src/
│       ├── common.h      # 通用类型与宏
│       ├── hashset.h/c   # 哈希集合（去重）
│       ├── enumerate.h/c # 枚举引擎
│       ├── timer.h       # 计时模块
│       └── main.c        # 入口 + 输出
├── rust/                 # Rust 实现
│   ├── Cargo.toml
│   ├── docs/HANDOFF-RUST.md
│   ├── src/
│   │   ├── main.rs       # 入口 + CLI（--export --jensen --dfs）
│   │   ├── types.rs      # 核心类型
│   │   ├── bit_utils.rs  # 位运算 + 洞检测 + 旋转 + 归一化
│   │   ├── hashset.rs    # 分片并发哈希集
│   │   ├── fixed.rs      # BFS 枚举 + 对称检测 + One-sided 去重
│   │   ├── export.rs     # 分块二进制导出 + 7z 压缩
│   │   ├── symmetric.rs  # 90°/180° 旋转对称
│   │   ├── burnside.rs   # Burnside 引理
│   │   ├── redelmeier.rs # Redelmeier DFS (实验)
│   │   ├── jensen.rs     # Jensen 转移矩阵 (实验)
│   │   └── bin/
│   │       └── dedup_check.rs  # 并行去重验证工具
│   └── output_n6/        # n=6 导出结果（.gitignore 排除）
├── read_shapes.py        # 形状提取脚本（支持 .zip 流式读取）
├── docs/
│   ├── plans/            # 实施计划
│   └── .lifecycle/       # 编排器状态
├── README.md
└── CHANGELOG.md
```

## 结果

| n | 总房间数 | 无洞（亏格0） | 有洞（亏格≥1） |
|---|---------|-------------|---------------|
| 1 | 1 | 1 | 0 |
| 2 | 4 | 4 | 0 |
| 3 | 46 | 44 | 2 |
| 4 | 2,404 | 1,899 | 505 |
| 5 | 520,818 | 267,976 | 252,842 |
| **6** | **410,964,612** | **112,877,832** | **298,086,780** |

n=6 由 Rust BFS 计算（328s 枚举，~655s 含导出+压缩，~9GB RAM）。

### 导出数据集

`rust/output_n6/` 包含 n=6 全部 410,964,612 个 One-sided polyomino：

| 文件 | 大小 | 内容 |
|------|------|------|
| `all_fixed.zip` | 1.0 GB | 全部（42 个 chunk，每块 10M masks） |
| `no_holes/n06_fixed.zip` | 288 MB | 无洞（112,877,832） |
| `with_holes/n06_fixed.zip` | 742 MB | 有洞（298,086,780） |

二进制格式：每个形状 8 字节 u64 LE（规范化位图，stride=8，左上角对齐）。

## 许可

MIT
