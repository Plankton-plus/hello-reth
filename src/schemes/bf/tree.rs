//! 元数据 BF 树：关键词预筛，叶节点按公共布尔语义精确判断。

use crate::model::{metadata::IndexedRecord, predicate::Predicate, query::LogQuery};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Entry {
    Leaf(usize),
    Branch(usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Node {
    bloom: Bloom,
    time: (u64, u64),
    entry: Entry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BfTree {
    records: Vec<IndexedRecord>,
    nodes: Vec<Node>,
    root: Option<usize>,
}

impl BfTree {
    /// 按 records 的真实交易顺序构树；奇数层末尾节点直接提升。
    pub fn build(records: Vec<IndexedRecord>) -> Self {
        let mut nodes: Vec<Node> = records
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let mut bloom = Bloom::default();
                for word in &record.metadata.fw {
                    bloom.insert(word);
                }
                Node {
                    bloom,
                    time: (record.metadata.ts_min, record.metadata.ts_max),
                    entry: Entry::Leaf(index),
                }
            })
            .collect();
        let mut level: Vec<usize> = (0..nodes.len()).collect();
        while level.len() > 1 {
            let mut parents = Vec::new();
            for pair in level.chunks(2) {
                let left = pair[0];
                let Some(&right) = pair.get(1) else {
                    parents.push(left);
                    continue;
                };
                let mut bloom = nodes[left].bloom.clone();
                for (byte, other) in bloom.0.iter_mut().zip(nodes[right].bloom.0) {
                    *byte |= other;
                }
                let parent = Node {
                    bloom,
                    time: (
                        nodes[left].time.0.min(nodes[right].time.0),
                        nodes[left].time.1.max(nodes[right].time.1),
                    ),
                    entry: Entry::Branch(left, right),
                };
                parents.push(nodes.len());
                nodes.push(parent);
            }
            level = parents;
        }
        Self {
            records,
            nodes,
            root: level.first().copied(),
        }
    }

    /// 编码完整树；结构体字段和节点顺序固定，所有节点均随块保存。
    pub fn encoded(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("BF 树字段均可序列化")
    }

    /// 解码已经通过区块承诺校验的 bytes，不补建缺失索引。
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("BF 树解码失败：{e}"))
    }

    /// 返回满足 query 且位于 candidates 中的交易位置。
    pub fn query(&self, query: &LogQuery, candidates: &[u32]) -> Vec<u32> {
        let mut pending: Vec<usize> = self.root.into_iter().collect();
        let mut found = Vec::new();
        while let Some(index) = pending.pop() {
            let node = &self.nodes[index];
            if node.time.1 < query.time.0
                || node.time.0 > query.time.1
                || !node.bloom.may_match(&query.predicate)
            {
                continue;
            }
            match node.entry {
                Entry::Leaf(index) => {
                    let record = &self.records[index];
                    if candidates.contains(&record.transaction_index)
                        && query.matches(&record.metadata)
                    {
                        found.push(record.transaction_index);
                    }
                }
                Entry::Branch(left, right) => {
                    pending.push(right);
                    pending.push(left);
                }
            }
        }
        found
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Bloom([u8; 32]);

impl Bloom {
    /// 插入 keyword；类型前缀将关键词与其他令牌域分开。
    fn insert(&mut self, keyword: &str) {
        let hash = alloy_primitives::keccak256([b"K:", keyword.as_bytes()].concat());
        for &position in &hash[..3] {
            self.0[usize::from(position) / 8] |= 1 << (position % 8);
        }
    }

    /// 判断 keyword 是否可能出现；true 仍需要精确检查。
    fn contains(&self, keyword: &str) -> bool {
        let hash = alloy_primitives::keccak256([b"K:", keyword.as_bytes()].concat());
        hash[..3]
            .iter()
            .all(|&p| self.0[usize::from(p) / 8] & (1 << (p % 8)) != 0)
    }

    /// 根据 predicate 保守筛选；NOT 不能通过翻转 BF 结果实现。
    fn may_match(&self, predicate: &Predicate) -> bool {
        match predicate {
            Predicate::Keyword(word) => self.contains(word),
            Predicate::And(children) => children.iter().all(|child| self.may_match(child)),
            Predicate::Or(children) => children.iter().any(|child| self.may_match(child)),
            Predicate::Not(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::metadata::LogMetadata;

    /// 穷举四个关键词的全部组合，BF 查询必须与精确布尔判断一致。
    #[test]
    fn boolean_filter_preserves_exact_results() {
        let records: Vec<_> = (0u32..16)
            .map(|bits| {
                let mut metadata = LogMetadata::parse(crate::test_support::line()).unwrap();
                metadata.lid = if bits % 2 == 0 { "LG_1" } else { "LG_2" }.into();
                metadata.ts_min = u64::from(bits) * 10;
                metadata.ts_max = metadata.ts_min + 5;
                metadata.fw = ["FTP", "Admin", "incorrect_pwd", "incorrect_usr"]
                    .iter()
                    .enumerate()
                    .filter(|(bit, _)| bits & (1 << bit) != 0)
                    .map(|(_, word)| word.to_string())
                    .collect();
                IndexedRecord {
                    transaction_index: bits,
                    metadata,
                }
            })
            .collect();
        let tree = BfTree::decode(&BfTree::build(records.clone()).encoded()).unwrap();
        let predicates = [
            r#"{"op":"not","args":{"op":"keyword","args":"Admin"}}"#,
            r#"{"op":"and","args":[{"op":"keyword","args":"FTP"},{"op":"not","args":{"op":"keyword","args":"Admin"}},{"op":"or","args":[{"op":"keyword","args":"incorrect_pwd"},{"op":"keyword","args":"incorrect_usr"}]}]}"#,
        ];
        for predicate in predicates {
            for time in [(0, 200), (35, 35), (40, 100)] {
                for generators in [vec![], vec!["LG_1".into()], vec!["LG_2".into()]] {
                    let query = LogQuery::new(generators, time, predicate.as_bytes()).unwrap();
                    let expected: Vec<_> = records
                        .iter()
                        .filter(|r| query.matches(&r.metadata))
                        .map(|r| r.transaction_index)
                        .collect();
                    assert_eq!(tree.query(&query, &(0..16).collect::<Vec<_>>()), expected);
                }
            }
        }
        assert!(
            BfTree::build(vec![])
                .query(
                    &LogQuery::new(vec![], (0, 200), predicates[0].as_bytes()).unwrap(),
                    &[]
                )
                .is_empty()
        );
    }
}
