//! BLAS 原生查询：普通块内路径与窗口共享目录路径，返回结果及独立 VO。
use super::{
    BodyFields, HeaderFields,
    block::IndexProof,
    ordered::decode,
    proof::{BlockWitness, GroupProof, QueryProof},
    window::WINDOW_BLOCKS,
};
use crate::{
    chain::BlockView,
    model::{metadata::IndexedRecord, query::LogQuery},
    query::{
        abi::{QueryMatch, QueryResponse},
        context::{QueryContext, QueryFailure, WorkBudget},
        result,
    },
};

/// 从已证明对象得到真实交易位置，并复用公共 ABI 格式化。
fn rows(
    block: &BlockView<HeaderFields, BodyFields>,
    proof: &IndexProof,
) -> Result<Vec<QueryMatch>, QueryFailure> {
    let positions = proof
        .objects
        .iter()
        .map(|path| decode::<IndexedRecord>(&path.leaf).map(|r| r.transaction_index))
        .collect::<Result<Vec<_>, _>>()
        .map_err(QueryFailure::Storage)?;
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
        if header.number != height {
            return Err(QueryFailure::Storage("BLAS 历史块不连续".into()));
        }
        let block = context.block(hash, budget)?;
        budget.spend(500 + block.fields.index.records.len() as u64 * 100)?;
        if query.aggregate
            && height % WINDOW_BLOCKS == 0
            && height - WINDOW_BLOCKS + 1 >= context.start
        {
            let window = block
                .fields
                .windows
                .first()
                .ok_or_else(|| QueryFailure::Storage("完整窗口索引缺失".into()))?;
            let proof = window.prove(query);
            let candidates = proof
                .candidates(header.fields.roots[5], query)
                .map_err(QueryFailure::Storage)?
                .indices();
            let mut hashes = Vec::new();
            let mut cursor = hash;
            for _ in 0..WINDOW_BLOCKS {
                let item = context.header(cursor, budget)?;
                hashes.push(cursor);
                cursor = item.parent_hash;
            }
            hashes.reverse();
            let mut witnesses = Vec::new();
            let mut matches = Vec::new();
            for offset in candidates {
                let candidate = context.block(hashes[offset], budget)?;
                let paths = window.keyword_paths(query, offset);
                let values = proof
                    .keyword_values(query, offset, &paths)
                    .map_err(QueryFailure::Storage)?;
                let index = candidate
                    .fields
                    .index
                    .prove_aggregated(query, &values)
                    .map_err(QueryFailure::Storage)?;
                matches.extend(rows(&candidate, &index)?);
                witnesses.push(BlockWitness {
                    hash: hashes[offset],
                    index,
                    keywords: paths,
                });
            }
            groups.push(GroupProof::Window {
                end_hash: hash,
                window: proof,
                blocks: witnesses,
            });
            chunks.push(matches);
            hash = cursor;
            height -= WINDOW_BLOCKS;
        } else {
            let index = block
                .fields
                .index
                .prove(query)
                .map_err(QueryFailure::Storage)?;
            chunks.push(rows(&block, &index)?);
            groups.push(GroupProof::Block {
                block: BlockWitness {
                    hash,
                    index,
                    keywords: vec![],
                },
            });
            hash = header.parent_hash;
            height -= 1;
        }
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
