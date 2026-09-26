# Polyomino Counter / Rust — 前沿 DP 原型与历史 BFS

当前分支为 `codex/transfer-20260926`。新增的前沿 DP 位于 `src/transfer.rs`，通过独立 `transfer_probe` 运行；旧主程序 `room-count` 尚未切换默认算法。它没有后续版本的 `--algorithm` 参数，旧 `--jensen` 也不是新 transfer 的入口。

在本目录运行：

```powershell
cargo build --release --locked
cargo run --release --locked --bin transfer_probe
cargo test --release --locked --bin transfer_probe
cargo run --release --locked --bin room-count -- 3
cargo run --release --locked --bin room-count -- 3 --export --export-dir output_n3_new
```

前沿探针固定运行 n=1..5；旧主程序支持 n≤6，无参数会默认启动较重的 n=6 BFS，因此示例显式使用 n=3。导出 ZIP 依赖 7-Zip，尚无原生 ZIP 或 `--no-compress`。新用户建议使用 main。

问题定义、结果、格式和限制见[根目录 README](../README.md)，各版本差异见[分支导航](../docs/BRANCHES.md)。[早期交接文档](docs/HANDOFF-RUST.md)保留当时的实验状态，不代表后续集成版本。
