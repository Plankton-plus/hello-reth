//! 所有接收路径先校验完整索引，再沿用 Ethereum 的状态与共识校验。

use crate::{
    node::block::{IndexedBlock, IndexedBody, IndexedPrimitives},
    node::payload::{IndexedEngineTypes, IndexedExecutionData},
    node::{chainspec::IndexedChainSpec, header::IndexedHeader},
    query::block::reader,
    scheme::SchemeReader,
};
use alloy_primitives::B256;
use reth_ethereum::{
    chainspec::ChainSpec,
    consensus::{
        Consensus, ConsensusError, EthBeaconConsensus, FullConsensus, HeaderValidator,
        ReceiptRootBloom,
    },
    node::{
        api::{
            AddOnsContext, FullNodeComponents, FullNodeTypes, NewPayloadError, NodeTypes,
            PayloadValidator,
        },
        builder::{BuilderContext, components::ConsensusBuilder, rpc::PayloadValidatorBuilder},
    },
    primitives::{Block, RecoveredBlock, SealedBlock, SealedHeader},
};
use reth_execution_types::BlockExecutionResult;
use std::sync::Arc;

/// Engine payload 入口：检查标准字段与索引承诺，完整重建留给执行前校验。
#[derive(Clone, Debug)]
pub struct IndexedPayloadValidator(
    pub reth_ethereum_payload_builder::EthereumExecutionPayloadValidator<ChainSpec>,
);
impl PayloadValidator<IndexedEngineTypes> for IndexedPayloadValidator {
    type Block = IndexedBlock;
    /// payload 明确携带索引；缺失或伪造时不构造替代索引。
    fn convert_payload_to_block(
        &self,
        payload: IndexedExecutionData,
    ) -> Result<SealedBlock<IndexedBlock>, NewPayloadError> {
        let ethereum = self
            .0
            .ensure_well_formed_payload(payload.ethereum)?
            .into_block();
        let mut header = IndexedHeader::from(ethereum.header);
        header.scheme = payload.scheme;
        header.version = payload.version;
        header.fields = payload.header_fields;
        let block = IndexedBlock {
            header,
            body: IndexedBody {
                ethereum: alloy_consensus::BlockBody {
                    transactions: ethereum.body.transactions,
                    ommers: ethereum.body.ommers.into_iter().map(Into::into).collect(),
                    withdrawals: ethereum.body.withdrawals,
                },
                fields: payload.body_fields,
            },
        };
        block
            .validate_commitment()
            .map_err(|e| NewPayloadError::other(std::io::Error::other(e)))?;
        let sealed = block.seal_slow();
        if sealed.hash() != payload.block_hash {
            return Err(NewPayloadError::other(std::io::Error::other(
                "完整自定义区块哈希不匹配",
            )));
        }
        Ok(sealed)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedValidatorBuilder;
impl<N> PayloadValidatorBuilder<N> for IndexedValidatorBuilder
where
    N: FullNodeComponents<
        Types: NodeTypes<ChainSpec = IndexedChainSpec, Payload = IndexedEngineTypes>,
    >,
{
    type Validator = IndexedPayloadValidator;
    /// 使用节点的真实链规则创建接收校验器。
    async fn build(self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<Self::Validator> {
        Ok(IndexedPayloadValidator(
            reth_ethereum_payload_builder::EthereumExecutionPayloadValidator::new(Arc::new(
                ctx.config
                    .chain
                    .as_ref()
                    .clone()
                    .map_header(|header| header.ethereum),
            )),
        ))
    }
}

/// P2P 和引擎共用的区块校验器，保留所有默认 Ethereum 校验。
#[derive(Clone, Debug)]
pub struct IndexedConsensus(pub EthBeaconConsensus<IndexedChainSpec>, pub SchemeReader);

impl HeaderValidator<IndexedHeader> for IndexedConsensus {
    /// 校验 header 的标准字段。
    fn validate_header(&self, header: &SealedHeader<IndexedHeader>) -> Result<(), ConsensusError> {
        header.validate_protocol().map_err(ConsensusError::msg)?;
        self.0.validate_header(header)
    }
    /// 校验 header 与 parent 的关系。
    fn validate_header_against_parent(
        &self,
        header: &SealedHeader<IndexedHeader>,
        parent: &SealedHeader<IndexedHeader>,
    ) -> Result<(), ConsensusError> {
        self.0.validate_header_against_parent(header, parent)
    }
}

impl Consensus<IndexedBlock> for IndexedConsensus {
    /// 检查 body 与 header 的承诺关系；完整索引重建只在执行前进行。
    fn validate_body_against_header(
        &self,
        body: &IndexedBody,
        header: &SealedHeader<IndexedHeader>,
    ) -> Result<(), ConsensusError> {
        IndexedBlock {
            header: header.header().clone(),
            body: body.clone(),
        }
        .validate_commitment()
        .map_err(ConsensusError::msg)?;
        <EthBeaconConsensus<IndexedChainSpec> as Consensus<IndexedBlock>>::validate_body_against_header(
            &self.0, body, header,
        )
    }
    /// 对完整 block 做执行前检查，网络导入同样必须通过索引校验。
    fn validate_block_pre_execution(
        &self,
        block: &SealedBlock<IndexedBlock>,
    ) -> Result<(), ConsensusError> {
        block
            .clone()
            .into_block()
            .validate_index(&self.1)
            .map_err(ConsensusError::msg)?;
        self.0.validate_block_pre_execution(block)
    }
}

impl FullConsensus<IndexedPrimitives> for IndexedConsensus {
    /// 原样校验 block 的执行结果 result、回执根 bloom 和访问列表哈希 bal。
    fn validate_block_post_execution(
        &self,
        block: &RecoveredBlock<IndexedBlock>,
        result: &BlockExecutionResult<reth_ethereum::Receipt>,
        bloom: Option<ReceiptRootBloom>,
        bal: Option<B256>,
    ) -> Result<(), ConsensusError> {
        <EthBeaconConsensus<IndexedChainSpec> as FullConsensus<IndexedPrimitives>>::validate_block_post_execution(&self.0, block, result, bloom, bal)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedConsensusBuilder;
impl<N> ConsensusBuilder<N> for IndexedConsensusBuilder
where
    N: FullNodeTypes<
        Types: NodeTypes<ChainSpec = IndexedChainSpec, Primitives = IndexedPrimitives>,
    >,
{
    type Consensus = Arc<IndexedConsensus>;
    /// 使用 ctx 中的链配置创建共识校验器。
    async fn build_consensus(self, ctx: &BuilderContext<N>) -> eyre::Result<Self::Consensus> {
        Ok(Arc::new(IndexedConsensus(
            EthBeaconConsensus::new(ctx.chain_spec()),
            reader(ctx.provider().clone()),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    /// 即使伪造索引同时更新了承诺，执行前入口仍必须重建并拒绝它。
    #[test]
    fn rejects_forged_index_before_execution() {
        let mut block = test_support::block();
        block.body.ethereum.transactions.clear();
        block.validate_commitment().unwrap();
        let consensus = IndexedConsensus(
            EthBeaconConsensus::new(Arc::new(
                reth_ethereum::chainspec::DEV
                    .as_ref()
                    .clone()
                    .map_header(IndexedHeader::from),
            )),
            test_support::reader(),
        );
        let error = consensus
            .validate_block_pre_execution(&block.seal_slow())
            .unwrap_err();
        assert!(error.to_string().contains("索引"));
    }
}
