//! BLAKE2s Merkle 树及独立路径验证；根认证扇出和叶子数量。
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use blake2::{Blake2s256, Digest};
use serde::{Deserialize, Serialize};

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
        if !(2..=256).contains(&self.fanout) || self.index >= self.count || self.count > 1_000_000 {
            return Err("Merkle 证明参数非法".into());
        }
        let (mut index, mut count) = (self.index, self.count);
        let mut node = tagged(0, vec![self.leaf.clone()]);
        let mut paths = self.siblings.iter();
        while count > 1 {
            let siblings = paths.next().ok_or("Merkle 路径缺失")?;
            let start = index / self.fanout as usize * self.fanout as usize;
            let width = (count - start).min(self.fanout as usize);
            if siblings.len() + 1 != width {
                return Err("Merkle 路径宽度非法".into());
            }
            let mut children = siblings.clone();
            children.insert(index - start, node);
            node = tagged(
                1,
                children
                    .iter()
                    .map(|h| h.as_slice().to_vec().into())
                    .collect(),
            );
            count = count.div_ceil(self.fanout as usize);
            index /= self.fanout as usize;
        }
        if paths.next().is_some() || commitment(self.fanout, self.count, node) != expected {
            return Err("Merkle 认证根不匹配".into());
        }
        Ok(())
    }
}
