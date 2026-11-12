//! 开发链使用自定义创世头，协议字段从创世块开始参与区块哈希。

use super::header::IndexedHeader;
use reth_cli::chainspec::ChainSpecParser;
use reth_ethereum::{chainspec::ChainSpec, cli::chainspec::EthereumChainSpecParser};
use std::sync::Arc;

pub type IndexedChainSpec = ChainSpec<IndexedHeader>;

#[derive(Clone, Debug)]
pub struct IndexedChainSpecParser;

impl ChainSpecParser for IndexedChainSpecParser {
    type ChainSpec = IndexedChainSpec;
    const SUPPORTED_CHAINS: &'static [&'static str] = &["dev"];

    /// 为实验开发链解析 value，保留执行规则并替换完整创世头。
    fn parse(value: &str) -> eyre::Result<Arc<IndexedChainSpec>> {
        eyre::ensure!(value == "dev", "当前原型仅支持 dev 实验链");
        let ethereum = EthereumChainSpecParser::parse(value)?;
        Ok(Arc::new(
            ethereum.as_ref().clone().map_header(IndexedHeader::from),
        ))
    }
}
