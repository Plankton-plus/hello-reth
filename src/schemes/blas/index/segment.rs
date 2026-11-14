//! 分段窗口索引与 ALIS：共享关键词目录，下层树认证各块的交易位图。
//! 实现段级索引，对应论文里的ALIS/段级索引。跨块查询时，不必每个块重复认证，而是用一个共享关键词目录+每关键词一颗下层位图
//! 来统一认证。
use super::{generator::GeneratorIndex, transaction::BlockIndex};
use crate::model::query::LogQuery;
use crate::schemes::blas::common::{
    bitmap::Bitmap,
    filters::{FilterProof, Filters, words},
    merkle::{MerkleProof, MerkleTree, hash},
    ordered::{OrderedIndex, OrderedProof, decode},
};
use alloy_primitives::B256;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 小样本协议采用四块窗口；论文的 1024 块配置需要另外扩大框架认证范围。
pub const WINDOW_BLOCKS: u64 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct BlockSummary {
    pub count: u32,
    pub min_time: u64,
    pub max_time: u64,
    pub roots: Vec<B256>,
}

impl BlockSummary {
    /// 摘取 index 的认证根与时间，当前块不依赖其尚未产生的最终块哈希。
    pub fn new(index: &BlockIndex) -> Self {
        Self {
            count: index.records.len() as u32,
            min_time: index
                .records
                .iter()
                .map(|r| r.metadata.t_min)
                .min()
                .unwrap_or(0),
            max_time: index
                .records
                .iter()
                .map(|r| r.metadata.t_max)
                .max()
                .unwrap_or(0),
            roots: index.roots(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct WindowDescriptor {
    //窗口的认证描述符
    pub start: u64,
    pub blocks: Vec<BlockSummary>,
    pub filter_roots: Vec<B256>, //过滤索引的根
    pub down_root: B256,         //共享关键词目录的根
    pub generator_root: B256,    //生成聚合目录的根
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct WindowIndex {
    pub start: u64,
    pub blocks: Vec<BlockSummary>,
    pub filters: Filters,
    pub directory: OrderedIndex, //关键词，下层树根
    pub down: Vec<MerkleTree>,   //每个关键词一颗各块位图树
    pub generators: GeneratorIndex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowProof {
    pub descriptor: WindowDescriptor,
    pub filters: FilterProof,
    pub directory: Vec<OrderedProof>,
}

impl WindowIndex {
    /// 校验目录、下层树和共享根的连接；父链语义由接收时重建验证。
    pub fn validate(&self) -> Result<(), String> {
        self.filters.validate()?;
        self.generators.validate()?;
        self.directory.validate()?;
        if self.directory.leaves.len() != self.down.len() {
            return Err("聚合目录长度不一致".into());
        }
        for (leaf, tree) in self.directory.leaves.iter().zip(&self.down) {
            tree.validate()?;
            if leaf.value.as_ref() != tree.root().as_slice()
                || tree.leaves.len() != self.blocks.len()
            {
                return Err("聚合目录与下层树承诺不一致".into());
            }
        }
        Ok(())
    }
    /// 对连续 indexes 构建块位图及每关键词的下层交易位图树。
    pub fn build(start: u64, indexes: &[BlockIndex]) -> Result<Self, String> {
        if start == 0 || indexes.len() != WINDOW_BLOCKS as usize {
            return Err("窗口起点或块数非法".into());
        }
        let blocks: Vec<_> = indexes.iter().map(BlockSummary::new).collect();
        let keys: Vec<Vec<_>> = indexes
            .iter()
            .map(|index| {
                index
                    .filters
                    .keywords
                    .leaves
                    .iter()
                    .map(|leaf| leaf.key.clone())
                    .collect()
            })
            .collect();
        let lids: Vec<_> = indexes
            .iter()
            .map(|index| index.records.iter().map(|r| r.metadata.lid).collect())
            .collect();
        let filters = Filters::build(
            &keys,
            &blocks
                .iter()
                .map(|b| (b.min_time, b.max_time))
                .collect::<Vec<_>>(),
            &lids,
        );
        let mut directory = BTreeMap::new();
        let mut down = Vec::new();
        for key in filters.keywords.leaves.iter().map(|leaf| &leaf.key) {
            let leaves = indexes
                .iter()
                .map(|index| {
                    let bitmap = index
                        .filters
                        .keywords
                        .leaves
                        .binary_search_by(|leaf| leaf.key.cmp(key))
                        .ok()
                        .map(|i| index.filters.keywords.leaves[i].value.clone())
                        .unwrap_or_else(|| {
                            alloy_rlp::encode(Bitmap::new(index.records.len(), false)).into()
                        });
                    bitmap
                })
                .collect();
            let tree = MerkleTree::build(leaves, 2);
            directory.insert(key.clone(), tree.root().as_slice().to_vec().into());
            down.push(tree);
        }
        Ok(Self {
            start,
            blocks,
            filters,
            generators: GeneratorIndex::build(start, indexes),
            directory: OrderedIndex::build(directory, 2),
            down,
        })
    }

    /// 以小型描述符认证全部窗口根，证明无需传输整个窗口索引。
    pub fn descriptor(&self) -> WindowDescriptor {
        WindowDescriptor {
            start: self.start,
            blocks: self.blocks.clone(),
            filter_roots: self.filters.roots(),
            down_root: self.directory.tree.root(),
            generator_root: self.generators.directory.tree.root(),
        }
    }

    /// 当前结束块头保存的窗口承诺。
    pub fn root(&self) -> B256 {
        hash(&alloy_rlp::encode(self.descriptor()))
    }

    /// query 的共享目录和块级剪枝证明，每个关键词只出现一次。
    pub fn prove(&self, query: &LogQuery) -> WindowProof {
        WindowProof {
            descriptor: self.descriptor(),
            filters: self.filters.prove(query, true),
            directory: words(&query.predicate)
                .iter()
                .map(|key| self.directory.prove(key))
                .collect(),
        }
    }

    /// 给 offset 块提供已查询关键词的下层路径；全窗口缺失的词无需路径。
    pub fn keyword_paths(&self, query: &LogQuery, offset: usize) -> Vec<Option<MerkleProof>> {
        words(&query.predicate)
            .iter()
            .map(|key| {
                self.directory
                    .leaves
                    .binary_search_by(|leaf| leaf.key.cmp(key))
                    .ok()
                    .map(|i| self.down[i].prove(offset))
            })
            .collect()
    }
}

impl WindowProof {
    /// 认证窗口描述符并重算候选块；NOT 使用保守块级剪枝。
    pub fn candidates(&self, root: B256, query: &LogQuery) -> Result<Bitmap, String> {
        let descriptor = &self.descriptor;
        if descriptor.blocks.len() != WINDOW_BLOCKS as usize
            || descriptor.start == 0
            || descriptor
                .blocks
                .iter()
                .any(|b| b.count > 256 || b.roots.len() != 5)
            || hash(&alloy_rlp::encode(descriptor)) != root
        {
            return Err("窗口认证信息不一致".into());
        }
        self.filters.verify(
            &descriptor.filter_roots,
            descriptor.blocks.len(),
            query,
            true,
        )
    }

    /// 独立认证 offset 块的聚合交易位图，目录缺失也必须有邻接证明。
    pub fn keyword_values(
        &self,
        query: &LogQuery,
        offset: usize,
        paths: &[Option<MerkleProof>],
    ) -> Result<BTreeMap<String, Bitmap>, String> {
        let keys: Vec<_> = words(&query.predicate).into_iter().collect();
        if offset >= self.descriptor.blocks.len()
            || keys.len() != paths.len()
            || keys.len() != self.directory.len()
        {
            return Err("窗口关键词证明数量非法".into());
        }
        let count = self.descriptor.blocks[offset].count as usize;
        let mut values = BTreeMap::new();
        for ((key, directory), path) in keys.iter().zip(&self.directory).zip(paths) {
            let root = directory.exact(self.descriptor.down_root, key)?;
            let bitmap = match (root, path) {
                (None, None) => Bitmap::new(count, false),
                (Some(root), Some(path))
                    if root.len() == 32
                        && path.index == offset
                        && path.count == self.descriptor.blocks.len() =>
                {
                    path.verify(B256::from_slice(&root))?;
                    decode::<Bitmap>(&path.leaf)?
                }
                _ => return Err("下层交易位图证明错误".into()),
            };
            bitmap.validate(count)?;
            values.insert(key.clone(), bitmap);
        }
        Ok(values)
    }
}
