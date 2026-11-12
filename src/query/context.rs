//! 查询快照和预算：方案可以选择块间算法，所有读取都限定在认证分支。

use crate::chain::{BlockView, ChainReader, HeaderView};
use alloy_primitives::B256;
use alloy_rlp::Encodable;
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum QueryFailure {
    Invalid(String),
    Storage(String),
    OutOfGas,
}

pub trait WorkBudget {
    /// 为本次确定性工作量 gas 计费；耗尽时中止查询。
    fn spend(&mut self, gas: u64) -> Result<(), QueryFailure>;
}

pub struct QueryContext<'a, H, B> {
    pub start: u64,
    pub end: u64,
    pub end_hash: B256,
    reader: &'a ChainReader<H, B>,
    ancestors: BTreeMap<B256, u64>,
}

impl<'a, H, B: Encodable> QueryContext<'a, H, B> {
    /// 创建已由 EVM 历史哈希认证的 ancestors 视图，不查询 latest。
    pub fn new(
        start: u64,
        end: u64,
        end_hash: B256,
        reader: &'a ChainReader<H, B>,
        ancestors: BTreeMap<B256, u64>,
    ) -> Self {
        Self {
            start,
            end,
            end_hash,
            reader,
            ancestors,
        }
    }

    /// 检查 hash 属于查询分支，再按固定费用读取头部。
    pub fn header(
        &self,
        hash: B256,
        budget: &mut dyn WorkBudget,
    ) -> Result<HeaderView<H>, QueryFailure> {
        let height = self
            .ancestors
            .get(&hash)
            .ok_or_else(|| QueryFailure::Invalid("读取超出查询快照".into()))?;
        budget.spend(500)?;
        let header = self.reader.header(hash).map_err(QueryFailure::Storage)?;
        if header.hash != hash || header.number != *height {
            return Err(QueryFailure::Storage("历史头部与认证快照不一致".into()));
        }
        Ok(header)
    }

    /// 读取完整块并按索引编码长度计费；不决定方案如何挑选候选块。
    pub fn block(
        &self,
        hash: B256,
        budget: &mut dyn WorkBudget,
    ) -> Result<BlockView<H, B>, QueryFailure> {
        self.header(hash, budget)?;
        let block = self.reader.block(hash).map_err(QueryFailure::Storage)?;
        let length = block.fields.length();
        if length > 65536 {
            return Err(QueryFailure::Invalid("单块索引超过 64 KiB".into()));
        }
        budget.spend(5000 + 8 * length as u64)?;
        Ok(block)
    }
}
