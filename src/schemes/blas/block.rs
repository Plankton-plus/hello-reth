//! BLAS 块内 OMT、IIMT、双时间范围树及其查询证明。
use super::{
    bitmap::Bitmap,
    filters::{FilterProof, Filters},
    merkle::{MerkleProof, MerkleTree},
};
use crate::model::{metadata::IndexedRecord, query::LogQuery};
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct BlockIndex {
    pub records: Vec<IndexedRecord>,
    pub omt: MerkleTree,
    pub filters: Filters,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexProof {
    pub filters: FilterProof,
    pub objects: Vec<MerkleProof>,
}

impl BlockIndex {
    /// 从同序 records 构建完整索引；逻辑位图位置不等于实际 Ethereum 交易位置。
    pub fn build(records: Vec<IndexedRecord>) -> Result<Self, String> {
        if records.len() > 256 {
            return Err("BLAS 单块最多 256 份日志文件元数据".into());
        }
        let omt = MerkleTree::build(
            records
                .iter()
                .map(|r| alloy_rlp::encode(r).into())
                .collect(),
            2,
        );
        let filters = Filters::build(
            &records
                .iter()
                .map(|r| r.metadata.tx_public_keys.clone())
                .collect::<Vec<_>>(),
            &records
                .iter()
                .map(|r| (r.metadata.t_min, r.metadata.t_max))
                .collect::<Vec<_>>(),
            &records
                .iter()
                .map(|r| vec![r.metadata.lid])
                .collect::<Vec<_>>(),
        );
        Ok(Self {
            records,
            omt,
            filters,
        })
    }

    /// 固定根顺序：OMT、关键词、开始时间、结束时间、生成器。
    pub fn roots(&self) -> Vec<B256> {
        let mut roots = vec![self.omt.root()];
        roots.extend(self.filters.roots());
        roots
    }

    /// 为 query 生成原生关键词证明及命中对象的 OMT 路径。
    pub fn prove(&self, query: &LogQuery) -> Result<IndexProof, String> {
        let filters = self.filters.prove(query);
        let bitmap = filters.verify(&self.filters.roots(), self.records.len(), query, false)?;
        Ok(IndexProof {
            filters,
            objects: bitmap
                .indices()
                .into_iter()
                .map(|i| self.omt.prove(i))
                .collect(),
        })
    }

    /// 按外部聚合位图 bitmap 生成对象路径，省略块内关键词证明。
    pub fn prove_aggregated(
        &self,
        query: &LogQuery,
        values: &std::collections::BTreeMap<String, Bitmap>,
    ) -> Result<IndexProof, String> {
        let mut filters = self.filters.prove(query);
        filters.keywords.clear();
        let bitmap = filters.verify_with_keywords(
            &self.filters.roots(),
            self.records.len(),
            query,
            false,
            Some(values),
        )?;
        Ok(IndexProof {
            filters,
            objects: bitmap
                .indices()
                .into_iter()
                .map(|i| self.omt.prove(i))
                .collect(),
        })
    }
}

impl IndexProof {
    /// 由 bitmap 逐项验证 OMT，既拒绝伪造对象，也拒绝遗漏或额外结果。
    pub fn objects(&self, root: B256, count: usize, bitmap: &Bitmap) -> Result<Vec<Bytes>, String> {
        let positions = bitmap.indices();
        if positions.len() != self.objects.len() {
            return Err("结果集合不完整或包含额外对象".into());
        }
        self.objects
            .iter()
            .zip(positions)
            .map(|(proof, index)| {
                if proof.index != index || proof.count != count {
                    return Err("对象位置或数量不一致".into());
                }
                proof.verify(root)?;
                Ok(proof.leaf.clone())
            })
            .collect()
    }
}
