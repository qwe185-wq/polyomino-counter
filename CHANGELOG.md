# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- Rust 实现：BFS + Burnside 引理（rayon 并行，1024 分片 hashset，mimalloc）
- Rust 实验算法：Redelmeier DFS、Jensen 转移矩阵
- Rust 单元测试（17 个）
- n=6 结果：410,964,612（328s，~8GB）

### Changed
- 目录分离：C 代码移入 `C/`，Rust 代码在 `rust/`
- 文档归入对应子目录（C/docs/、rust/docs/）
- README 更新项目结构和构建说明
