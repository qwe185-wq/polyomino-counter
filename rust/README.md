# Polyomino Counter / Rust — 默认前沿 DP 与并行形状枚举优化

**当前分支：`codex/performance-20260926`。** 将精确前沿 DP 接入默认纯计数入口，引入 one-sided BFS、可选算法、可调 ZIP 压缩和读取器兼容性验证。

尺寸范围为 1..6；本分支这一轮回归、性能和导出对照只验证到 n=5。ZIP 压缩依赖外部 7-Zip，尚无原生流式 ZIP 或动态尺寸。

[分支导航](../docs/BRANCHES.md) · [完整项目说明](../README.md)

当前默认计数路线是 **前沿连通性 DP + 旋转轨道 + Burnside**；默认导出使用并行 **one-sided BFS**。n 是必填参数，支持范围1..6；本轮改造只验证到5，本分支未重验n=6。

```powershell
cargo build --release --locked
cargo run --release -- 5
cargo run --release -- 5 --algorithm bfs
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw
cargo run --release -- 5 --export --export-dir output_n5_zip
cargo test --release --locked
```

完整需求、数据格式、ZIP等级选择、旧 global_index 兼容性和 n=5 性能数据见[项目说明](../README.md)及[性能验证记录](../docs/performance-2026-09-26.md)。

`--jensen` 是新DP的兼容别名，旧的实验 DFS/Jensen 源码不再编入主程序。新数据集必须写入新目录，默认 ZIP 等级1；`--compression-level 9` 可换取更高压缩率。没有7-Zip时使用 `--no-compress`。
