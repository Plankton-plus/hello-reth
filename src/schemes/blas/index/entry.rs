//! 链下 LogFile：条目 OMT、关键词 IIMT、timestamp→sn 的四叉时间索引。
use crate::model::{
    entry::Entry,
    metadata::{LogMetadata, normalize_words},
    predicate::Predicate,
    query::LogQuery,
};
use crate::schemes::blas::common::{
    bitmap::Bitmap,
    filters::{evaluate, number, words},
    merkle::{MerkleProof, MerkleTree, hash},
    ordered::{Leaf, OrderedIndex, OrderedProof, decode},
};
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileInput {
    pub lid: u32,
    pub tx_public_keywords: Vec<String>,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogFile {
    pub metadata: LogMetadata,
    pub entries: Vec<Entry>,
    pub omt: MerkleTree,
    pub keywords: OrderedIndex,
    pub times: OrderedIndex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeProof {
    pub before: Option<OrderedProof>,
    pub end: OrderedProof,
    pub rows: Vec<MerkleProof>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileProof {
    pub metadata: LogMetadata,
    pub count: u32,
    pub time_root: B256,
    pub keywords: Vec<OrderedProof>,
    pub time: TimeProof,
    pub objects: Vec<MerkleProof>,
}

/// 把条目数量与时间树根连接，防止空结果时伪造 NOT 的全集长度。
fn time_commitment(count: usize, root: B256) -> B256 {
    hash(&alloy_rlp::encode(vec![
        alloy_rlp::encode(count as u64),
        alloy_rlp::encode(root),
    ]))
}

impl LogFile {
    /// 从 input 构建文件；sn 必须为文件内从零开始的连续位置，允许重复时间戳。
    pub fn build(mut input: FileInput) -> Result<Self, String> {
        let count = input.entries.len();
        if count == 0 || count > 2048 || input.lid == 0 || input.lid > 4096 {
            return Err("文件条目数或生成器非法".into());
        }
        normalize_words(&mut input.tx_public_keywords, 256)?;
        for (index, entry) in input.entries.iter_mut().enumerate() {
            if entry.sn as usize != index {
                return Err("文件内 sn 必须连续且从零开始".into());
            }
            normalize_words(&mut entry.keywords, 256)?;
        }
        let vocabulary: BTreeSet<_> = input
            .entries
            .iter()
            .flat_map(|entry| entry.keywords.iter().cloned())
            .collect();
        let values = vocabulary
            .iter()
            .map(|key| {
                let bitmap = Bitmap::positions(
                    count,
                    input
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.keywords.contains(key))
                        .map(|(i, _)| i),
                );
                (key.clone(), alloy_rlp::encode(bitmap).into())
            })
            .collect();
        let mut times = BTreeMap::<String, Vec<u32>>::new();
        for entry in &input.entries {
            times
                .entry(number(entry.timestamp))
                .or_default()
                .push(entry.sn);
        }
        let times = OrderedIndex::build(
            times
                .into_iter()
                .map(|(key, positions)| (key, alloy_rlp::encode(positions).into()))
                .collect(),
            4,
        );
        let keywords = OrderedIndex::build(values, 2);
        let omt = MerkleTree::build(
            input
                .entries
                .iter()
                .map(|entry| alloy_rlp::encode(entry).into())
                .collect(),
            2,
        );
        let metadata = LogMetadata {
            lid: input.lid,
            t_min: input.entries.iter().map(|e| e.timestamp).min().unwrap(),
            t_max: input.entries.iter().map(|e| e.timestamp).max().unwrap(),
            tx_public_keys: input.tx_public_keywords,
            entry_keywords: vocabulary.into_iter().collect(),
            omt_root: omt.root(),
            iimt_root: keywords.tree.root(),
            rimbt_root: time_commitment(count, times.tree.root()),
        };
        Ok(Self {
            metadata,
            entries: input.entries,
            omt,
            keywords,
            times,
        })
    }

    /// 验证磁盘文件没有改变内容或缓存索引，避免查询使用损坏的派生数据。
    pub fn validate(&self) -> Result<(), String> {
        let expected = Self::build(FileInput {
            lid: self.metadata.lid,
            tx_public_keywords: self.metadata.tx_public_keys.clone(),
            entries: self.entries.clone(),
        })?;
        if expected != *self {
            return Err("链下文件或 ADS 不一致".into());
        }
        Ok(())
    }

    /// 构造 query 对应的文件证明；文件谓词与条目谓词分别解释。
    pub fn prove(&self, query: &LogQuery) -> Result<FileProof, String> {
        self.validate()?;
        let predicate = query.entry_predicate.as_ref().unwrap_or(&Predicate::All);
        let before = query
            .time
            .0
            .checked_sub(1)
            .map(|value| self.times.prove(&number(value)));
        let end = self.times.prove(&number(query.time.1));
        let start_index = before
            .as_ref()
            .and_then(|p| p.left.as_ref())
            .map(|p| p.index + 1)
            .unwrap_or(0);
        let end_index = end.left.as_ref().map(|p| p.index + 1).unwrap_or(0);
        let mut proof = FileProof {
            metadata: self.metadata.clone(),
            count: self.entries.len() as u32,
            time_root: self.times.tree.root(),
            keywords: words(predicate)
                .iter()
                .map(|key| self.keywords.prove(key))
                .collect(),
            time: TimeProof {
                before,
                end,
                rows: (start_index..end_index)
                    .map(|i| self.times.tree.prove(i))
                    .collect(),
            },
            objects: vec![],
        };
        let bitmap = proof.bitmap(&self.metadata, query)?;
        proof.objects = bitmap
            .indices()
            .into_iter()
            .map(|i| self.omt.prove(i))
            .collect();
        Ok(proof)
    }
}

impl FileProof {
    /// 只由已认证 metadata、query 和本证明恢复精确候选位图。
    fn bitmap(&self, expected: &LogMetadata, query: &LogQuery) -> Result<Bitmap, String> {
        let count = self.count as usize;
        if self.metadata != *expected
            || count == 0
            || count > 2048
            || time_commitment(count, self.time_root) != expected.rimbt_root
        {
            return Err("文件根或条目数量不一致".into());
        }
        let predicate = query.entry_predicate.as_ref().unwrap_or(&Predicate::All);
        let keys: Vec<_> = words(predicate).into_iter().collect();
        if keys.len() != self.keywords.len() {
            return Err("文件关键词证明缺失".into());
        }
        let mut values = BTreeMap::new();
        for (key, proof) in keys.iter().zip(&self.keywords) {
            let bitmap = proof
                .exact(expected.iimt_root, key)?
                .map(|bytes| decode::<Bitmap>(&bytes))
                .transpose()?
                .unwrap_or_else(|| Bitmap::new(count, false));
            bitmap.validate(count)?;
            values.insert(key.clone(), bitmap);
        }
        let mut bitmap = evaluate(predicate, &values, count, false);
        let start = match (query.time.0.checked_sub(1), &self.time.before) {
            (Some(value), Some(proof)) => {
                proof.floor(self.time_root, &number(value))?;
                proof.left.as_ref().map(|p| p.index + 1).unwrap_or(0)
            }
            (None, None) => 0,
            _ => return Err("文件时间左边界错误".into()),
        };
        self.time.end.floor(self.time_root, &number(query.time.1))?;
        let end = self
            .time
            .end
            .left
            .as_ref()
            .map(|p| p.index + 1)
            .unwrap_or(0);
        if end < start || self.time.rows.len() != end - start {
            return Err("文件时间范围不完整".into());
        }
        let mut positions = BTreeSet::new();
        for (index, proof) in (start..end).zip(&self.time.rows) {
            if proof.index != index {
                return Err("文件时间叶不连续".into());
            }
            proof.verify(self.time_root)?;
            let leaf: Leaf = decode(&proof.leaf)?;
            if leaf.key < number(query.time.0) || leaf.key > number(query.time.1) {
                return Err("文件时间叶越界".into());
            }
            for sn in decode::<Vec<u32>>(&leaf.value)? {
                if sn as usize >= count || !positions.insert(sn as usize) {
                    return Err("文件时间位置非法或重复".into());
                }
            }
        }
        bitmap = bitmap.combine(&Bitmap::positions(count, positions), false);
        if !query.matches(expected) {
            bitmap = Bitmap::new(count, false);
        }
        Ok(bitmap)
    }

    /// 独立验证并返回条目，验证器不接收或读取完整 LogFile。
    pub fn verify(&self, expected: &LogMetadata, query: &LogQuery) -> Result<Vec<Entry>, String> {
        let indices = self.bitmap(expected, query)?.indices();
        if indices.len() != self.objects.len() {
            return Err("文件结果遗漏或多报".into());
        }
        self.objects
            .iter()
            .zip(indices)
            .map(|(proof, index)| {
                if proof.index != index || proof.count != self.count as usize {
                    return Err("条目序号错误".into());
                }
                proof.verify(expected.omt_root)?;
                let entry: Entry = decode(&proof.leaf)?;
                if entry.sn as usize != index
                    || entry.timestamp < query.time.0
                    || entry.timestamp > query.time.1
                    || !query
                        .entry_predicate
                        .as_ref()
                        .unwrap_or(&Predicate::All)
                        .matches(&entry.keywords)
                {
                    return Err("条目不满足查询".into());
                }
                Ok(entry)
            })
            .collect()
    }
}
