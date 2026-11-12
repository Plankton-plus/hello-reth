# Argus：BLAS 分层可验证日志查询

基于 Reth v2.10.0 的自定义原生区块框架，当前接入 Rust BLAS 方案。节点构建、校验并保存完整区块与窗口索引；Solidity 合约通过 `0xa901` 预编译查询，客户端独立验证链上和文件内证明。

运行、构建、测试、重启及持久化命令见 [RUNBOOK.md](RUNBOOK.md)。迁移说明、完整调用链和实测记录见 [BLAS-MIGRATION.md](BLAS-MIGRATION.md)。本轮仅运行共用小样本，论文规模及 VO 格式差异在 RUNBOOK 中明确记录。
