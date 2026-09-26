# 动态尺寸实现与验证

上一批导出优化 `ba3de9286edaf964214f10a3ae62b26ac448bfc6` 已获用户确认并快进应用至主目录。本批以该提交为固定基线，在原任务工作树的 `codex/dynamic-n-20260926` 分支开发。

## 范围与路由

用户明确选择动态表示，支持n=7、8、9…，不人为设置小尺寸上限，也不以资源预算限制功能。CLI接受可表示的正整数，校验尺寸乘加不溢出；没有内存/时间门槛。n≤6默认路线及v2格式保留，n>6的`auto/transfer/frontier`调用通用实现。旧固定表示的`bfs/canonical/redelmeier`继续作为n≤6参考算法，超出时给出明确替代入口，不静默更换指定算法。

## 实现

- `dynamic_transfer.rs`：Vec前沿标签、动态状态键和usize节点索引；BigUint保存DP权重、分类数与Burnside结果。关闭分量累计，χ≤0吸收，宽度相关状态动态分配。旋转轨道的Gray计数器也是动态数组，不使用`1usize << orbit_count`。n≤7的对称子问题仍可安全使用stride8/u64；更大尺寸按坐标动态检查。
- `dynamic_frontier.rs`：按宽共享拓扑，后缀可行性共享但逐条前缀位图不合并，显式栈避免递归深度成为尺寸门槛。BigUint存mask和计数，通用路线当前串行。小尺寸并行优化路径不变。
- 满宽位图不能直接沿用旧Euler位移：当stride==width时先重排出一列空隙，避免最后一列串到下一行。先复现实心8×8被误报洞，再修复并对照独立背景洪泛。
- `dynamic_export.rs`：分类槽按实际数据惰性创建，slot总数与chunk编号均为BigUint。动态记录宽度的缓冲刷新用`>=`，支持记录宽度不整除256 KiB。ZIP64、失败无成功清单、非空目录保护，清单写入partial后发布。裸输出、native ZIP和外部7z均可选。
- `read_shapes.py`：根据清单设置每条记录宽度和几何步长，保留v1/v2默认；直接打开关联的分类ZIP/目录/bin也读取根清单。支持超过六位的chunk编号并按整数排序；v3拒绝重复编号、异常BIN及未使用高位。

## v3格式

```json
{
  "format_version": 3,
  "max_n": 9,
  "stride": 9,
  "record_bytes": 11,
  "encoding": "uint-le-fixed",
  "count": "十进制总数",
  "storage_layout": "classified-single-copy"
}
```

完整清单还包括dataset_id、equivalence、representative、category_dimension、ordering、streams与complete，语义延续v2。`count`和每个流的`count`为十进制字符串；尺寸字段仍为JSON整数。记录是无符号小端固定宽整数，bit=`row*stride+col`，紧包围盒左上归一化，取C4数值最小代表。`stride=max(8,n)`，`record_bytes=max(8,ceil(n*stride/8))`。n=7/8、9、17分别使用8、11、37字节。

每条记录只在一个有洞/无洞及精确最大边长分类中；按清单顺序拼接逻辑全集。不兼容假设所有记录都为8字节的第三方程序，新数据集索引不可跨运行沿用。旧用户未跟踪脚本未改动。

## 实际验证

- Release与Debug各67项测试通过；Python 15项测试通过。
- 动态计数n≤5与原优化计数全部一致。真正的DP `3×40` 计数超过`2^80`：中列全满，左右80个格子任意选择均连通，提供独立下界，证明实际累加未截断到u64。
- `placement_rows(8,1)`、`(9,1)`分别得到36、45种无洞放置；1×8/9、8×9轨道、超过64个轨道的Gray进位均有验证。
- 动态枚举n≤4的2404个最小代表及分类逐条等于旧frontier；9格长条的旋转跨bit64，9×9环跨bit80。
- 实心、缺角、环形与随机连通满框8×8/9×9，洞分类与独立四邻接洪泛一致。
- Rust生成完整n=4 v3 raw/ZIP，Python逐条读取后的集合均等于旧v2全集；n=9的实心/十字/环三个**样例夹具**验证81位记录、11字节长度、分类、ASCII及独立分类流读取。这些样例不是n=9全集。
- 实际运行n=7默认CLI，得到总数 **1,185,652,433,093**、无洞 **144,608,553,854**、有洞 **1,041,043,879,239**。另在独立临时入口使用冻结旧参考DP（仅将尺寸常量设为7）计算，n=1..7三分类逐行一致。
- n=7一次算法计时约0.251秒，既非中位数，也不包含逐形状导出。未执行n≥8整盘计数或n≥7完整形状导出，不将接口支持宣称为这些全集已经验收。
- Sol high独立只读审查未发现确定性缺陷；额外独立穷举3×3、4×4全部四连通位图的Euler/背景洪泛一致。

## 使用

```powershell
cd rust
cargo build --release --locked
./target/release/room-count.exe 7
./target/release/room-count.exe 9 --algorithm transfer
./target/release/room-count.exe 7 --export --no-compress --export-dir output_n7_new
./target/release/room-count.exe 9 --export --export-dir output_n9_new
# 在仓库根目录读取；自动选择清单中的步长与记录宽度
python read_shapes.py rust/output_n9_new --info
python read_shapes.py rust/output_n9_new --ascii --limit 10
```

历史`measure-transfer.ps1`、`measure-export.ps1`及独立u64验收器仍定位为n≤6的基准工具，不作为动态主程序的限制。

证据根目录：`C:/Users/wangj/.codex/visualizations/2026/09/26/01a0dba5-62e9-7182-b799-714526f6f773`。日志包括 `dynamic-release-tests.log`、`dynamic-debug-tests.log`、`dynamic-python-tests.log`、`dynamic-n7.log`、`reference-n7.log`、`dynamic-hole-red.log`、`dynamic-hole-green.log`、`dynamic-roundtrip.log` 和 `dynamic-review.json`；临时reference/probe源代码与明确命名的样例夹具同目录保留。
