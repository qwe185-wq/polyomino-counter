# Polyomino Counter / Rust — 动态尺寸、任意精度与旋转商图研究

**当前分支：`codex/dynamic-n-20260926`。** 在 main 的动态计数和导出能力上，继续研究旋转固定集商图，加入紧凑标签、数值特化和 split/single/two/center 四种显式商图模式。

第二轮改进主要作用于显式 quotient 内核；默认 auto 的内核选择未改变。局部内核速度与内存收益不等于完整 n 计数的同等收益，也不代表已完成 n=14。

[分支导航](../docs/BRANCHES.md) · [完整项目说明](../README.md)

当前默认计数路线是 **前沿连通性 DP + 旋转轨道 + Burnside**；默认导出使用 **前沿状态图回溯**，每个形状仅写入所属分类。n 是必填正整数；n≤6使用既有优化路线，n>6自动切换动态前沿、动态位图与BigUint精确计数，无小尺寸硬上限。n=6完整裸导出已验证，后续完整纯计数已记录到n=13（验证强度见根目录README）；普通回归不启动完整n≥6枚举。

```powershell
cargo build --release --locked
cargo run --release -- 5
cargo run --release -- 7
cargo run --release -- 9 --algorithm transfer
cargo run --release -- 5 --algorithm bfs
cargo run --release -- 5 --algorithm canonical
cargo run --release -- 5 --algorithm frontier
cargo run --release -- 5 --algorithm redelmeier
cargo run --release -- 5 --export --no-compress --export-dir output_n5_raw
cargo run --release -- 5 --export --export-dir output_n5_zip
cargo test --release --locked

# 显式选择 n=6，使用脚本的硬资源上限；32 MiB硬提交内存上限，默认30秒超时
./scripts/measure-transfer.ps1 -Exe ./target/release/room-count.exe -N 6 -MemoryMiB 32

# 完整导出：8线程，128 MiB硬上限，默认60秒超时，保留数据与日志
./scripts/measure-export.ps1 -Exe ./target/release/room-count.exe -N 6 -Mode raw
```

完整需求、数据格式、ZIP等级选择、旧 global_index 兼容性和 n=5 性能数据见[项目说明](../README.md)及[性能验证记录](../docs/performance-2026-09-26.md)。

后续[有界 Jensen 优化记录](../docs/jensen-optimization-2026-09-26.md)：n=6算法中位4.777→2.217ms，峰值Job提交内存约12.53MiB；该内存上限由Windows测试脚本执行，直接运行主程序不自动限内存。

完整形状导出的算法取舍、内存与端到端证据见[导出优化记录](../docs/export-optimization-2026-09-26.md)。

`--jensen` 是新DP的兼容别名，旧实验 DFS/Jensen 源码不编入主程序。新数据集必须写入新目录，默认原生流式 ZIP 等级1，无需外部7-Zip；`--compression-backend 7z` 保留原压缩器，`--no-compress` 输出裸文件。原生压缩计入算法耗时，ZIP收尾单独报告；整进程时间用于比较完整导出。

`read_shapes.py` 可直接读取数据集根目录，按 `dataset.json` 中的 `streams` 顺序拼接逻辑全集；旧v1、ZIP和bin输入仍兼容。

中心商图的第二轮研究与实验见[商图及抽象结构调研](../docs/fixedset-round2-2026-09-26.md)。显式选择 `--symmetry-engine quotient` 或运行 `quotient_probe` 时，可用环境变量 `ROOM_COUNT_QUOTIENT_MODE` 比较四种精确计数模式：`split`（默认，原中心图案分支）、`single`（不拆中心的单遍商图）、`two`（偶×偶半转拆中心全空/非空）、`center`（偶×偶半转拆中心全满/不全满，后者利用永久洞证明省χ）。其他尺寸下 `two/center` 沿用原两分支；这些模式只影响显式商图内核，自动内核选择不变。

```powershell
$env:ROOM_COUNT_QUOTIENT_MODE='single'
./target/release/quotient_probe.exe 12 12 half
Remove-Item Env:ROOM_COUNT_QUOTIENT_MODE
```

n>6使用v3清单（`uint-le-fixed`、动态`stride/record_bytes`、十进制字符串计数、ZIP64）。已实测n=7总数1,185,652,433,093，与旧参考DP一致；n=8纯计数总数12,575,973,909,316,634，优化前后分类一致。本轮600秒/4GiB受控测试中n=13用122.40秒完成，n=14触发内存限制未完成；结果及新选项见[分阶段优化记录](../docs/fixedset-performance-2026-09-26.md)。n≥7全集导出仍未完成验证。大轨道集改用保留原格连通标签的DP，小轨道集沿用Gray位运算；超过单字宽度的通用位图内核使用动态`Vec<u64>`，不设小尺寸上限。边界与早期验证详见[动态尺寸记录](../docs/dynamic-dimensions-2026-09-26.md)，历史位运算改进见[大尺寸计数优化](../docs/large-count-optimization-2026-09-26.md)，最新算法、调研取舍与性能见[旋转固定集优化](../docs/symmetric-fixedset-optimization-2026-09-26.md)。旧参考算法 `bfs/canonical/redelmeier` 与历史有界基准脚本仍只处理n≤6；动态入口使用 `auto/transfer/frontier`，不套用资源限制。
