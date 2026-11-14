//! 同一生成器的跨块对象树；目录认证对象根和数量，块位图复用段过滤索引。
use super::{segment::BlockSummary, transaction::BlockIndex};
use crate::model::metadata::IndexedRecord;
use crate::schemes::blas::common::{
    filters::number,
    merkle::{MerkleMultiProof, MerkleTree, hash},
    ordered::{OrderedIndex, OrderedProof, decode},
};
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct ObjectReference {
    pub height: u64,
    pub position: u32,
    pub digest: B256,
}

impl ObjectReference {
    /// height 是块高，position 是紧凑记录位置；实际交易序号由 record 摘要绑定。
    pub fn new(height: u64, position: usize, record: &IndexedRecord) -> Self {
        Self {
            height,
            position: position as u32,
            digest: hash(&alloy_rlp::encode(record)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct GeneratorSummary {
    pub root: B256,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct GeneratorIndex {
    pub directory: OrderedIndex,
    pub trees: Vec<MerkleTree>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratorProof {
    pub directory: OrderedProof,
    pub objects: Option<MerkleMultiProof>,
    pub records: Vec<IndexedRecord>,
}

impl GeneratorIndex {
    /// 将 start 起的连续 indexes 按生成器分组，叶顺序固定为块号、记录位置。
    pub fn build(start: u64, indexes: &[BlockIndex]) -> Self {
        let mut groups: BTreeMap<u32, Vec<Bytes>> = BTreeMap::new();
        for (offset, index) in indexes.iter().enumerate() {
            for (position, record) in index.records.iter().enumerate() {
                let leaf = ObjectReference::new(start + offset as u64, position, record);
                groups
                    .entry(record.metadata.lid)
                    .or_default()
                    .push(alloy_rlp::encode(leaf).into());
            }
        }
        let mut directory = BTreeMap::new();
        let mut trees = Vec::new();
        for (lid, leaves) in groups {
            let tree = MerkleTree::build(leaves, 2);
            let summary = GeneratorSummary {
                root: tree.root(),
                count: tree.leaves.len() as u32,
            };
            directory.insert(number(lid as u64), alloy_rlp::encode(summary).into());
            trees.push(tree);
        }
        Self {
            directory: OrderedIndex::build(directory, 2),
            trees,
        }
    }

    /// 检查目录与树根、对象数量；父链重建负责校验对象位置和内容。
    pub fn validate(&self) -> Result<(), String> {
        self.directory.validate()?;
        if self.directory.leaves.len() != self.trees.len() {
            return Err("生成器目录长度不一致".into());
        }
        for (leaf, tree) in self.directory.leaves.iter().zip(&self.trees) {
            tree.validate()?;
            let summary: GeneratorSummary = decode(&leaf.value)?;
            if summary.root != tree.root()
                || summary.count as usize != tree.leaves.len()
                || summary.count == 0
            {
                return Err("生成器目录与对象树不一致".into());
            }
        }
        Ok(())
    }
    /// 以 lid 选择对象树，从 selected 的块高、记录位置生成共享证明。
    pub fn prove(
        &self,
        lid: u32,
        selected: &BTreeMap<(u64, u32), IndexedRecord>,
    ) -> Result<GeneratorProof, String> {
        let key = number(lid as u64);
        let mut proof = GeneratorProof {
            directory: self.directory.prove(&key),
            objects: None,
            records: Vec::new(),
        };
        let index = self
            .directory
            .leaves
            .binary_search_by(|leaf| leaf.key.cmp(&key))
            .ok();
        if let Some(index) = index {
            let tree = &self.trees[index];
            let mut indices = Vec::new();
            for (i, bytes) in tree.leaves.iter().enumerate() {
                let reference: ObjectReference = decode(bytes)?;
                if let Some(record) = selected.get(&(reference.height, reference.position)) {
                    indices.push(i);
                    proof.records.push(record.clone());
                }
            }
            if !indices.is_empty() {
                proof.objects = Some(tree.multi_prove(&indices)?);
            }
        }
        Ok(proof)
    }
}

impl GeneratorProof {
    /// 验证外部 root 到共享对象路径；对象集合完整性由调用方的过滤位图检查。
    pub fn verify(
        &self,
        root: B256,
        lid: u32,
        start: u64,
        blocks: &[BlockSummary],
    ) -> Result<Vec<(ObjectReference, IndexedRecord)>, String> {
        let value = self.directory.exact(root, &number(lid as u64))?;
        let Some(objects) = &self.objects else {
            if !self.records.is_empty() {
                return Err("生成器对象路径缺失".into());
            }
            return Ok(Vec::new());
        };
        let summary: GeneratorSummary = decode(value.as_deref().ok_or("生成器不存在")?)?;
        if objects.fanout != 2
            || objects.count != summary.count as usize
            || objects.leaves.len() != self.records.len()
        {
            return Err("生成器共享证明数量不一致".into());
        }
        objects.verify(summary.root)?;
        self.verify_records(objects, lid, start, blocks)
    }
    /// 将 objects 中每个摘要与 records 对应，检查 lid 与窗口位置。
    fn verify_records(
        &self,
        objects: &MerkleMultiProof,
        lid: u32,
        start: u64,
        blocks: &[BlockSummary],
    ) -> Result<Vec<(ObjectReference, IndexedRecord)>, String> {
        let mut result = Vec::new();
        for (bytes, record) in objects.leaves.iter().zip(&self.records) {
            let reference: ObjectReference = decode(bytes)?;
            let offset = reference.height.checked_sub(start).ok_or("对象早于窗口")? as usize;
            let count = blocks.get(offset).ok_or("对象晚于窗口")?.count;
            if reference.position >= count
                || record.metadata.lid != lid
                || reference
                    != ObjectReference::new(reference.height, reference.position as usize, record)
            {
                return Err("聚合对象生成器、位置或摘要不一致".into());
            }
            result.push((reference, record.clone()));
        }
        Ok(result)
    }
}
