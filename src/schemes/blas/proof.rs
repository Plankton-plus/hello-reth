//! 查询 VO 的独立验证器；输入仅为可信头字段、查询、结果和证明。
use super::{
    HeaderFields,
    bitmap::Bitmap,
    block::IndexProof,
    merkle::MerkleProof,
    ordered::decode,
    window::{WINDOW_BLOCKS, WindowProof},
};
use crate::{
    model::{metadata::IndexedRecord, query::LogQuery},
    query::abi::QueryMatch,
};
use alloy_primitives::{B256, keccak256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedHeader {
    pub hash: B256,
    pub number: u64,
    pub fields: HeaderFields,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockWitness {
    pub hash: B256,
    pub index: IndexProof,
    pub keywords: Vec<Option<MerkleProof>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GroupProof {
    Block {
        block: BlockWitness,
    },
    Window {
        end_hash: B256,
        window: WindowProof,
        blocks: Vec<BlockWitness>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryProof {
    pub groups: Vec<GroupProof>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifiedRecord {
    pub block_hash: B256,
    pub record: IndexedRecord,
}

impl BlockWitness {
    /// 对单块认证过滤位图及所有命中对象；aggregated 由窗口目录认证后提供。
    fn verify(
        &self,
        header: &TrustedHeader,
        query: &LogQuery,
        aggregated: Option<&BTreeMap<String, Bitmap>>,
    ) -> Result<Vec<VerifiedRecord>, String> {
        if self.hash != header.hash
            || header.fields.roots.len() != 6
            || header.fields.record_count > 256
            || header.fields.window_size != WINDOW_BLOCKS
            || (aggregated.is_none() && !self.keywords.is_empty())
        {
            return Err("VO 区块与可信头不一致".into());
        }
        let count = header.fields.record_count as usize;
        let bitmap = self.index.filters.verify_with_keywords(
            &header.fields.roots[1..5],
            count,
            query,
            false,
            aggregated,
        )?;
        let mut records = Vec::new();
        for leaf in self.index.objects(header.fields.roots[0], count, &bitmap)? {
            let record: IndexedRecord = decode(&leaf)?;
            if !query.matches(&record.metadata)
                || records.last().is_some_and(|last: &VerifiedRecord| {
                    last.record.transaction_index >= record.transaction_index
                })
            {
                return Err("VO 交易内容、顺序或查询条件不一致".into());
            }
            records.push(VerifiedRecord {
                block_hash: self.hash,
                record,
            });
        }
        Ok(records)
    }
}

impl QueryProof {
    /// headers 必须由调用方认证并覆盖整个请求范围；遗漏块或结果均拒绝。
    pub fn verify(
        &self,
        headers: &[TrustedHeader],
        query: &LogQuery,
    ) -> Result<Vec<VerifiedRecord>, String> {
        if headers.is_empty()
            || headers.len() > 32
            || headers
                .windows(2)
                .any(|pair| pair[0].number + 1 != pair[1].number)
        {
            return Err("可信头范围非法".into());
        }
        let mut cursor = 0;
        let mut records = Vec::new();
        for group in &self.groups {
            match group {
                GroupProof::Block { block } => {
                    let header = headers.get(cursor).ok_or("VO 包含额外区块")?;
                    records.extend(block.verify(header, query, None)?);
                    cursor += 1;
                }
                GroupProof::Window {
                    end_hash,
                    window,
                    blocks,
                } => {
                    if !query.aggregate {
                        return Err("普通查询收到聚合证明".into());
                    }
                    let size = window.descriptor.blocks.len();
                    let range = headers
                        .get(cursor..cursor + size)
                        .filter(|range| !range.is_empty())
                        .ok_or("窗口越界")?;
                    let last = range.last().unwrap();
                    if range[0].number != window.descriptor.start
                        || last.hash != *end_hash
                        || last.fields.roots.len() != 6
                    {
                        return Err("窗口边界未被可信头认证".into());
                    }
                    let bitmap = window.candidates(last.fields.roots[5], query)?;
                    for (summary, header) in window.descriptor.blocks.iter().zip(range) {
                        if header.fields.roots.len() != 6
                            || summary.roots != header.fields.roots[..5]
                            || summary.count as u64 != header.fields.record_count
                            || summary.min_time != header.fields.min_tx_time
                            || summary.max_time != header.fields.max_tx_time
                        {
                            return Err("窗口描述与原始区块不一致".into());
                        }
                    }
                    let candidates = bitmap.indices();
                    if candidates.len() != blocks.len() {
                        return Err("窗口候选块被遗漏".into());
                    }
                    for (offset, block) in candidates.into_iter().zip(blocks) {
                        let values = window.keyword_values(query, offset, &block.keywords)?;
                        records.extend(block.verify(&range[offset], query, Some(&values))?);
                    }
                    cursor += size;
                }
            }
        }
        if cursor != headers.len() {
            return Err("VO 遗漏查询区块".into());
        }
        Ok(records)
    }

    /// 将已证明的业务记录与 ABI 返回的真实签名交易哈希和位置绑定。
    pub fn verify_response(
        &self,
        headers: &[TrustedHeader],
        query: &LogQuery,
        matches: &[QueryMatch],
    ) -> Result<Vec<VerifiedRecord>, String> {
        let records = self.verify(headers, query)?;
        if records.len() != matches.len() {
            return Err("返回结果与 VO 数量不同".into());
        }
        for (verified, row) in records.iter().zip(matches) {
            if verified.block_hash != row.blockHash
                || verified.record.transaction_index != row.transactionIndex
                || verified.record.transaction_hash != row.transactionHash
                || keccak256(&row.rawTransaction) != row.transactionHash
            {
                return Err("返回交易与 VO 不一致".into());
            }
        }
        Ok(records)
    }
}
