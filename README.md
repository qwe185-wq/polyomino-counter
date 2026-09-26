# Polyomino Counter — 前沿连通性 DP 计数原型

**当前分支：`codex/transfer-20260926`。** 保存首次引入 transfer.rs 的独立算法原型及遍历优化，便于研究从形状枚举转向状态计数的过程。

前沿 DP 通过 transfer_probe 单独运行；本分支主程序仍默认使用 Fixed BFS，不能使用后续版本的 --algorithm transfer。

[分支导航与版本选择](docs/BRANCHES.md) · [GitHub 仓库](https://github.com/qwe185-wq/polyomino-counter) · [本分支技术依据](rust/src/transfer.rs)

## 问题定义

统计可嵌入 n×n 网格的非空四邻接连通格子集合，合并平移与旋转，镜像保持不同。n 是最大包围盒边长，不是格子数。输出总数、无洞数和有洞数；洞按背景四邻接可达性判断，对角缝隙不算通道。本程序只计形状，不另外计算门或墙的布置。

## 本分支实现与限制

- `rust/src/transfer.rs`：前沿连通性状态、欧拉特征累计、平移归一化和旋转固定集计数原型。
- `rust/src/bin/transfer_probe.rs`：独立运行前沿 DP，固定检查 n=1..5，并保留计时结果避免编译优化消去工作。
- `rust/src/main.rs`：仍为早期 Fixed BFS + Burnside 主入口，`--jensen`、`--dfs` 调用历史实验实现，不能当作本分支新 transfer 模块入口。
- `rust/src/fixed.rs` 与 `export.rs`：枚举、旋转代表输出及外部 7-Zip 压缩，尚无后续 `--algorithm`、`--no-compress` 或原生 ZIP 参数。
- 支持的固定位图尺寸为 n≤6；前沿探针与其已知结果测试只运行到 n=5。新用户建议使用 `main` 的集成实现。

## 构建与运行

需要 Rust/Cargo；读取工具使用 Python 3.10+ 标准库。命令在 `rust` 目录执行。显式给出小 n，避免旧主程序无参数时默认启动 n=6 BFS。

```powershell
git clone https://github.com/qwe185-wq/polyomino-counter.git
cd polyomino-counter
git switch --track origin/codex/transfer-20260926
cd rust
cargo build --release --locked

# 本分支新增的 DP 原型：固定计算到 n=5
cargo run --release --locked --bin transfer_probe
cargo test --release --locked --bin transfer_probe

# 历史主程序的 BFS 入口；这里必须指定二进制名
cargo run --release --locked --bin room-count -- 3
cargo run --release --locked --bin room-count -- 3 --export --export-dir output_n3_new
```

导出压缩需要外部 7-Zip。旧版本的错误处理与输出目录行为不同于 main，应使用新的输出目录。该分支适合复现算法演进，日常数据生产优先采用 main。

## 已知分类计数

| n | 总数 | 无洞 | 有洞 |
|---:|---:|---:|---:|
| 1 | 1 | 1 | 0 |
| 2 | 4 | 4 | 0 |
| 3 | 46 | 44 | 2 |
| 4 | 2,404 | 1,899 | 505 |
| 5 | 520,818 | 267,976 | 252,842 |
| 6 | 410,964,612 | 112,877,832 | 298,086,780 |

n=1..5 是 transfer_probe 的自动校验值。n=6 来自历史 Rust BFS 记录，不能据此声称本分支已做同规模 DP 性能验收。旧 BFS 记录为约 328 秒枚举、约 655 秒含导出压缩、约 9 GB 内存；它与后续前沿 DP 的算法时间和资源口径不同。

## 数据读取与编码

导出记录是 8 字节 u64 little-endian 位图，bit=`row*8+col`，紧包围盒左上对齐，取四个旋转中的最小代表。名称中的 `fixed` 是历史命名，输出代表实际按 one-sided 旋转去重。每个 chunk 最多 10,000,000 条记录。

该版本同时写出 `all_fixed` 全集和有洞/无洞分类副本，没有后续 v2 单份分类流或动态 v3 清单。

```powershell
# 返回仓库根目录后执行
python read_shapes.py rust/output_n3_new/all_fixed.zip --info
python read_shapes.py rust/output_n3_new/all_fixed.zip --ascii --limit 10
```

并行枚举顺序和不同数据集的索引可能不同，不能将一份 ZIP 的 global_index 直接用于另一份输出。仓库不附带生成的数据集。

## 代码和历史记录

| 路径 | 内容 |
|---|---|
| [rust/src/transfer.rs](rust/src/transfer.rs) | 新增前沿 DP 原型 |
| [rust/src/bin/transfer_probe.rs](rust/src/bin/transfer_probe.rs) | 计时探针及 n≤5 校验 |
| [rust/README.md](rust/README.md) | 本分支 Rust 入口说明 |
| [C/docs/HANDOFF.md](C/docs/HANDOFF.md) | C 版本历史实现与测量 |
| [rust/docs/HANDOFF-RUST.md](rust/docs/HANDOFF-RUST.md) | Rust 早期实现记录 |
| [CHANGELOG.md](CHANGELOG.md) | 早期迭代记录 |

历史说明中的实验实现状态应结合当前分支源码阅读。原项目 README 标注许可为 MIT。
