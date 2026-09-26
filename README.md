# Polyomino Counter — 默认前沿 DP 与并行形状枚举优化

**当前分支：`codex/performance-20260926`。** 将精确前沿 DP 接入默认纯计数入口，引入 one-sided BFS、可选算法、可调 ZIP 压缩和读取器兼容性验证。

尺寸范围为 1..6；本分支这一轮回归、性能和导出对照只验证到 n=5。ZIP 压缩依赖外部 7-Zip，尚无原生流式 ZIP 或动态尺寸。

[分支导航与版本选择](docs/BRANCHES.md) · [GitHub 仓库](https://github.com/qwe185-wq/polyomino-counter) · [本分支技术依据](docs/performance-2026-09-26.md)

统计能嵌入 n×n 正方形网格的非空边连通格子集合。平移、旋转合并，镜像保持不同（one-sided polyomino）；分别统计总数、无洞和有洞。这里计的是形状，不区分门的位置或数量。洞按空格的四邻接可达性判断，对角缝隙不算通道。

Rust 主程序提供两条默认路线：

- **只计数：前沿连通性 DP + 旋转轨道枚举 + Burnside**，不保存每个形状。
- **导出：并行 one-sided BFS**，每个旋转等价类只保留一个最小位图代表。

原 Fixed BFS 可显式选择，作为交叉验证和性能比较。旧 `jensen.rs`、`redelmeier.rs` 保留为历史实验源码，不再编入生产入口；旧 `--jensen` 参数调用新的正确 DP，`--dfs` 已移除。C 目录保留历史实现，不作为当前高性能入口。

## 环境与版本选择

需要 Rust/Cargo 工具链；Python 读取工具使用 Python 3.10+ 标准库。Rust 包和可执行文件名仍为 `room-count`。请先克隆仓库并切换到本文开头标明的分支，再按下方命令运行。完整分支获取命令见[分支导航](docs/BRANCHES.md)。n 是最大包围盒边长，不是形状包含的格子数。

## 构建与使用

```powershell
cd rust
cargo build --release --locked

# n 是必填参数，无参数不会启动计算
cargo run --release -- 5
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --algorithm bfs --verbose

# 每次使用新的目录；默认 ZIP 压缩等级1
cargo run --release -- 5 --export --export-dir output_n5_new

# 裸二进制，跳过压缩
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw

# 更高压缩率，耗时也更长
cargo run --release -- 5 --export --compression-level 9 --export-dir output_n5_zip9

cargo test --release --locked
cargo test --locked
```

实现的尺寸上限仍为 6；本轮性能改造的测试、基准和导出验证全部限制在 n≤5。**n=6 尚未重新验证，本分支未提供这一轮 n=6 重验结论。**测试和 `transfer_probe` 不会自动运行 n=6。

ZIP 压缩需要 7-Zip。Windows 自动检测常用安装位置，否则从 PATH 查找 `7z`；可用环境变量 `ROOM_COUNT_7Z` 指定可执行文件。使用 `--no-compress` 不需要 7-Zip。算法和输出编码不依赖新的外部库。

## 性能

2026-09-26，Windows x86_64、Rust 1.97.1，Rayon 32 线程，release 默认可移植编译配置。n=5，每种计数预热一次后交替测量9次；导出预热一次后测3次。下表为中位数。

| 路线 | 算法耗时 | 完整进程耗时 |
|---|---:|---:|
| 改造前 Fixed BFS | 205 ms | 227 ms |
| 新前沿 DP（默认计数） | **0.570 ms** | **11.2 ms** |
| 优化后 Fixed BFS | 68.3 ms | 81.9 ms |
| 新 one-sided BFS | 29.0 ms | 41.2 ms |
| 改造前导出 + ZIP9 | 282 ms | 3.275 s |
| 新导出 + ZIP9 | 34.1 ms | 2.923 s |
| 新导出 + ZIP1（默认） | 34.8 ms | **0.389 s** |
| 新导出、不压缩 | 34.5 ms | **0.050 s** |

计数算法约快 360 倍，但包含启动开销的 CLI 约快20倍，不能混用这两种口径。默认导出总耗时约缩短到原来的1/8.4，既包括枚举优化也包括降低压缩等级；保持 ZIP9 时，压缩仍是主要耗时。

ZIP 总体积的一个 n=5 样本：旧 ZIP9 2.60 MB，新 ZIP9 2.47 MB，新 ZIP1 3.18 MB。并行导出顺序不固定，因此压缩体积可略有变化。详见 [性能实现与验证](docs/performance-2026-09-26.md)。上述数据不能外推为 n=6 的实测速度或峰值内存。

## 结果与正确性

| n | 总数 | 无洞 | 有洞 |
|---:|---:|---:|---:|
| 1 | 1 | 1 | 0 |
| 2 | 4 | 4 | 0 |
| 3 | 46 | 44 | 2 |
| 4 | 2,404 | 1,899 | 505 |
| 5 | 520,818 | 267,976 | 252,842 |
| 6（历史结果，本轮未复验） | 410,964,612 | 112,877,832 | 298,086,780 |

新 DP、Fixed BFS、one-sided BFS 在 n≤5 的三类计数一致。测试用独立坐标/洪泛 oracle 穷举 n≤4，验证形状、旋转、洞、导出集合；还校验小矩形 DP 和旋转固定点。实际 n=5 旧版 ZIP9、新版 ZIP9/ZIP1 各有520,818个不重复形状，完整集合相同。

## 数据格式与兼容性

每个 mask 为8字节 u64 little-endian，格子 `(row,col)` 对应 bit `row*8+col`，紧包围盒左上对齐，取4个旋转中位图值最小的代表。每10,000,000条记录分一个 chunk。

```text
output_n5_new/
├── dataset.json
├── all_fixed.zip
├── no_holes/n01_fixed.zip … n05_fixed.zip
└── with_holes/n03_fixed.zip … n05_fixed.zip
```

`fixed` 是兼容历史工具的文件名，文件内容实际为 **one-sided**。分类文件的 nXX 表示最大包围盒边长**恰好为XX**，程序的 n 行表示边长**不超过n**的累计值。空分类不生成文件。裸导出使用同名目录和 `shapes_000001.bin`。

导出拒绝非空目录，避免覆盖现有数据。每个成功完成的数据集有独立 `dataset_id`；顺序标记为 `parallel-unspecified`。**形状集合保持兼容，但旧 ZIP 的 global_index 不能直接用于新 ZIP。**外部 Parquet、landmarks 和游戏资产必须绑定原数据集，或对新数据重新建立索引。写盘、压缩失败返回非零状态；失败任务不会生成 `complete: true` 清单，原始数据保留用于排查。

现有读取工具仍可使用：

```powershell
python read_shapes.py rust/output_n5_new/all_fixed.zip --info
python read_shapes.py rust/output_n5_new/all_fixed.zip --ascii --limit 10
```

`dedup_check` 是历史辅助工具，只能检查部分性质；完整正确性以回归测试、分类计数和集合对照为准。

## 主要代码

- `rust/src/transfer.rs`：前沿分量状态、增量欧拉特征、差分去平移、旋转轨道。
- `rust/src/fixed.rs`：两种 BFS、位前沿、分块任务、局部统计和流式导出。
- `rust/src/bit_utils.rs`：位矩阵旋转、平移归一化、欧拉洞检测。
- `rust/src/export.rs`：惰性分配输出槽、新目录保护、可选 ZIP。
- `rust/src/validation.rs`、`rust/tests/cli.rs`：独立 oracle、全集合、CLI及失败路径回归。
- `rust/scripts/benchmark.ps1`：指定旧版 executable 后复现 n≤5 对照基准。

MIT。

## AI 辅助工具

- **DeepSeek**
- **OpenAI Codex**

以上工具用于辅助项目开发与文档整理。项目由 [qwe185-wq](https://github.com/qwe185-wq) 维护；工具署名与 GitHub 根据提交记录生成的贡献者列表分别管理。
