//! BLAS：文件层、块内索引及分段聚合；Reth 框架仅通过 Scheme 调用本模块。
pub mod bitmap;
pub mod block;
pub mod file;
pub mod filters;
pub mod merkle;
pub mod ordered;
pub mod proof;
pub mod query;
pub mod window;

#[cfg(test)]
mod tests;

use crate::{
    chain::BuildContext,
    model::{metadata::IndexedRecord, query::LogQuery},
    query::{
        abi::QueryResponse,
        context::{QueryContext, QueryFailure, WorkBudget},
    },
    scheme::Scheme,
};
use alloy_primitives::B256;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use block::BlockIndex;
use serde::{Deserialize, Serialize};
use window::{BlockSummary, WINDOW_BLOCKS, WindowIndex};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct HeaderFields {
    pub min_tx_time: u64,
    pub max_tx_time: u64,
    pub record_count: u64,
    pub roots: Vec<B256>,
    pub window_size: u64,
}

impl Default for HeaderFields {
    /// 创世字段也认证窗口参数，修改参数必须使用新数据库。
    fn default() -> Self {
        Self {
            min_tx_time: 0,
            max_tx_time: 0,
            record_count: 0,
            roots: vec![],
            window_size: WINDOW_BLOCKS,
        }
    }
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct BodyFields {
    pub index: BlockIndex,
    pub windows: Vec<WindowIndex>,
}

pub struct BlasScheme;

impl Scheme for BlasScheme {
    const ID: u16 = 25;
    const VERSION: u16 = 1;
    type HeaderFields = HeaderFields;
    type BodyFields = BodyFields;

    /// 从 records 和固定父链构建索引；只在完整窗口结束块提交聚合索引。
    fn build(
        context: &BuildContext<'_, HeaderFields, BodyFields>,
        records: &[IndexedRecord],
    ) -> Result<(HeaderFields, BodyFields), String> {
        let index = BlockIndex::build(records.to_vec())?;
        let summary = BlockSummary::new(&index);
        let height = if context.parent_hash == B256::ZERO {
            1
        } else {
            context.header(context.parent_hash)?.number + 1
        };
        let mut windows = Vec::new();
        if height % WINDOW_BLOCKS == 0 {
            let mut cursor = context.parent_hash;
            let mut indexes = Vec::new();
            for offset in 1..WINDOW_BLOCKS {
                let block = context.block(cursor)?;
                if block.header.number != height - offset {
                    return Err("窗口历史块不连续".into());
                }
                cursor = block.header.parent_hash;
                indexes.push(block.fields.index);
            }
            indexes.reverse();
            indexes.push(index.clone());
            windows.push(WindowIndex::build(height - WINDOW_BLOCKS + 1, &indexes)?);
        }
        let mut roots = summary.roots;
        roots.push(windows.first().map(WindowIndex::root).unwrap_or(B256::ZERO));
        Ok((
            HeaderFields {
                min_tx_time: summary.min_time,
                max_tx_time: summary.max_time,
                record_count: records.len() as u64,
                roots,
                window_size: WINDOW_BLOCKS,
            },
            BodyFields { index, windows },
        ))
    }

    /// 检查完整树内容及全部认证根；不能只信任体中缓存的根。
    fn verify_commitment(header: &HeaderFields, body: &BodyFields) -> Result<(), String> {
        if body.windows.len() > 1 || header.window_size != WINDOW_BLOCKS {
            return Err("BLAS 窗口参数非法".into());
        }
        let expected = BlockIndex::build(body.index.records.clone())?;
        if expected != body.index {
            return Err("BLAS 完整索引与记录不一致".into());
        }
        for window in &body.windows {
            window.validate()?;
        }
        let summary = BlockSummary::new(&expected);
        let mut roots = summary.roots;
        roots.push(
            body.windows
                .first()
                .map(WindowIndex::root)
                .unwrap_or(B256::ZERO),
        );
        if header.roots != roots
            || header.record_count != expected.records.len() as u64
            || header.min_tx_time != summary.min_time
            || header.max_tx_time != summary.max_time
        {
            return Err("BLAS 索引承诺或时间边界不一致".into());
        }
        Ok(())
    }

    /// 在相同父链重建当前块和窗口，拒绝同步篡改索引与头部的伪造。
    fn validate(
        context: &BuildContext<'_, HeaderFields, BodyFields>,
        records: &[IndexedRecord],
        header: &HeaderFields,
        body: &BodyFields,
    ) -> Result<(), String> {
        let expected = Self::build(context, records)?;
        if expected.0 != *header || expected.1 != *body {
            return Err("BLAS 索引与交易或父链不一致".into());
        }
        Ok(())
    }

    /// 查询策略和证明全部在 BLAS 内实现，框架仅传递上下文与预算。
    fn query(
        context: &QueryContext<'_, HeaderFields, BodyFields>,
        query: &LogQuery,
        budget: &mut dyn WorkBudget,
    ) -> Result<QueryResponse, QueryFailure> {
        query::execute(context, query, budget)
    }
}
