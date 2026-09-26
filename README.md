# Polyomino Counter — 动态尺寸、任意精度与旋转商图研究

**当前分支：`codex/dynamic-n-20260926`。** 在 main 的动态计数和导出能力上，继续研究旋转固定集商图，加入紧凑标签、数值特化和 split/single/two/center 四种显式商图模式。

第二轮改进主要作用于显式 quotient 内核；默认 auto 的内核选择未改变。局部内核速度与内存收益不等于完整 n 计数的同等收益，也不代表已完成 n=14。

[分支导航与版本选择](docs/BRANCHES.md) · [GitHub 仓库](https://github.com/qwe185-wq/polyomino-counter) · [本分支技术依据](docs/fixedset-round2-2026-09-26.md)

统计能嵌入 n×n 正方形网格的非空边连通格子集合。平移、旋转合并，镜像保持不同（one-sided polyomino）；分别统计总数、无洞和有洞。这里计的是形状，不区分门的位置或数量。洞按空格的四邻接可达性判断，对角缝隙不算通道。

Rust 主程序提供两条默认路线：

- **只计数：前沿连通性 DP + 旋转轨道枚举 + Burnside**，不保存每个形状。
- **导出：前沿状态图回溯（frontier）**，共享后缀可行性表，逐条生成最小旋转代表，不保存全局形状集合。

**现在支持 n=7、8、9…的动态尺寸**。n≤6沿用已优化的固定位图路线；n>6自动采用动态前沿、任意宽位图和任意精度计数，不设置经验性的内存、时间或小尺寸上限。只拒绝零、非法参数及机器地址/尺寸运算溢出。旧 `bfs/canonical/redelmeier` 参考实现仍限n≤6；大尺寸使用 `auto/transfer/frontier`。

`canonical`（one-sided BFS）、`bfs`（Fixed BFS）和新 `redelmeier` 可显式选择，作为交叉验证或低内存参考。旧 `jensen.rs`、`redelmeier.rs` 保留为历史实验源码，不再编入生产入口；新的 Redelmeier 实现在 `redelmeier_export.rs`。旧 `--jensen` 参数调用正确 DP，`--dfs` 已移除。C 目录保留历史实现。

## 项目用途与快速开始

**Polyomino Counter**（原目录名 `room-count`）是面向组合计数、网格房间形状研究和程序化内容数据准备的命令行工具。Rust 负责精确计数与导出，Python 负责读取和 ASCII 展示，C 版本保留早期实现以供比较。仓库名称改变不影响 Cargo 包名和可执行文件名 `room-count`。

这里的 **n 是容纳形状的正方形边长，不是形状包含的格子数量**。例如 n=3 会统计从单格到 3×3 实心方块的所有可容纳连通形状；总数为 46，其中无洞 44、有洞 2。

```powershell
git clone https://github.com/qwe185-wq/polyomino-counter.git
cd polyomino-counter/rust
cargo build --release --locked
cargo run --release --locked -- 3
```

需要 Rust/Cargo 工具链；Python 读取器使用 Python 3.10+ 标准库。当前原生 ZIP 无需额外安装 7-Zip，只有显式选择 `--compression-backend 7z` 才依赖它。历史 C 版本使用 GCC、Make 与 OpenMP，构建说明见 [C 实现记录](C/docs/HANDOFF.md)。性能记录主要来自 Windows，跨平台运行时间不保证相同。

## 算法与能力选择

| 入口 | 适用任务 | 尺寸与行为 |
|---|---|---|
| `--algorithm auto`（默认） | 常规计数或加 `--export` 导出 | 自动选择 transfer 计数 / frontier 导出 |
| `--algorithm transfer` | 只需要数量 | 前沿 DP、旋转固定集与 Burnside；不保存全集，不支持导出 |
| `--algorithm frontier` | 逐形状遍历，可加 `--export` | 前沿图回溯，复用可行性表；支持动态尺寸 |
| `--algorithm bfs` | Fixed BFS 参考与交叉验证 | n≤6，保存枚举状态 |
| `--algorithm canonical` | one-sided BFS 参考 | n≤6，按旋转代表去重 |
| `--algorithm redelmeier` | 生长枚举参考 | n≤6，无全局形状去重表 |

计数按平移规范化后的形状计算旋转不动点，再以 Burnside 引理合并旋转轨道。洞分类与连通性在状态转移中保留，因此可在不写出每个形状的情况下计算分类数量。更细的模块职责和历史优化证据见下文。

## 大尺寸计数与资源控制

```powershell
# 以下命令在 rust 目录运行；先用小 n 验证环境
cargo run --release --locked -- 7 --profile-count
cargo run --release --locked -- 7 --count-cache-dir ./count-cache
cargo run --release --locked -- 7 --count-threads 4 --count-memory-mib 4096
```

`--profile-count` 输出分阶段诊断；`--count-cache-dir` 复用已完成的精确分项，不是中途 DP 状态的断点续算。缓存发布依赖支持硬链接的文件系统。`--count-threads` 并行处理较小分项，大型 DP 单独运行。`--count-memory-mib` 是调度预算，**不是操作系统强制内存上限**。这些选项只用于纯计数 `auto/transfer`，不能和导出混用。

`--symmetry-engine auto|frontier|quotient|gray` 用于旋转固定集内核比较。默认使用 `auto`；大轨道强制 Gray 枚举可能非常耗时。动态整数表示避免固定 u64 计数溢出，但计算成本仍随尺寸快速增长。

## 当前验证边界

| 范围 | 已记录的证据 | 限制 |
|---|---|---|
| n≤4 | 独立坐标/洪泛 oracle 穷举与集合核对 | 小规模完整交叉验证 |
| n≤5 | 多种算法、导出集合与读取兼容性对照 | 历次性能环境见原记录 |
| n=6 | 纯计数及 410,964,612 条完整分类导出验收 | 全集不会随源码仓库提供 |
| n=7 | 动态 DP 与独立保留的旧参考 DP 一致 | 大尺寸全集导出尚未完成验证 |
| n=8..13 | 完整计数与优化前后/重复运行的分类核对 | n=13 尚无独立全量计数 oracle |
| n=14 | 600 秒、4 GiB Job 限制内尝试 | 内存限制终止，未得到完整结果 |

n=13 的一次完整运行用时 122.404 秒、峰值 Job 提交内存约 1.719 GiB；n=14 在约 517.661 秒因内存限制停止。计时口径和分类结果见[分阶段优化与完整计数](docs/fixedset-performance-2026-09-26.md)。这些是已有实验记录，本次文档发布没有重新运行大规模计算。

## 构建与使用

```powershell
cd rust
cargo build --release --locked

# n 是必填参数，无参数不会启动计算
cargo run --release -- 5
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --algorithm bfs --verbose

# 动态尺寸：完整计数使用BigUint，不会按u64取模
cargo run --release -- 7
cargo run --release -- 9 --algorithm transfer

# 每次使用新的目录；默认 ZIP 压缩等级1
cargo run --release -- 5 --export --export-dir output_n5_new

# 裸二进制，跳过压缩
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw

# 更高压缩率，耗时也更长
cargo run --release -- 5 --export --compression-level 9 --export-dir output_n5_zip9

# 可选外部7-Zip：同等级与native的耗时、压缩率不同
cargo run --release -- 5 --export --compression-backend 7z --compression-level 9 --export-dir output_n5_7z

cargo test --release --locked
cargo test --locked
```

动态路线的n=7完整计数已实跑，并与独立保留的旧DP参考（仅将尺寸常量改为7）一致：总数1,185,652,433,093，无洞144,608,553,854，有洞1,041,043,879,239。后续完整计数已推进到n=13，详见上方验证边界；n≥7完整形状导出尚未完成验证；动态表示通过小规模全集与大位宽边界测试，详见[动态尺寸实现与验证](docs/dynamic-dimensions-2026-09-26.md)。普通回归不会启动完整n≥6枚举。

之前n=6的受限验证继续有效：纯计数通过32 MiB Job上限；完整裸导出在8线程、128 MiB上限下成功，单次整进程5.27秒、峰值Job提交内存96.25 MiB。

Windows 下需要有界运行时，先构建，再执行以下命令；脚本默认 n=5、512 MiB、30秒，n=6必须显式指定：

```powershell
# 在 rust 目录内；显式选择 n=6，使用脚本的硬资源上限
./scripts/measure-transfer.ps1 -Exe ./target/release/room-count.exe -N 6 -MemoryMiB 32 -Runs 9

# 完整裸导出，保留数据集与计时证据；默认8线程、128 MiB、60秒
./scripts/measure-export.ps1 -Exe ./target/release/room-count.exe -N 6 -Mode raw
```

内存限制仅由该脚本的 Job Object 强制执行；直接调用主程序不会自动套用这个上限。

这两个历史基准脚本只验证已知的n≤6。动态尺寸可直接调用程序，例如 `./target/release/room-count.exe 7 --export --no-compress --export-dir output_n7_new`；主程序不施加这些脚本的资源上限。

默认 `--compression-backend native` 使用Rust `zip`/`flate2` 流式压缩，无需外部程序。选择 `7z` 时，Windows 自动检测常用安装位置，否则从 PATH 查找 `7z`；可用 `ROOM_COUNT_7Z` 指定可执行文件。等级0为不压缩ZIP，默认等级1优先速度。

## 性能

完整导出优化：n=5、8线程，原版双份裸导出整进程中位92.72 ms，新前沿回溯单份导出42.19 ms；后续同批ZIP1对照，原生流式ZIP整进程59.44 ms，外部7z为282.13 ms。新格式裸数据减半。上述比较分别包含算法、存储布局或压缩后端变化，不能作为单一算法的加速倍数。详见[完整导出优化与验收](docs/export-optimization-2026-09-26.md)。

后续 Jensen 优化：n=6 算法中位数 **4.777 → 2.217 ms（约2.15倍）**；n=5 为 **0.568 → 0.232 ms**。最终版 n=6 Job 峰值提交内存约12.53 MiB，并通过32 MiB硬限制。受限启动器下进程耗时约27 ms，本轮未测出明确的进程端到端加速。详见[逐项优化与有界验证](docs/jensen-optimization-2026-09-26.md)。

下面保留第一批改造的历史对照：2026-09-26，Windows x86_64、Rust 1.97.1，Rayon 32 线程，release 默认可移植编译配置。n=5，每种计数预热一次后交替测量9次；导出预热一次后测3次。下表为中位数。

| 路线 | 算法耗时 | 完整进程耗时 |
|---|---:|---:|
| 改造前 Fixed BFS | 205 ms | 227 ms |
| 第一批前沿 DP | **0.570 ms** | **11.2 ms** |
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
| 6（纯计数及完整裸导出已受限验收） | 410,964,612 | 112,877,832 | 298,086,780 |
| 7（动态纯计数已交叉验证） | 1,185,652,433,093 | 144,608,553,854 | 1,041,043,879,239 |

新 DP、Fixed BFS、one-sided BFS 在 n≤5 的三类计数一致。测试用独立坐标/洪泛 oracle 穷举 n≤4，验证形状、旋转、洞、导出集合；还校验小矩形 DP 和旋转固定点。实际 n=5 旧版 ZIP9、新版 ZIP9/ZIP1 各有520,818个不重复形状，完整集合相同。

本次frontier和新Redelmeier也通过n≤5计数与完整集合对照；n=6裸数据及原生ZIP解码结果的410,964,612条记录通过独立逐条合法性、分类和全键查重验收。原生ZIP1单次整进程16.02秒、126.41 MiB峰值Job提交内存、1.11 GB压缩文件；该内存值接近本次128 MiB限制，不能当作跨机器上界。

## 数据格式与兼容性

每个 mask 为8字节 u64 little-endian，格子 `(row,col)` 对应 bit `row*8+col`，紧包围盒左上对齐，取4个旋转中位图值最小的代表。每10,000,000条记录分一个 chunk。

以上是n≤6的v2格式。**n>6输出v3格式**：无符号小端整数，`stride=max(8,n)`，每记录 `record_bytes=max(8,ceil(n*stride/8))` 字节；两者都写入清单。n=7/8仍为8字节，n=9为11字节，n=17为37字节。`count`及分类流条数以十进制字符串保存，避免JSON工具丢失大整数精度。ZIP条目启用ZIP64；每形状仍只存一次。读取新数据应使用根目录或保留其关联清单，不可脱离元数据假定每条8字节。

```text
output_n5_new/
├── dataset.json
├── no_holes/n01_fixed.zip … n05_fixed.zip
└── with_holes/n03_fixed.zip … n05_fixed.zip
```

`fixed` 是兼容历史工具的文件名，文件内容实际为 **one-sided**。分类文件的 nXX 表示最大包围盒边长**恰好为XX**，程序的 n 行表示边长**不超过n**的累计值。空分类不生成文件。裸导出使用同名目录和 `shapes_000001.bin`。

新 `format_version: 2` 每个形状只存入所属分类，不再写 `all_fixed` 副本。清单的 `streams` 记录分类、精确包围盒边长和条数；按清单顺序串接即逻辑全集，`ordering` 为 `category-major-parallel-unspecified`。n=6原始有效载荷3,287,716,896字节（约3.062 GiB）。

导出拒绝非空目录，避免覆盖现有数据。每个成功数据集有独立 `dataset_id`。**形状集合保持兼容，但旧 ZIP 的 global_index 不能直接用于新 ZIP，也不能跨独立运行复用。**外部 Parquet、landmarks 和游戏资产必须绑定原数据集，或重新建立索引。写盘、压缩失败返回非零；失败任务不会生成成功清单，裸文件或 `.zip.partial` 保留用于排查。原生ZIP完成全部归档后才发布正式文件。

现有读取工具仍可使用：

```powershell
python read_shapes.py rust/output_n5_new --info
python read_shapes.py rust/output_n5_new --ascii --limit 10
python read_shapes.py rust/output_n5_new/with_holes/n05_fixed.zip --ascii --limit 10
```

读取器兼容旧v1根目录、独立ZIP/bin和chunk目录；按256 KiB块解码，跨分类读取时惰性打开ZIP。`--info`读取清单与文件元数据，不代表逐条验证通过。

`dedup_check` 是历史辅助工具，只能检查部分性质；完整正确性以回归测试、分类计数和集合对照为准。

## 主要代码

- `rust/src/transfer.rs`：前沿分量状态、增量欧拉特征、差分去平移、旋转轨道。
- `rust/src/dynamic.rs`、`dynamic_transfer.rs`、`dynamic_frontier.rs`、`dynamic_export.rs`：动态尺寸类型、计数、逐形状枚举和v3输出。
- `rust/src/fixed.rs`：两种 BFS、位前沿、分块任务、局部统计和流式导出。
- `rust/src/frontier_export.rs`：共享拓扑图与后缀表、互斥前缀并行回溯。
- `rust/src/redelmeier_export.rs`：无全局去重表的串行半平面生长枚举。
- `rust/src/bit_utils.rs`：位矩阵旋转、平移归一化、欧拉洞检测。
- `rust/src/export.rs`：惰性分配输出槽、新目录保护、可选 ZIP。
- `rust/src/validation.rs`、`rust/tests/cli.rs`：独立 oracle、全集合、CLI及失败路径回归。
- `rust/scripts/benchmark.ps1`：指定旧版 executable 后复现 n≤5 对照基准。
- `rust/scripts/measure-transfer.ps1`：Windows Job 硬内存与超时限制下的计数验证，显式支持 n=6。
- `rust/scripts/measure-export.ps1`：有界完整导出，记录算法、ZIP收尾/压缩、整进程、峰值内存和数据量。
- `rust/scripts/verify-export.rs`：标准库独立裸输出验收器，磁盘分桶全键查重和逐形状语义检查。
- `rust/src/transfer_reference.rs`：冻结的第一批 DP，仅编入测试用于差分校验。

MIT。
