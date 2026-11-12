//! Argus：统一数据模型、方案接口、原生 Reth 区块和合约查询框架。

pub mod chain; //只读链视图
pub mod model; //统一数据模型
pub mod node; //Reth节点组件（区块、存储、执行、校验、RPC）
pub mod query; //合约查询框架
pub mod scheme;//方案接口
pub mod schemes; //具体方案实现

#[cfg(test)]
pub(crate) mod test_support;
