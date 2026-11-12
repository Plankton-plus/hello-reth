//! 静态有序 Merkle 索引；邻接叶证明同时认证命中、缺失和范围边界。
use super::merkle::{MerkleProof, MerkleTree};
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{Decodable, RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct Leaf {
    pub key: String,
    pub value: Bytes,
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct OrderedIndex {
    pub leaves: Vec<Leaf>,
    pub tree: MerkleTree,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderedProof {
    pub key: String,
    pub fanout: u32,
    pub left: Option<MerkleProof>,
    pub right: Option<MerkleProof>,
}

/// 从 bytes 解码一个规范 RLP 值，拒绝尾随字节。
pub fn decode<T: Decodable>(bytes: &[u8]) -> Result<T, String> {
    let mut remaining = bytes;
    let value = T::decode(&mut remaining).map_err(|error| error.to_string())?;
    if !remaining.is_empty() {
        return Err("索引 RLP 有尾随字节".into());
    }
    Ok(value)
}

impl OrderedIndex {
    /// 检查键序和叶子编码及全部哈希缓存，读取完整索引时调用。
    pub fn validate(&self) -> Result<(), String> {
        if self
            .leaves
            .windows(2)
            .any(|pair| pair[0].key >= pair[1].key)
            || self.tree.leaves
                != self
                    .leaves
                    .iter()
                    .map(|leaf| alloy_rlp::encode(leaf).into())
                    .collect::<Vec<Bytes>>()
        {
            return Err("有序索引叶子或键序不一致".into());
        }
        self.tree.validate()
    }
    /// 按 key 排序打包 leaves；fanout 区分二叉 IIMT 与四叉范围树。
    pub fn build(values: BTreeMap<String, Bytes>, fanout: u32) -> Self {
        let leaves: Vec<_> = values
            .into_iter()
            .map(|(key, value)| Leaf { key, value })
            .collect();
        let tree = MerkleTree::build(
            leaves
                .iter()
                .map(|leaf| alloy_rlp::encode(leaf).into())
                .collect(),
            fanout,
        );
        Self { leaves, tree }
    }

    /// 为 key 提供其前驱与后继；首尾及空树均有明确证明。
    pub fn prove(&self, key: &str) -> OrderedProof {
        let upper = self.upper_bound(key);
        OrderedProof {
            key: key.into(),
            fanout: self.tree.fanout,
            left: upper.checked_sub(1).map(|index| self.tree.prove(index)),
            right: (upper < self.leaves.len()).then(|| self.tree.prove(upper)),
        }
    }
    /// 沿打包树的子树最小键路由；分隔键由有序叶子确定，不另存易失配副本。
    fn upper_bound(&self, key: &str) -> usize {
        if self
            .leaves
            .first()
            .is_none_or(|leaf| key < leaf.key.as_str())
        {
            return 0;
        }
        let fanout = self.tree.fanout as usize;
        let mut span = fanout.pow(self.tree.levels.len().saturating_sub(1) as u32);
        let mut offset = 0;
        while span > 1 {
            let child_span = span / fanout;
            let end = (offset + span).min(self.leaves.len());
            offset = (offset..end)
                .step_by(child_span)
                .take_while(|index| self.leaves[*index].key.as_str() <= key)
                .last()
                .unwrap_or(offset);
            span = child_span;
        }
        offset + 1
    }
}

impl OrderedProof {
    /// 对可信 root 认证 key 的前驱；相邻序号保证服务端不能跳过中间叶。
    pub fn floor(&self, root: B256, key: &str) -> Result<Option<Leaf>, String> {
        if self.key != key || ![2, 4].contains(&self.fanout) {
            return Err("索引证明请求不一致".into());
        }
        for path in [&self.left, &self.right].into_iter().flatten() {
            if path.fanout != self.fanout {
                return Err("索引扇出不一致".into());
            }
            path.verify(root)?;
        }
        let left = self
            .left
            .as_ref()
            .map(|p| decode::<Leaf>(&p.leaf))
            .transpose()?;
        let right = self
            .right
            .as_ref()
            .map(|p| decode::<Leaf>(&p.leaf))
            .transpose()?;
        if left.as_ref().is_some_and(|leaf| leaf.key.as_str() > key)
            || right.as_ref().is_some_and(|leaf| leaf.key.as_str() <= key)
        {
            return Err("索引边界与查询不符".into());
        }
        match (&self.left, &self.right) {
            (Some(a), Some(b)) if a.count == b.count && a.index + 1 == b.index => (),
            (None, Some(b)) if b.index == 0 => (),
            (Some(a), None) if a.index + 1 == a.count => (),
            (None, None) if MerkleTree::build(vec![], self.fanout).root() == root => (),
            _ => return Err("索引边界不相邻或空树承诺错误".into()),
        }
        Ok(left)
    }

    /// 认证精确 key；缺失时返回 None，而非服务端未经证明的零位图。
    pub fn exact(&self, root: B256, key: &str) -> Result<Option<Bytes>, String> {
        Ok(self
            .floor(root, key)?
            .filter(|leaf| leaf.key == key)
            .map(|leaf| leaf.value))
    }
}
