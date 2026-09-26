# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

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
