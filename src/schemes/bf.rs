//! BF 方案：关键词树与生成器倒排索引，构建和查询策略均局限在本模块。

use crate::{
    chain::BuildContext,
    model::{metadata::IndexedRecord, query::LogQuery},
    query::{
        abi::QueryMatch,
        context::{QueryContext, QueryFailure, WorkBudget},
        result,
    },
    scheme::Scheme,
};
use alloy_primitives::keccak256;
use fields::{BfBodyFields, BfHeaderFields, GeneratorEntry};
use std::collections::BTreeMap;

pub mod fields;
pub mod tree;

/// 当前用于验证框架接入的最小具体方案。
pub struct BfScheme;

impl Scheme for BfScheme {
    const ID: u16 = 1;
    const VERSION: u16 = 2;
    type HeaderFields = BfHeaderFields;
    type BodyFields = BfBodyFields;

    /// 从 records 构建两份索引；context 提供可用于块间聚合的父链视图。
    fn build(
        _context: &BuildContext<'_, BfHeaderFields, BfBodyFields>,
        records: &[IndexedRecord],
    ) -> Result<(BfHeaderFields, BfBodyFields), String> {
        let mut generators = BTreeMap::<String, Vec<u32>>::new();
        for record in records {
            generators
                .entry(record.metadata.lid.clone())
                .or_default()
                .push(record.transaction_index);
        }
        let body = BfBodyFields {
            keyword_tree: tree::BfTree::build(records.to_vec()).encoded().into(),
            generator_index: generators
                .into_iter()
                .map(|(lid, positions)| GeneratorEntry { lid, positions })
                .collect(),
        };
        let header = BfHeaderFields {
            min_tx_time: records.iter().map(|r| r.metadata.ts_min).min().unwrap_or(0),
            max_tx_time: records.iter().map(|r| r.metadata.ts_max).max().unwrap_or(0),
            record_count: records.len() as u64,
            roots: roots(&body),
        };
        Ok((header, body))
    }

    /// roots 的固定顺序为关键词树、生成器倒排索引；两者均须完整存在。
    fn verify_commitment(header: &BfHeaderFields, body: &BfBodyFields) -> Result<(), String> {
        if body.keyword_tree.is_empty() || header.roots != roots(body) {
            return Err("完整索引缺失或索引根集合不匹配".into());
        }
        Ok(())
    }

    /// 用相同父链 context 重建 records，对比全部方案头字段及复合体字段。
    fn validate(
        context: &BuildContext<'_, BfHeaderFields, BfBodyFields>,
        records: &[IndexedRecord],
        header: &BfHeaderFields,
        body: &BfBodyFields,
    ) -> Result<(), String> {
        let expected = Self::build(context, records)?;
        if header != &expected.0 || body != &expected.1 {
            return Err("索引或头部摘要与交易元数据不一致".into());
        }
        Ok(())
    }

    /// BF 方案逐块检查头部时间，再用生成器索引和关键词树查询候选块。
    fn query(
        context: &QueryContext<'_, BfHeaderFields, BfBodyFields>,
        query: &LogQuery,
        budget: &mut dyn WorkBudget,
    ) -> Result<Vec<QueryMatch>, QueryFailure> {
        let mut hash = context.end_hash;
        let mut chunks = Vec::new();
        for height in (context.start..=context.end).rev() {
            let header = context.header(hash, budget)?;
            if header.number != height {
                return Err(QueryFailure::Storage("查询分支高度不连续".into()));
            }
            if header.fields.record_count > 0
                && header.fields.min_tx_time <= query.time.1
                && header.fields.max_tx_time >= query.time.0
            {
                let block = context.block(hash, budget)?;
                let candidates: Vec<u32> = block
                    .fields
                    .generator_index
                    .iter()
                    .filter(|entry| {
                        query.generators.is_empty() || query.generators.contains(&entry.lid)
                    })
                    .flat_map(|entry| entry.positions.iter().copied())
                    .collect();
                if !candidates.is_empty() {
                    let tree = tree::BfTree::decode(&block.fields.keyword_tree)
                        .map_err(QueryFailure::Storage)?;
                    chunks.push(result::matches(&block, tree.query(query, &candidates))?);
                }
            }
            hash = header.parent_hash;
        }
        Ok(chunks.into_iter().rev().flatten().collect())
    }
}

/// 计算 body 中各个完整索引的摘要，顺序属于方案协议。
fn roots(body: &BfBodyFields) -> Vec<alloy_primitives::B256> {
    vec![
        keccak256(&body.keyword_tree),
        keccak256(alloy_rlp::encode(&body.generator_index)),
    ]
}
