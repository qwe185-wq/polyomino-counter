# Rust 计数与导出

当前默认计数路线是 **前沿连通性 DP + 旋转轨道 + Burnside**；默认导出使用并行 **one-sided BFS**。n 是必填参数，支持范围1..6；本轮改造只验证到5，n=6需用户另行允许。

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
