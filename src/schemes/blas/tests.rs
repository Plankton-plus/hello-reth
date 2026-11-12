//! BLAS 独立证明、空集、非二次幂位图与窗口聚合语义的可检查测试。
use super::*;
use crate::model::{entry::Entry, metadata::LogMetadata, predicate::Predicate};
use alloy_primitives::Bytes;

/// 创建不依赖链状态的业务查询，entry_predicate 由文件测试另行指定。
fn query(predicate: Predicate, time: (u64, u64)) -> LogQuery {
    LogQuery {
        generators: vec![],
        time,
        predicate,
        entry_predicate: None,
        aggregate: true,
    }
}

/// 创建三位关键词真值组合的八条记录，真实交易位置刻意包含空隙。
fn records() -> Vec<IndexedRecord> {
    (0..8)
        .map(|value| {
            let mut metadata: LogMetadata =
                serde_json::from_str(crate::test_support::line()).unwrap();
            metadata.lid = value % 2 + 1;
            metadata.t_min = (value / 2) as u64 * 10;
            metadata.t_max = metadata.t_min + 10;
            metadata.tx_public_keys = ["a", "b", "c"]
                .iter()
                .enumerate()
                .filter(|(i, _)| value & (1 << i) != 0)
                .map(|(_, s)| (*s).into())
                .collect();
            IndexedRecord {
                transaction_index: value * 2,
                transaction_hash: B256::repeat_byte(value as u8),
                metadata,
            }
        })
        .collect()
}

/// 用独立逐条判断验证所有布尔与时间边界，并检查结果遗漏会失败。
#[test]
fn block_proofs_match_scan_and_reject_omissions() {
    let records = records();
    let index = block::BlockIndex::build(records.clone()).unwrap();
    let a = Predicate::Keyword("a".into());
    let missing = Predicate::Keyword("missing".into());
    for predicate in [
        Predicate::All,
        a.clone(),
        Predicate::Not(Box::new(a.clone())),
        missing.clone(),
        Predicate::Not(Box::new(missing)),
        Predicate::Or(vec![a.clone(), Predicate::Keyword("b".into())]),
        Predicate::And(vec![
            Predicate::Not(Box::new(a)),
            Predicate::Keyword("c".into()),
        ]),
    ] {
        for time in [(0, u64::MAX), (10, 10), (11, 19), (100, 101)] {
            let query = query(predicate.clone(), time);
            let proof = index.prove(&query).unwrap();
            let bitmap = proof
                .filters
                .verify(&index.filters.roots(), records.len(), &query, false)
                .unwrap();
            let actual: Vec<IndexedRecord> = proof
                .objects(index.omt.root(), records.len(), &bitmap)
                .unwrap()
                .iter()
                .map(|bytes| ordered::decode(bytes).unwrap())
                .collect();
            let expected: Vec<_> = records
                .iter()
                .filter(|record| query.matches(&record.metadata))
                .cloned()
                .collect();
            assert_eq!(actual, expected);
            if !actual.is_empty() {
                let mut forged = proof.clone();
                forged.objects.pop();
                assert!(
                    forged
                        .objects(index.omt.root(), records.len(), &bitmap)
                        .is_err()
                );
            }
        }
    }
    let mut corrupt = index.clone();
    corrupt.records[0].metadata.lid = 99;
    assert_ne!(
        block::BlockIndex::build(corrupt.records.clone()).unwrap(),
        corrupt
    );
}

/// 文件时间不要求输入有序，重复时间戳必须保留所有序号，未知词有缺失证明。
#[test]
fn file_proofs_cover_duplicates_not_and_tampering() {
    let file = file::LogFile::build(file::FileInput {
        lid: 1,
        tx_public_keywords: vec!["public".into()],
        entries: [110, 100, 110, 130]
            .into_iter()
            .enumerate()
            .map(|(i, timestamp)| Entry {
                sn: i as u32,
                timestamp,
                keywords: if i % 2 == 0 {
                    vec!["a".into()]
                } else {
                    vec!["b".into()]
                },
                log_value: format!("entry {i}"),
            })
            .collect(),
    })
    .unwrap();
    for predicate in [
        Predicate::All,
        Predicate::Keyword("a".into()),
        Predicate::Keyword("unknown".into()),
        Predicate::Not(Box::new(Predicate::Keyword("unknown".into()))),
    ] {
        for time in [(0, u64::MAX), (110, 110), (111, 129), (0, 0)] {
            let mut query = query(Predicate::All, time);
            query.entry_predicate = Some(predicate.clone());
            let proof = file.prove(&query).unwrap();
            let entries = proof.verify(&file.metadata, &query).unwrap();
            let expected: Vec<_> = file
                .entries
                .iter()
                .filter(|e| {
                    e.timestamp >= time.0 && e.timestamp <= time.1 && predicate.matches(&e.keywords)
                })
                .cloned()
                .collect();
            assert_eq!(entries, expected);
            if !entries.is_empty() {
                let mut forged = proof.clone();
                forged.objects[0].leaf = Bytes::from_static(b"forged");
                assert!(forged.verify(&file.metadata, &query).is_err());
            }
            let mut forged = proof;
            forged.count += 1;
            assert!(forged.verify(&file.metadata, &query).is_err());
        }
    }
}

/// ALIS 下层位图与各块 IIMT 必须一致，包含空块与全窗口缺失关键词。
#[test]
fn window_shared_paths_match_native_indexes() {
    let records = records();
    let indexes = [
        records[..3].to_vec(),
        vec![],
        records[3..5].to_vec(),
        records[5..].to_vec(),
    ]
    .into_iter()
    .map(|rows| block::BlockIndex::build(rows).unwrap())
    .collect::<Vec<_>>();
    let window = window::WindowIndex::build(1, &indexes).unwrap();
    window.validate().unwrap();
    for predicate in [
        Predicate::All,
        Predicate::Keyword("a".into()),
        Predicate::Not(Box::new(Predicate::Keyword("a".into()))),
        Predicate::Keyword("missing".into()),
        Predicate::Not(Box::new(Predicate::Keyword("missing".into()))),
    ] {
        let query = query(predicate, (0, u64::MAX));
        let proof = window.prove(&query);
        let candidates = proof.candidates(window.root(), &query).unwrap().indices();
        for (offset, index) in indexes.iter().enumerate() {
            let native = index.prove(&query).unwrap();
            if !native.objects.is_empty() {
                assert!(candidates.contains(&offset));
            }
            let paths = window.keyword_paths(&query, offset);
            let values = proof.keyword_values(&query, offset, &paths).unwrap();
            let aggregated = index.prove_aggregated(&query, &values).unwrap();
            assert_eq!(
                serde_json::to_value(&native.objects).unwrap(),
                serde_json::to_value(aggregated.objects).unwrap()
            );
        }
        let mut forged = proof;
        forged.descriptor.blocks[0].count += 1;
        assert!(forged.candidates(window.root(), &query).is_err());
    }
}

/// NOT 只覆盖逻辑长度，补位或截断位图不应成为不存在的对象。
#[test]
fn bitmap_padding_and_merkle_boundaries() {
    let bitmap = bitmap::Bitmap::positions(9, [0, 8]);
    assert_eq!(bitmap.complement().indices(), (1..8).collect::<Vec<_>>());
    let mut forged = bitmap;
    forged.bits = vec![1, 255].into();
    assert!(forged.validate(9).is_err());
    for count in [0, 1, 3, 5, 17] {
        let tree = merkle::MerkleTree::build((0..count).map(|i| Bytes::from(vec![i])).collect(), 4);
        tree.validate().unwrap();
        for i in 0..count as usize {
            tree.prove(i).verify(tree.root()).unwrap();
        }
    }
}

/// 分隔键路由必须覆盖非满二叉和四叉树，并对首尾缺失键给出正确邻接证明。
#[test]
fn ordered_routing_covers_partial_nodes() {
    for count in [0, 1, 3, 5, 17, 65] {
        for fanout in [2, 4] {
            let values = (0..count)
                .map(|i| (filters::number(i * 2), Bytes::new()))
                .collect();
            let index = ordered::OrderedIndex::build(values, fanout);
            for value in 0..count * 2 + 2 {
                let key = filters::number(value);
                let proof = index.prove(&key);
                let actual = proof
                    .floor(index.tree.root(), &key)
                    .unwrap()
                    .map(|leaf| leaf.key);
                let expected = index
                    .leaves
                    .iter()
                    .rev()
                    .find(|leaf| leaf.key <= key)
                    .map(|leaf| leaf.key.clone());
                assert_eq!(actual, expected);
            }
        }
    }
}
