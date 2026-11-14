//! 查询 VO 的独立验证器；输入仅为可信头字段、查询、结果和证明。
//!
use super::{
    HeaderFields, common::bitmap::Bitmap, common::merkle::MerkleProof, common::ordered::decode,
    index::generator::GeneratorProof, index::segment::WindowProof, index::transaction::IndexProof,
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
    Skip {
        count: usize,
    },
    Block {
        block: BlockWitness,
    },
    Window {
        end_hash: B256,
        window: WindowProof,
        blocks: Vec<BlockWitness>,
        generators: Vec<GeneratorProof>,
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
    /// 用 header 和可选 aggregated 位图认证过滤层，供两种对象证明共用。
    fn verify_filters(
        &self,
        header: &TrustedHeader,
        query: &LogQuery,
        aggregated: Option<&BTreeMap<String, Bitmap>>,
    ) -> Result<Bitmap, String> {
        if self.hash != header.hash || header.fields.roots.len() != 6 {
            return Err("VO 区块与可信头不一致".into());
        }
        let count = header.fields.record_count as usize;
        self.index.filters.verify_with_keywords(
            &header.fields.roots[1..5],
            count,
            query,
            false,
            aggregated,
        )
    }

    /// 对单块认证过滤位图及所有命中对象；aggregated 由窗口目录认证后提供。
    fn verify(
        &self,
        header: &TrustedHeader,
        query: &LogQuery,
        aggregated: Option<&BTreeMap<String, Bitmap>>,
    ) -> Result<Vec<VerifiedRecord>, String> {
        let bitmap = self.verify_filters(header, query, aggregated)?;
        let count = header.fields.record_count as usize;
        let mut records = Vec::new();
        for leaf in self.index.objects(header.fields.roots[0], count, &bitmap)? {
            let record: IndexedRecord = decode(&leaf)?;
            records.push(VerifiedRecord {
                block_hash: self.hash,
                record,
            });
        }
        Ok(records)
    }
}

impl QueryProof {
    /// 从窗口生成器承诺认证所有对象，并与过滤层的 expected 位置逐项对应。
    fn generator_records(
        window: &WindowProof,
        proofs: &[GeneratorProof],
        headers: &[TrustedHeader],
        query: &LogQuery,
        mut expected: std::collections::BTreeSet<(u64, u32)>,
    ) -> Result<Vec<VerifiedRecord>, String> {
        if proofs.len() != query.generators.len() {
            return Err("生成器证明数量或顺序非法".into());
        }
        let descriptor = &window.descriptor;
        let mut records = BTreeMap::new();
        for (lid, proof) in query.generators.iter().zip(proofs) {
            for (reference, record) in proof.verify(
                descriptor.generator_root,
                *lid,
                descriptor.start,
                &descriptor.blocks,
            )? {
                let position = (reference.height, reference.position);
                if !expected.remove(&position) || !query.matches(&record.metadata) {
                    return Err("聚合结果包含额外、重复或不匹配对象".into());
                }
                let offset = (reference.height - descriptor.start) as usize;
                records.insert(
                    position,
                    VerifiedRecord {
                        block_hash: headers[offset].hash,
                        record,
                    },
                );
            }
        }
        if !expected.is_empty() {
            return Err("聚合结果遗漏命中对象".into());
        }
        Ok(records.into_values().collect())
    }

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
        let mut cursor: usize = 0;
        let mut records = Vec::new();
        for group in &self.groups {
            match group {
                GroupProof::Skip { count } => {
                    let range = headers
                        .get(cursor..cursor.saturating_add(*count))
                        .filter(|range| !range.is_empty())
                        .ok_or("跳过范围越界")?;
                    if !query.aggregate
                        || !(range.last().unwrap().fields.past_max_time < query.time.0
                            || range.iter().all(|h| h.fields.outside(query.time)))
                    {
                        return Err("跳过范围可能包含命中对象".into());
                    }
                    cursor += count;
                }
                GroupProof::Block { block } => {
                    let header = headers.get(cursor).ok_or("VO 包含额外区块")?;
                    records.extend(block.verify(header, query, None)?);
                    cursor += 1;
                }
                GroupProof::Window {
                    end_hash,
                    window,
                    blocks,
                    generators,
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
                    let candidates = bitmap.indices();
                    if candidates.len() != blocks.len() {
                        return Err("窗口候选块被遗漏".into());
                    }
                    let mut expected = std::collections::BTreeSet::new();
                    for (offset, block) in candidates.into_iter().zip(blocks) {
                        let values = window.keyword_values(query, offset, &block.keywords)?;
                        if query.generators.is_empty() {
                            records.extend(block.verify(&range[offset], query, Some(&values))?);
                        } else {
                            if !block.index.objects.is_empty() {
                                return Err("生成器聚合不应附带块内对象路径".into());
                            }
                            let bitmap =
                                block.verify_filters(&range[offset], query, Some(&values))?;
                            expected.extend(
                                bitmap
                                    .indices()
                                    .into_iter()
                                    .map(|position| (range[offset].number, position as u32)),
                            );
                        }
                    }
                    records.extend(Self::generator_records(
                        window, generators, range, query, expected,
                    )?);
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
