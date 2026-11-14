//! BLAS 块内 OMT、IIMT、双时间范围树及其查询证明。
//! 定义了块内索引层的核心，BlockIndex索引，持久化。IndexProof证明
use crate::model::{metadata::IndexedRecord, query::LogQuery};
use crate::schemes::blas::common::{
    bitmap::Bitmap,
    filters::{FilterProof, Filters},
    merkle::{MerkleProof, MerkleTree},
};
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct BlockIndex {
    //IndexedBody::fields::BodyFields::BlockIndex
    pub records: Vec<IndexedRecord>, //块内全部日志文件元数据
    pub omt: MerkleTree, //对象Merkle树，叶子 = encode(IndexedRecord),用于证明某个逻辑位置的对象确实存在且内容未被篡改
    pub filters: Filters, //过滤索引，内部是四棵OrderedIndex：kw,min_time,max_time,lid
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexProof {
    pub filters: FilterProof,      //过滤层证明
    pub objects: Vec<MerkleProof>, //命中对象的OMT路径
}

impl BlockIndex {
    /// 从同序 records 构建完整索引；逻辑位图位置不等于实际 Ethereum 交易位置。
    pub fn build(records: Vec<IndexedRecord>) -> Result<Self, String> {
        if records.len() > 256 {
            return Err("BLAS 单块最多 256 份日志文件元数据".into());
        }
        let omt = MerkleTree::build(
            //叶子是整条IndexedRecord的RLP编码，即对象树承诺的是完整元数据
            records
                .iter()
                .map(|r| alloy_rlp::encode(r).into())
                .collect(),
            2,
        );
        let filters = Filters::build(
            //输入是三个同序数组
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

    /// 按 query 过滤；keywords 可提供聚合位图，with_objects 决定是否生成块内 OMT。
    pub fn prove(
        &self,
        query: &LogQuery,
        keywords: Option<&std::collections::BTreeMap<String, Bitmap>>,
        with_objects: bool,
    ) -> Result<(IndexProof, Bitmap), String> {
        let filters = self.filters.prove(query, keywords.is_none());
        let bitmap = filters.verify_with_keywords(
            &self.filters.roots(),
            self.records.len(),
            query,
            false,
            keywords,
        )?;
        let objects = if with_objects {
            bitmap
                .indices()
                .into_iter()
                .map(|i| self.omt.prove(i))
                .collect()
        } else {
            Vec::new()
        };
        Ok((IndexProof { filters, objects }, bitmap))
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
