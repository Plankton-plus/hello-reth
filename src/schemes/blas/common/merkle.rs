//! BLAKE2s Merkle 树及独立路径验证；根认证扇出和叶子数量。通用Merkle树基层
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use blake2::{Blake2s256, Digest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 返回 data 的 BLAKE2s-256 摘要，与上游实际算法一致。
pub fn hash(data: &[u8]) -> B256 {
    B256::from_slice(&Blake2s256::digest(data))
}

/// 对域 tag 和有序 parts 做带边界编码后哈希。
fn tagged(tag: u8, parts: Vec<Bytes>) -> B256 {
    let mut input = vec![Bytes::from(vec![tag])];
    input.extend(parts);
    hash(&alloy_rlp::encode(input))
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct MerkleTree {
    pub fanout: u32,
    pub leaves: Vec<Bytes>,
    pub levels: Vec<Vec<B256>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleProof {
    pub fanout: u32,
    pub count: usize,
    pub index: usize,
    pub leaf: Bytes,
    pub siblings: Vec<Vec<B256>>,
}

/// 将摘要与树形参数绑定；空树也有唯一承诺。
fn commitment(fanout: u32, count: usize, root: B256) -> B256 {
    tagged(
        2,
        vec![
            alloy_rlp::encode(fanout).into(),
            alloy_rlp::encode(count as u64).into(),
            root.as_slice().to_vec().into(),
        ],
    )
}

impl MerkleTree {
    /// 检查缓存层级与叶子一致，避免磁盘损坏仅修改叶子而沿用旧根。
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=256).contains(&self.fanout)
            || Self::build(self.leaves.clone(), self.fanout) != *self
        {
            return Err("Merkle 树缓存与叶子不一致".into());
        }
        Ok(())
    }
    /// 用 leaves 的既定顺序构建 fanout 叉树。
    pub fn build(leaves: Vec<Bytes>, fanout: u32) -> Self {
        assert!((2..=256).contains(&fanout));
        let mut levels = vec![
            leaves
                .iter()
                .map(|leaf| tagged(0, vec![leaf.clone()]))
                .collect::<Vec<_>>(),
        ];
        while levels.last().unwrap().len() > 1 {
            let next = levels
                .last()
                .unwrap()
                .chunks(fanout as usize)
                .map(|group| {
                    tagged(
                        1,
                        group.iter().map(|h| h.as_slice().to_vec().into()).collect(),
                    )
                })
                .collect();
            levels.push(next);
        }
        Self {
            fanout,
            leaves,
            levels,
        }
    }
    /// 读取预计算根及树形承诺。
    pub fn root(&self) -> B256 {
        commitment(
            self.fanout,
            self.leaves.len(),
            self.levels
                .last()
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(B256::ZERO),
        )
    }
    /// 从已保存的树层级生成 index 的路径。
    pub fn prove(&self, mut index: usize) -> MerkleProof {
        let mut proof = MerkleProof {
            fanout: self.fanout,
            count: self.leaves.len(),
            index,
            leaf: self.leaves[index].clone(),
            siblings: Vec::new(),
        };
        for level in self.levels.iter().take(self.levels.len() - 1) {
            let start = index / self.fanout as usize * self.fanout as usize;
            proof.siblings.push(
                (start..(start + self.fanout as usize).min(level.len()))
                    .filter(|i| *i != index)
                    .map(|i| level[i])
                    .collect(),
            );
            index /= self.fanout as usize;
        }
        proof
    }
}

impl MerkleProof {
    /// 仅凭路径和外部可信根 expected 验证，不读取服务端树。
    pub fn verify(&self, expected: B256) -> Result<(), String> {
        MerkleMultiProof {
            fanout: self.fanout,
            count: self.count,
            indices: vec![self.index],
            leaves: vec![self.leaf.clone()],
            siblings: self.siblings.iter().flatten().copied().collect(),
        }
        .verify(expected)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MerkleMultiProof {
    pub fanout: u32,
    pub count: usize,
    pub indices: Vec<usize>,
    pub leaves: Vec<Bytes>,
    pub siblings: Vec<B256>,
}

impl MerkleTree {
    /// 为严格递增的 indices 生成共享证明，每个缺失兄弟节点仅写入一次。
    pub fn multi_prove(&self, indices: &[usize]) -> Result<MerkleMultiProof, String> {
        if indices.is_empty()
            || indices.windows(2).any(|pair| pair[0] >= pair[1])
            || *indices.last().unwrap() >= self.leaves.len()
        {
            return Err("共享证明位置非法".into());
        }
        let mut proof = MerkleMultiProof {
            fanout: self.fanout,
            count: self.leaves.len(),
            indices: indices.to_vec(),
            leaves: indices.iter().map(|i| self.leaves[*i].clone()).collect(),
            siblings: Vec::new(),
        };
        let mut active: BTreeSet<_> = indices.iter().copied().collect();
        for level in self.levels.iter().take(self.levels.len() - 1) {
            let parents: BTreeSet<_> = active.iter().map(|i| i / self.fanout as usize).collect();
            for parent in &parents {
                let start = parent * self.fanout as usize;
                for i in start..(start + self.fanout as usize).min(level.len()) {
                    if !active.contains(&i) {
                        proof.siblings.push(level[i]);
                    }
                }
            }
            active = parents;
        }
        Ok(proof)
    }
}

impl MerkleMultiProof {
    /// 验证严格递增的叶位置、全部共享节点及外部承诺 expected。
    pub fn verify(&self, expected: B256) -> Result<(), String> {
        if !(2..=256).contains(&self.fanout)
            || self.count > 1_000_000
            || self.indices.is_empty()
            || self.indices.len() != self.leaves.len()
            || self.indices.windows(2).any(|pair| pair[0] >= pair[1])
            || *self.indices.last().unwrap() >= self.count
        {
            return Err("共享证明参数非法".into());
        }
        let mut active: BTreeMap<_, _> = self
            .indices
            .iter()
            .copied()
            .zip(self.leaves.iter().map(|leaf| tagged(0, vec![leaf.clone()])))
            .collect();
        let mut count = self.count;
        let mut siblings = self.siblings.iter();
        while count > 1 {
            let parents: BTreeSet<_> = active.keys().map(|i| i / self.fanout as usize).collect();
            let mut next = BTreeMap::new();
            for parent in parents {
                let start = parent * self.fanout as usize;
                let mut children = Vec::new();
                for i in start..(start + self.fanout as usize).min(count) {
                    let child = match active.get(&i) {
                        Some(hash) => hash,
                        None => siblings.next().ok_or("共享证明兄弟节点缺失")?,
                    };
                    children.push(child.as_slice().to_vec().into());
                }
                next.insert(parent, tagged(1, children));
            }
            active = next;
            count = count.div_ceil(self.fanout as usize);
        }
        let root = *active.get(&0).ok_or("共享证明根缺失")?;
        if siblings.next().is_some() || commitment(self.fanout, self.count, root) != expected {
            return Err("共享证明认证根不匹配".into());
        }
        Ok(())
    }
}
