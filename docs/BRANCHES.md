# 分支导航与版本选择

项目 GitHub 名称为 **polyomino-counter**，Rust 包和命令仍名为 `room-count`。本仓库按最大包围盒边长 n 计数；n 不是多联骨牌的格子数。每个分支的 README 标明该版本可用的入口、算法与验证边界。

| 分支 | 标题与定位 | 主要差异 |
|---|---|---|
| [main](https://github.com/qwe185-wq/polyomino-counter/tree/main) | 集成版：精确计数与数据集导出 | 动态尺寸、BigUint、v2/v3、缓存与诊断；推荐入口 |
| [codex/transfer-20260926](https://github.com/qwe185-wq/polyomino-counter/tree/codex/transfer-20260926) | 前沿 DP 原型 | 独立 transfer_probe；主 CLI 仍默认 BFS |
| [codex/performance-20260926](https://github.com/qwe185-wq/polyomino-counter/tree/codex/performance-20260926) | 默认计数与枚举性能 | 默认 DP、one-sided BFS、外部 ZIP；1≤n≤6 |
| [codex/jensen-20260926](https://github.com/qwe185-wq/polyomino-counter/tree/codex/jensen-20260926) | 转移状态与内存优化 | 状态压缩、拓扑复用、有洞吸收；n=6 有界计数 |
| [codex/export-20260926](https://github.com/qwe185-wq/polyomino-counter/tree/codex/export-20260926) | 完整分类导出优化 | frontier / Redelmeier、原生 ZIP、v2 单份存储 |
| [codex/dynamic-n-20260926](https://github.com/qwe185-wq/polyomino-counter/tree/codex/dynamic-n-20260926) | 动态尺寸与商图研究 | main 能力 + 四种显式 quotient 模式和第二轮研究 |

## 如何选择

首次使用从 `main` 开始。复现实验时切换到对应分支，使用该分支的 `Cargo.lock`、README 和性能记录，不将主分支参数直接套到旧分支。分支之间既有连续演进，也有独立实验与重复引入的提交；本表不是严格的线性合并关系。

```powershell
git clone https://github.com/qwe185-wq/polyomino-counter.git
cd polyomino-counter
git branch -a
git switch --track origin/codex/export-20260926
cd rust
cargo build --release --locked
```

上例 `--track` 适用于本地尚不存在该分支的首次检出；已有本地分支时使用 `git switch 分支名`。

## 共同的问题定义

- 对象是能嵌入 n×n 正方形网格的非空、四邻接连通格子集合。
- 合并平移及 0°/90°/180°/270° 旋转；不合并镜像，属于 one-sided polyomino。
- 空格同样按四邻接判断能否连通外部；对角接触不形成通道。
- 同时输出总数、无洞数、有洞数。只计形状，不额外计数门或墙的布置。
- 主程序输出的 n 行是包围盒边长不超过 n 的累计计数；导出分类中的 nXX 为精确最大包围盒边长。

## 历史与发布范围

首次 GitHub 发布保留原有 6 个分支和其可达的 44 个历史提交，文档改进作为各分支的新提交追加，不重写作者、日期或历史消息。发布前本地没有 Git 标签。保留原分支名以便对照旧记录；分支的人类可读标题和用途以 README 为准。

Git 跟踪的源码、锁文件、测试、计划和研究记录随历史上传。`main` 额外收录发布时尚未跟踪的 `extract_landmarks.py` 原稿。构建目录、Python 缓存和被 `.gitignore` 排除的生成数据不属于源码发布；本仓库不附带完整 n=6 或更大数据集。

历史文档中的本机路径、实验授权及执行环境是当时的复现记录。基准数据只适用于记录的代码、硬件、线程、构建与限制条件。形状全集的导出规模增长很快；动态类型支持更大的 n，不代表任意 n 都能在有限时间和内存内完成。
