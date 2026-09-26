# Polyomino Counter / Rust — 前沿转移状态压缩与有界计数优化

**当前分支：`codex/jensen-20260926`。** 在前沿 DP 上复用相邻矩形扫描和转移缓冲区，压缩状态键，分离拓扑与欧拉特征累计，并吸收已确定有洞的历史。

n=6 纯计数通过 32 MiB Windows Job 硬限制；导出仍使用 one-sided BFS 和外部 7-Zip。支持尺寸为 1..6。

[分支导航](../docs/BRANCHES.md) · [完整项目说明](../README.md)

当前默认计数路线是 **前沿连通性 DP + 旋转轨道 + Burnside**；默认导出使用并行 **one-sided BFS**。n 是必填参数，支持范围1..6。n=6纯计数已获授权并通过有界运行验证；普通回归与探针仍默认只跑到5。

```powershell
cargo build --release --locked
cargo run --release -- 5
cargo run --release -- 5 --algorithm bfs
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw
cargo run --release -- 5 --export --export-dir output_n5_zip
cargo test --release --locked

# 显式选择 n=6，使用脚本的硬资源上限；32 MiB硬提交内存上限，默认30秒超时
./scripts/measure-transfer.ps1 -Exe ./target/release/room-count.exe -N 6 -MemoryMiB 32
```

完整需求、数据格式、ZIP等级选择、旧 global_index 兼容性和 n=5 性能数据见[项目说明](../README.md)及[性能验证记录](../docs/performance-2026-09-26.md)。

后续[有界 Jensen 优化记录](../docs/jensen-optimization-2026-09-26.md)：n=6算法中位4.777→2.217ms，峰值Job提交内存约12.53MiB；该内存上限由Windows测试脚本执行，直接运行主程序不自动限内存。

`--jensen` 是新DP的兼容别名，旧的实验 DFS/Jensen 源码不再编入主程序。新数据集必须写入新目录，默认 ZIP 等级1；`--compression-level 9` 可换取更高压缩率。没有7-Zip时使用 `--no-compress`。
