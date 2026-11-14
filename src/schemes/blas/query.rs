//! BLAS 原生查询：普通块内路径与窗口共享目录路径，返回结果及独立 VO。
use super::{
    BodyFields, HeaderFields,
    common::bitmap::Bitmap,
    index::segment::WINDOW_BLOCKS,
    proof::{BlockWitness, GroupProof, QueryProof},
};
use crate::{
    chain::BlockView,
    model::query::LogQuery,
    query::{
        abi::{QueryMatch, QueryResponse},
        context::{QueryContext, QueryFailure, WorkBudget},
        result,
    },
};

use std::collections::BTreeMap;

/// 按命中 bitmap 读取 block 中的真实交易位置，复用公共 ABI 格式化。
fn rows(
    block: &BlockView<HeaderFields, BodyFields>,
    bitmap: &Bitmap,
) -> Result<Vec<QueryMatch>, QueryFailure> {
    let positions = bitmap
        .indices()
        .into_iter()
        .map(|i| block.fields.index.records[i].transaction_index)
        .collect();
    result::matches(block, positions)
}

/// 在已认证 context 内执行 query；所有索引读取及证明输出均计入 gas。
pub fn execute(
    context: &QueryContext<'_, HeaderFields, BodyFields>,
    query: &LogQuery,
    budget: &mut dyn WorkBudget,
) -> Result<QueryResponse, QueryFailure> {
    let mut hash = context.end_hash;
    let mut height = context.end;
    let mut groups = Vec::new();
    let mut chunks = Vec::new();
    while height >= context.start {
        let header = context.header(hash, budget)?;
        if query.aggregate && header.fields.past_max_time < query.time.0 {
            groups.push(GroupProof::Skip {
                count: (height - context.start + 1) as usize,
            });
            break;
        }
        let size = if query.aggregate
            && height % WINDOW_BLOCKS == 0
            && height - WINDOW_BLOCKS + 1 >= context.start
        {
            WINDOW_BLOCKS
        } else {
            1
        };
        let mut cursor = header.parent_hash;
        let mut headers = vec![header];
        for _ in 1..size {
            let item = context.header(cursor, budget)?;
            cursor = item.parent_hash;
            headers.push(item);
        }
        headers.reverse();
        if query.aggregate && headers.iter().all(|h| h.fields.outside(query.time)) {
            groups.push(GroupProof::Skip {
                count: size as usize,
            });
            hash = cursor;
            height -= size;
            continue;
        }
        let block = context.block(hash, budget)?;
        budget.spend(500 + block.fields.index.records.len() as u64 * 100)?;
        if size == WINDOW_BLOCKS {
            let window = block
                .fields
                .windows
                .first()
                .ok_or_else(|| QueryFailure::Storage("完整窗口索引缺失".into()))?;
            let proof = window.prove(query);
            let candidates = proof
                .candidates(headers.last().unwrap().fields.roots[5], query)
                .map_err(QueryFailure::Storage)?
                .indices();
            let mut selected = BTreeMap::new();
            let mut witnesses = Vec::new();
            let mut matches = Vec::new();
            for offset in candidates {
                let candidate = context.block(headers[offset].hash, budget)?;
                let paths = window.keyword_paths(query, offset);
                let values = proof
                    .keyword_values(query, offset, &paths)
                    .map_err(QueryFailure::Storage)?;
                let use_omt = query.generators.is_empty();
                let (index, bitmap) = candidate
                    .fields
                    .index
                    .prove(query, Some(&values), use_omt)
                    .map_err(QueryFailure::Storage)?;
                matches.extend(rows(&candidate, &bitmap)?);
                if !use_omt {
                    for position in bitmap.indices() {
                        selected.insert(
                            (window.start + offset as u64, position as u32),
                            candidate.fields.index.records[position].clone(),
                        );
                    }
                }
                witnesses.push(BlockWitness {
                    hash: headers[offset].hash,
                    index,
                    keywords: paths,
                });
            }
            if !query.generators.is_empty() {
                let count: usize = window.generators.trees.iter().map(|t| t.leaves.len()).sum();
                budget.spend(500 * query.generators.len() as u64 + 100 * count as u64)?;
            }
            let generators = query
                .generators
                .iter()
                .map(|lid| window.generators.prove(*lid, &selected))
                .collect::<Result<Vec<_>, _>>()
                .map_err(QueryFailure::Storage)?;
            groups.push(GroupProof::Window {
                end_hash: hash,
                window: proof,
                blocks: witnesses,
                generators,
            });
            chunks.push(matches);
        } else {
            let (index, bitmap) = block
                .fields
                .index
                .prove(query, None, true)
                .map_err(QueryFailure::Storage)?;
            chunks.push(rows(&block, &bitmap)?);
            groups.push(GroupProof::Block {
                block: BlockWitness {
                    hash,
                    index,
                    keywords: vec![],
                },
            });
        }
        hash = cursor;
        height -= size;
    }
    groups.reverse();
    let proof = serde_json::to_vec(&QueryProof { groups })
        .map_err(|error| QueryFailure::Storage(error.to_string()))?;
    budget.spend(proof.len() as u64 * 4)?;
    Ok(QueryResponse {
        matches: chunks.into_iter().rev().flatten().collect(),
        proof: proof.into(),
    })
}
