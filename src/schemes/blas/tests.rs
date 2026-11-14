//! BLAS 基本回归：扫描对照、时间边界、聚合一致性与块头剪枝。
use super::*;
use super::{common::*, index::*};
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

/// 将块内查询结果与逐条扫描比较，覆盖布尔条件与闭区间边界。
#[test]
fn block_proofs_match_scan() {
    let records = records();
    let index = transaction::BlockIndex::build(records.clone()).unwrap();
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
            let proof = index.prove(&query, None, true).unwrap().0;
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
        }
    }
}

/// 文件时间不要求输入有序，重复时间戳必须保留所有序号，未知词有缺失证明。
#[test]
fn file_proofs_cover_duplicates_and_not() {
    let file = entry::LogFile::build(entry::FileInput {
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
    .map(|rows| transaction::BlockIndex::build(rows).unwrap())
    .collect::<Vec<_>>();
    let window = segment::WindowIndex::build(1, &indexes).unwrap();
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
            let native = index.prove(&query, None, true).unwrap().0;
            if !native.objects.is_empty() {
                assert!(candidates.contains(&offset));
            }
            let paths = window.keyword_paths(&query, offset);
            let values = proof.keyword_values(&query, offset, &paths).unwrap();
            let aggregated = index.prove(&query, Some(&values), true).unwrap().0;
            assert_eq!(
                serde_json::to_value(&native.objects).unwrap(),
                serde_json::to_value(aggregated.objects).unwrap()
            );
        }
    }
}

/// NOT 只覆盖逻辑长度，单对象路径覆盖空树和非满树。
#[test]
fn bitmap_padding_and_merkle_boundaries() {
    let bitmap = bitmap::Bitmap::positions(9, [0, 8]);
    assert_eq!(bitmap.complement().indices(), (1..8).collect::<Vec<_>>());
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

/// 共享证明覆盖非满树，并比逐对象路径传输更少的兄弟哈希。
#[test]
fn multiproof_reuses_siblings() {
    for fanout in [2, 4] {
        for count in [1, 3, 5, 17] {
            let tree = merkle::MerkleTree::build(
                (0..count).map(|i| Bytes::from(vec![i])).collect(),
                fanout,
            );
            let indices: Vec<_> = (0..count as usize).filter(|i| i % 2 == 0).collect();
            let proof = tree.multi_prove(&indices).unwrap();
            proof.verify(tree.root()).unwrap();
            let separate: usize = indices
                .iter()
                .map(|i| tree.prove(*i).siblings.iter().map(Vec::len).sum::<usize>())
                .sum();
            assert!(proof.siblings.len() <= separate);
            if count > 3 {
                assert!(proof.siblings.len() < separate);
            }
        }
    }
}

/// 创建跨三块、夹空块及真实交易序号有间隔的窗口；返回可信头和两种证明。
fn generator_window_case(
    query: &LogQuery,
) -> (
    Vec<proof::TrustedHeader>,
    proof::QueryProof,
    proof::QueryProof,
) {
    let records = records();
    let indexes: Vec<_> = [
        records[..3].to_vec(),
        vec![],
        records[3..5].to_vec(),
        records[5..].to_vec(),
    ]
    .into_iter()
    .map(|rows| transaction::BlockIndex::build(rows).unwrap())
    .collect();
    let window = segment::WindowIndex::build(1, &indexes).unwrap();
    let headers = indexes
        .iter()
        .enumerate()
        .map(|(i, index)| {
            let summary = segment::BlockSummary::new(index);
            let mut roots = summary.roots;
            roots.push(if i == 3 { window.root() } else { B256::ZERO });
            proof::TrustedHeader {
                hash: B256::repeat_byte(i as u8),
                number: i as u64 + 1,
                fields: HeaderFields {
                    min_tx_time: summary.min_time,
                    max_tx_time: summary.max_time,
                    past_max_time: indexes[..=i]
                        .iter()
                        .map(|b| segment::BlockSummary::new(b).max_time)
                        .max()
                        .unwrap_or(0),
                    record_count: summary.count as u64,
                    roots,
                    window_size: 4,
                },
            }
        })
        .collect::<Vec<_>>();
    let native = proof::QueryProof {
        groups: indexes
            .iter()
            .zip(&headers)
            .map(|(index, header)| proof::GroupProof::Block {
                block: proof::BlockWitness {
                    hash: header.hash,
                    index: index.prove(query, None, true).unwrap().0,
                    keywords: vec![],
                },
            })
            .collect(),
    };
    let window_proof = window.prove(query);
    let mut selected = std::collections::BTreeMap::new();
    let blocks = window_proof
        .candidates(window.root(), query)
        .unwrap()
        .indices()
        .into_iter()
        .map(|offset| {
            let keywords = window.keyword_paths(query, offset);
            let values = window_proof
                .keyword_values(query, offset, &keywords)
                .unwrap();
            let (index, bitmap) = indexes[offset].prove(query, Some(&values), false).unwrap();
            for position in bitmap.indices() {
                selected.insert(
                    (offset as u64 + 1, position as u32),
                    indexes[offset].records[position].clone(),
                );
            }
            proof::BlockWitness {
                hash: headers[offset].hash,
                index,
                keywords,
            }
        })
        .collect();
    let generators = query
        .generators
        .iter()
        .map(|lid| window.generators.prove(*lid, &selected).unwrap())
        .collect();
    let aggregated = proof::QueryProof {
        groups: vec![proof::GroupProof::Window {
            end_hash: headers[3].hash,
            window: window_proof,
            blocks,
            generators,
        }],
    };
    (headers, native, aggregated)
}

/// 同一生成器跨块共享认证；多生成器、缺失生成器和空结果均与独立扫描一致。
#[test]
fn generator_aggregation_matches_scan_and_native_proofs() {
    for generators in [vec![1], vec![2], vec![1, 2], vec![3], vec![1, 3]] {
        for predicate in [
            Predicate::All,
            Predicate::Keyword("a".into()),
            Predicate::Not(Box::new(Predicate::Keyword("a".into()))),
            Predicate::Keyword("missing".into()),
        ] {
            for time in [(0, u64::MAX), (10, 10), (100, 101)] {
                let mut query = query(predicate.clone(), time);
                query.generators = generators.clone();
                let (headers, native, aggregated) = generator_window_case(&query);
                let expected: Vec<_> = records()
                    .into_iter()
                    .filter(|r| query.matches(&r.metadata))
                    .collect();
                let actual = aggregated.verify(&headers, &query).unwrap();
                let native = native.verify(&headers, &query).unwrap();
                assert_eq!(
                    actual.iter().map(|r| r.record.clone()).collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(native).unwrap()
                );
            }
        }
    }
}

/// 将指定时间 t 的元数据编码成真实交易信封，保持记录哈希和原始交易一致。
fn timed_transaction(t: u64) -> reth_ethereum::TransactionSigned {
    use alloy_consensus::{SignableTransaction, TxLegacy};
    use alloy_primitives::{Signature, TxKind, U256};
    let mut metadata = LogMetadata::parse(crate::test_support::line()).unwrap();
    metadata.t_min = t;
    metadata.t_max = t + 10;
    let mut input = crate::model::record::RECORD_TAG.to_vec();
    input.extend(serde_json::to_vec(&metadata).unwrap());
    TxLegacy {
        to: TxKind::Call(crate::model::record::DATA_ADDRESS),
        input: input.into(),
        ..Default::default()
    }
    .into_signed(Signature::new(U256::from(1), U256::from(1), false))
    .into()
}

/// 构造十二个带乱序日志时间的块；计数 reader 实际读取的完整块体。
fn pruning_chain() -> (
    Vec<proof::TrustedHeader>,
    crate::scheme::SchemeReader,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use crate::chain::{BlockView, ChainReader, HeaderView};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let mut headers = Vec::new();
    let mut indexes = Vec::new();
    let mut bodies = Vec::new();
    let mut transactions = Vec::new();
    let mut past_max_time = 0;
    for number in 1..=12 {
        let txs = vec![timed_transaction(if number == 2 { 500 } else { 100 })];
        let records = crate::model::record::records_from_transactions(&txs).unwrap();
        let index = transaction::BlockIndex::build(records).unwrap();
        let summary = segment::BlockSummary::new(&index);
        past_max_time = past_max_time.max(summary.max_time);
        indexes.push(index.clone());
        let windows = if number % 4 == 0 {
            vec![segment::WindowIndex::build(number - 3, &indexes[indexes.len() - 4..]).unwrap()]
        } else {
            vec![]
        };
        let mut roots = summary.roots;
        roots.push(
            windows
                .first()
                .map(segment::WindowIndex::root)
                .unwrap_or(B256::ZERO),
        );
        bodies.push(BodyFields { index, windows });
        transactions.push(txs);
        headers.push(proof::TrustedHeader {
            hash: B256::repeat_byte(number as u8),
            number,
            fields: HeaderFields {
                min_tx_time: summary.min_time,
                max_tx_time: summary.max_time,
                past_max_time,
                record_count: 1,
                roots,
                window_size: 4,
            },
        });
    }
    let view = |h: &proof::TrustedHeader| HeaderView {
        hash: h.hash,
        parent_hash: B256::repeat_byte(h.number as u8 - 1),
        number: h.number,
        fields: h.fields.clone(),
    };
    let views: Vec<_> = headers.iter().map(view).collect();
    let heads = views.clone();
    let counter = reads.clone();
    let reader = ChainReader::new(
        Arc::new(move |hash| Ok(heads.iter().find(|h| h.hash == hash).cloned())),
        Arc::new(move |hash| {
            counter.fetch_add(1, Ordering::Relaxed);
            let i = views.iter().position(|h| h.hash == hash).unwrap();
            Ok(Some(BlockView {
                header: views[i].clone(),
                transactions: transactions[i].clone(),
                fields: bodies[i].clone(),
            }))
        }),
    );
    (headers, reader, reads)
}

/// 头部剪枝把十二次完整块读取降为零，且保持普通查询和客户端结果一致。
#[test]
fn header_pruning_skips_segments_and_stops_history() {
    use crate::query::context::{QueryContext, QueryFailure, WorkBudget};
    use std::sync::atomic::Ordering;
    struct Budget;
    impl WorkBudget for Budget {
        /// 测试只计数完整块读取；gas 在真实合约验收中比较。
        fn spend(&mut self, _: u64) -> Result<(), QueryFailure> {
            Ok(())
        }
    }
    let (headers, reader, reads) = pruning_chain();
    let ancestors = headers.iter().map(|h| (h.hash, h.number)).collect();
    let context = QueryContext::new(1, 12, headers[11].hash, &reader, ancestors);
    for (time, groups) in [((200, 210), 3), ((600, 610), 1)] {
        for aggregate in [false, true] {
            reads.store(0, Ordering::Relaxed);
            let mut query = query(Predicate::All, time);
            query.aggregate = aggregate;
            let response = query::execute(&context, &query, &mut Budget).unwrap();
            let proof: proof::QueryProof = serde_json::from_slice(&response.proof).unwrap();
            assert!(proof.verify(&headers, &query).unwrap().is_empty());
            assert!(response.matches.is_empty());
            assert_eq!(
                reads.load(Ordering::Relaxed),
                if aggregate { 0 } else { 12 }
            );
            if aggregate {
                assert_eq!(proof.groups.len(), groups);
            }
        }
    }
    for aggregate in [false, true] {
        reads.store(0, Ordering::Relaxed);
        let mut query = query(Predicate::All, (500, 510));
        query.aggregate = aggregate;
        let response = query::execute(&context, &query, &mut Budget).unwrap();
        let proof: proof::QueryProof = serde_json::from_slice(&response.proof).unwrap();
        let matched = proof
            .verify_response(&headers, &query, &response.matches)
            .unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].block_hash, headers[1].hash);
        assert_eq!(
            reads.load(Ordering::Relaxed),
            if aggregate { 2 } else { 12 }
        );
    }
    assert!(!headers[0].fields.outside((110, 110)));
    assert!(headers[0].fields.outside((111, 111)));
}
