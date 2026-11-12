//! 将 Reth provider 转成方案无关的头部和完整区块读取器。

use crate::{
    chain::{BlockView, HeaderView},
    node::{block::IndexedBlock, header::IndexedHeader},
    scheme::{HeaderFields, SchemeReader},
};
use alloy_primitives::{B256, Sealable};
use reth_ethereum::provider::BlockReader;
use std::sync::Arc;

pub type IndexReader = SchemeReader;

/// 认证 header 的完整哈希，再提取方案可见的公共字段。
fn header_view(hash: B256, header: IndexedHeader) -> Result<HeaderView<HeaderFields>, String> {
    if header.hash_slow() != hash {
        return Err("完整自定义区块头哈希不匹配".into());
    }
    Ok(HeaderView {
        hash,
        parent_hash: header.parent_hash,
        number: header.number,
        fields: header.fields,
    })
}

/// 从 provider 构造只读适配器；调用方使用快照上下文限制可访问分支。
pub fn reader<P>(provider: P) -> SchemeReader
where
    P: BlockReader<Block = IndexedBlock, Header = IndexedHeader> + Clone + Send + Sync + 'static,
{
    let headers = provider.clone();
    SchemeReader::new(
        Arc::new(move |hash| {
            headers
                .header(hash)
                .map_err(|e| e.to_string())?
                .map(|header| header_view(hash, header))
                .transpose()
        }),
        Arc::new(move |hash| {
            provider
                .block_by_hash(hash)
                .map_err(|e| e.to_string())?
                .map(|block| {
                    block.validate_commitment()?;
                    Ok(BlockView {
                        header: header_view(hash, block.header)?,
                        fields: block.body.fields,
                        transactions: block.body.ethereum.transactions,
                    })
                })
                .transpose()
        }),
    )
}
