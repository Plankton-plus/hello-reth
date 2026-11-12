//! 查询公共入口：解码、认证快照、计费，再调用当前方案的完整查询算法。

use crate::{
    model::query::LogQuery,
    query::{
        abi::{QueryError, QueryRequest},
        block::IndexReader,
        context::{QueryContext, QueryFailure, WorkBudget},
    },
    scheme::{ActiveScheme, Scheme},
};
use alloy_primitives::B256;
use alloy_sol_types::{SolError, SolValue};
use evm2::{
    BaseEvmTypes, Evm, PrecompileError,
    evm::precompile::PrecompileOutput,
    interpreter::{GasTracker, Host, Word},
    precompiles::{PrecompileHalt, PrecompileResult},
};
use std::collections::BTreeMap;

/// 将 reason 编码为合约可识别的 QueryError。
pub fn query_error(reason: impl Into<String>) -> PrecompileError {
    PrecompileError::Revert(
        QueryError {
            reason: reason.into(),
        }
        .abi_encode()
        .into(),
    )
}

/// 区分用户输入错误、存储故障和耗气失败，避免错误链上回执。
fn execution_error(error: QueryFailure) -> PrecompileError {
    match error {
        QueryFailure::Invalid(reason) => query_error(reason),
        QueryFailure::Storage(reason) => PrecompileError::fatal(std::io::Error::other(reason)),
        QueryFailure::OutOfGas => PrecompileHalt::OutOfGas.into(),
    }
}

struct Budget<'a>(&'a mut GasTracker);

impl WorkBudget for Budget<'_> {
    /// 将方案报告的 gas 计入本次 EVM 调用。
    fn spend(&mut self, gas: u64) -> Result<(), QueryFailure> {
        self.0.spend(gas).map_err(|_| QueryFailure::OutOfGas)
    }
}

/// 解码 input，形成所有方案共用的业务查询，拒绝超长载荷。
fn decode_query(input: &[u8]) -> Result<(QueryRequest, LogQuery), PrecompileError> {
    if input.len() > 8192 {
        return Err(query_error("查询输入超过 8192 字节"));
    }
    let request =
        QueryRequest::abi_decode_validate(input).map_err(|_| query_error("查询 ABI 非法"))?;
    let query = LogQuery::new(
        request.generators.clone(),
        (request.timeMin, request.timeMax),
        &request.predicate,
    )
    .map_err(query_error)?;
    Ok((request, query))
}

/// 用 EVM 历史哈希认证请求范围；保留原型的 32 块及最近 256 块边界。
/// todo 用户查询不会有哈希查询字段吧。
fn authenticate(
    request: &QueryRequest,
    evm: &mut Evm<'_, BaseEvmTypes>,
) -> Result<(u64, BTreeMap<B256, u64>), PrecompileError> {
    let current: u64 = evm
        .block()
        .number
        .try_into()
        .map_err(|_| query_error("调用块高溢出"))?;
    if request.startBlock == 0
        || request.startBlock >= current
        || current - request.startBlock > 256
    {
        return Err(query_error("起始块须在调用快照之前的最近 256 个祖先块内"));
    }
    if request.endBlockHash == B256::ZERO {
        return Err(query_error("结束块哈希不能为零"));
    }
    let last = request.startBlock.saturating_add(31).min(current - 1);
    let mut ancestors = BTreeMap::new();
    for height in request.startBlock..=last {
        let hash = evm.block_hash(&Word::from(height))?;
        ancestors.insert(hash, height);
        if hash == request.endBlockHash {
            return Ok((height, ancestors));
        }
    }
    Err(query_error("结束块未知、非历史祖先或跨度超过 32 块"))
}

/// 在 reader 的认证范围内执行 input；框架不规定块间和块内查询算法。
pub fn execute_query(
    reader: &IndexReader,
    evm: &mut Evm<'_, BaseEvmTypes>,
    input: &[u8],
    gas: &mut GasTracker,
) -> PrecompileResult {
    gas.spend(5000)?;
    let (request, query) = decode_query(input)?;
    let (end, ancestors) = authenticate(&request, evm)?;
    let context = QueryContext::new(
        request.startBlock,
        end,
        request.endBlockHash,
        reader,
        ancestors,
    );
    let matches =
        ActiveScheme::query(&context, &query, &mut Budget(gas)).map_err(execution_error)?;
    if matches.len() > 128 {
        return Err(query_error("结果超过 128 条，请缩小范围"));
    }
    let output = matches.abi_encode();
    if output.len() > 65536 {
        return Err(query_error("输出超过 64 KiB，请缩小范围"));
    }
    gas.spend(8 * output.len() as u64)?;
    Ok(PrecompileOutput::new(output.into()))
}
