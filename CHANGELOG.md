# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Complete export follow-up (2026-09-26)
- 完整导出改为前沿状态图回溯，按宽共享拓扑图，前缀并行且不保存全局形状集合；保留两种BFS和新Redelmeier作为参考。
- `format_version: 2` 每形状只存所属分类，以清单定义逻辑全集；不再重复写all副本。读取器兼容新旧格式并按256 KiB分批解码。
- 默认原生流式ZIP，支持 `--compression-backend native|7z`；256 KiB缓冲、失败保留partial，成功才写数据集清单。
- canonical BFS减少旋转，按父形状复用旋转结果；独立坐标oracle验证成长公式。
- n=6完整裸导出单次整进程5.27秒、峰值Job提交内存96.25 MiB，128 MiB硬上限；410,964,612条逐条合法性、分类及全键查重验收通过。
- 提供有界导出复现脚本与[详细证据](docs/export-optimization-2026-09-26.md)；纯计数约2.2ms不包含形状导出。

### Jensen follow-up (2026-09-26)
- 同宽矩形共享逐行扫描，已关闭形状移入累计器；拓扑状态采用紧凑键和预计算转移图。
- χ≤0的非空前缀合并为有洞吸收桶，计数循环使用数组和稀疏活跃索引。
- 新增Windows Job内存/超时受限基准，严格校验n=1..N的三分类结果并拒绝覆盖证据目录。
- 用户授权后复验n=6：算法中位4.777→2.217ms；最终版通过32MiB硬内存限制。38项Release及38项Debug回归通过。
- 完整进程耗时未测出明确改善；此次不包含n=6导出加速。逐项证据见 `docs/jensen-optimization-2026-09-26.md`。

### Performance (2026-09-26)
- 默认精确计数改为前沿DP、独立旋转轨道及Burnside；n=5内部算法中位0.570ms（旧版205ms）。
- 导出默认直接枚举one-sided等价类，加入位前沿、位旋转、欧拉洞判定、局部归并统计和有界写缓冲。
- ZIP默认等级1；支持 `--compression-level 9` 和 `--no-compress`，分别报告算法、压缩和总时间。
- 新增独立穷举、三路线计数、完整导出集合、CLI和错误路径验证；本轮未运行n=6。

### Changed (2026-09-26)
- n改为必填并严格校验；`--algorithm auto|transfer|bfs|canonical`。
- 旧错误DFS/Jensen退出生产入口；`--jensen`兼容新DP，移除`--dfs`。
- 拒绝非空导出目录并写入独立数据集清单；旧global_index不能直接用于新并行输出。
- 写盘和压缩失败返回非零，不将部分导出报告为成功。

### Added
- **One-sided 二进制导出**（`--export`）：枚举时内联计算 One-sided canonical（4 旋转取最小），流式写入分块 `.bin` 文件
- **7-zip 自动压缩**：导出完成后自动调用 7-zip，删除原始 `.bin` 目录
- **并行去重验证**（`dedup_check`）：两阶段算法——hash 分片 + rayon 排序，支持跨文件查重
- **形状提取脚本**（`read_shapes.py`）：从 `.zip` 流式读取，`--info` 零解压，支持 ASCII/文本/范围筛选
- **分块存储**：每 10M 个 mask 一个独立文件（80MB），zip 内可随机解压单个 chunk
- n=6 One-sided 数据集：410,964,612 形状，1.01 GB 压缩（42 chunks，已去重验证）

### Changed
- 枚举引擎增加 One-sided 去重层（`compute_canonical` + 独立哈希集）
- 流式导出：通过 `sync_channel` 避免大片数据积压内存
- `.gitignore`：排除 `output_*/` `rust/output_*/`

---

### 历史版本

#### 2026-08-09 — Rust 实现 + Burnside 引理

**Added**
- Rust 实现：BFS + Burnside 引理（rayon 并行，1024 分片 hashset，mimalloc）
- Rust 实验算法：Redelmeier DFS、Jensen 转移矩阵
- Rust 单元测试
- n=6 结果：410,964,612（328s，~8GB）

**Changed**
- 目录分离：C 代码移入 `C/`，Rust 代码在 `rust/`
- 文档归入对应子目录

#### 2026-08-08 — C 实现 + 性能优化迭代

**Performance**
- OpenMP 并行化（形状级 parallel for）
- 全局方向集跳过规范化 + `__builtin_ctzll` 位扫描
- 分块数组、面积余额、包围盒满跳过、不变量预筛选
- 增强计时模块 + 不变分桶实验
- hashset 接口清理（移除 CAS 实验）

**Added**
- 项目骨架：polyomino 枚举引擎（Redelmeier 生长法 + One-sided canonical）
- n=1..5 实测结果验证
