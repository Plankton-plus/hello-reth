//! 将方案找到的交易位置转换为统一 ABI 返回值。

use crate::{
    chain::BlockView,
    query::{abi::QueryMatch, context::QueryFailure},
};
use alloy_eips::Encodable2718;
use alloy_primitives::keccak256;

/// 从 block 的 positions 提取原始签名交易；越界表示索引损坏。
pub fn matches<H, B>(
    block: &BlockView<H, B>,
    positions: Vec<u32>,
) -> Result<Vec<QueryMatch>, QueryFailure> {
    positions
        .into_iter()
        .map(|position| {
            let transaction = block
                .transactions
                .get(position as usize)
                .ok_or_else(|| QueryFailure::Storage("方案返回的交易位置越界".into()))?;
            let raw = transaction.encoded_2718();
            Ok(QueryMatch {
                blockHash: block.header.hash,
                transactionIndex: position,
                transactionHash: keccak256(&raw),
                rawTransaction: raw.into(),
            })
        })
        .collect()
}
