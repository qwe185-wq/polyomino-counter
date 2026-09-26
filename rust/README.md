# Rust 计数与导出

当前默认计数路线是 **前沿连通性 DP + 旋转轨道 + Burnside**；默认导出使用 **前沿状态图回溯**，每个形状仅写入所属分类。n 是必填参数，支持范围1..6。n=6完整裸导出已在128 MiB Job上限内通过，并逐条验收；普通回归与探针仍不枚举n=6。

```powershell
cargo build --release --locked
cargo run --release -- 5
cargo run --release -- 5 --algorithm bfs
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --algorithm frontier
cargo run --release -- 5 --algorithm redelmeier
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw
cargo run --release -- 5 --export --export-dir output_n5_zip
cargo test --release --locked

# 已有本次 n=6 授权；32 MiB硬提交内存上限，默认30秒超时
./scripts/measure-transfer.ps1 -Exe ./target/release/room-count.exe -N 6 -MemoryMiB 32

# 完整导出：8线程，128 MiB硬上限，默认60秒超时，保留数据与日志
./scripts/measure-export.ps1 -Exe ./target/release/room-count.exe -N 6 -Mode raw
```

完整需求、数据格式、ZIP等级选择、旧 global_index 兼容性和 n=5 性能数据见[项目说明](../README.md)及[性能验证记录](../docs/performance-2026-09-26.md)。

后续[有界 Jensen 优化记录](../docs/jensen-optimization-2026-09-26.md)：n=6算法中位4.777→2.217ms，峰值Job提交内存约12.53MiB；该内存上限由Windows测试脚本执行，直接运行主程序不自动限内存。

完整形状导出的算法取舍、内存与端到端证据见[导出优化记录](../docs/export-optimization-2026-09-26.md)。

`--jensen` 是新DP的兼容别名，旧实验 DFS/Jensen 源码不编入主程序。新数据集必须写入新目录，默认原生流式 ZIP 等级1，无需外部7-Zip；`--compression-backend 7z` 保留原压缩器，`--no-compress` 输出裸文件。原生压缩计入算法耗时，ZIP收尾单独报告；整进程时间用于比较完整导出。

`read_shapes.py` 可直接读取数据集根目录，按 `dataset.json` 中的 `streams` 顺序拼接逻辑全集；旧v1、ZIP和bin输入仍兼容。
