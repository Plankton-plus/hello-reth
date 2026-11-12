//! 复用 Ethereum 交易选择与状态执行，将最终构建结果升级为原生扩展区块。

use crate::{
    node::block::IndexedPrimitives,
    node::evm::{IndexedEvmConfig, QueryEthEvmConfig},
    node::payload::{IndexedBuiltPayload, IndexedEngineTypes},
    node::{
        chainspec::IndexedChainSpec,
        header::{IndexedHeader, execution_parent},
    },
    query::block::reader,
    scheme::SchemeReader,
};
use reth_basic_payload_builder::{BuildArguments, BuildOutcome, PayloadBuilder, PayloadConfig};
use reth_ethereum::{
    TransactionSigned,
    chainspec::ChainSpecProvider,
    node::{
        api::{FullNodeTypes, NodeTypes},
        builder::{BuilderContext, PayloadBuilderConfig, components::PayloadBuilderBuilder},
    },
    pool::{PoolTransaction, TransactionPool},
    provider::{BlockReader, StateProviderFactory},
    rpc::types::engine::PayloadAttributes,
};
use reth_ethereum_payload_builder::{EthereumBuilderConfig, EthereumPayloadBuilder};
use reth_payload_builder::PayloadBuilderError;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedPayloadBuilderBuilder; //工厂，被Reth框架调用来创建真正的出块器

#[derive(Clone, Debug)]
pub struct IndexedPayloadBuilder<P, C>(
    EthereumPayloadBuilder<P, C, QueryEthEvmConfig>,
    SchemeReader,
);

impl<N, P> PayloadBuilderBuilder<N, P, IndexedEvmConfig> for IndexedPayloadBuilderBuilder
where
    N: FullNodeTypes<
        Types: NodeTypes<
            ChainSpec = IndexedChainSpec,
            Primitives = IndexedPrimitives,
            Payload = IndexedEngineTypes,
        >,
    >,
    P: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>
        + Unpin
        + 'static,
{
    type PayloadBuilder = IndexedPayloadBuilder<P, N::Provider>;
    /// 从 ctx、pool 创建构建器；EVM 执行沿用同一链规则。
    async fn build_payload_builder(
        self,
        ctx: &BuilderContext<N>, //构建上下文
        pool: P,                 //交易池
        evm: IndexedEvmConfig,   //执行配置
    ) -> eyre::Result<Self::PayloadBuilder> {
        Ok(IndexedPayloadBuilder(
            EthereumPayloadBuilder::new(
                ctx.provider().clone(),
                pool,
                evm.inner,
                EthereumBuilderConfig::new()
                    .with_extra_data(ctx.payload_builder_config().extra_data()),
            ),
            reader(ctx.provider().clone()),
        ))
    }
}

impl<P, C> PayloadBuilder for IndexedPayloadBuilder<P, C>
where
    P: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>,
    C: StateProviderFactory
        + ChainSpecProvider<ChainSpec = IndexedChainSpec>
        + BlockReader<Block = crate::node::block::IndexedBlock, Header = IndexedHeader>
        + Clone
        + Send
        + Sync
        + 'static,
{
    type Attributes = PayloadAttributes;
    type BuiltPayload = IndexedBuiltPayload;
    /// 原样传递 args 的取消、缓存和费用状态，仅升级成功构建的区块。
    fn try_build(
        &self,
        args: BuildArguments<PayloadAttributes, IndexedBuiltPayload>,
    ) -> Result<BuildOutcome<IndexedBuiltPayload>, PayloadBuilderError> {
        let BuildArguments {
            cached_reads,
            execution_cache,
            state_root_handle,
            config,
            cancel,
            best_payload,
        } = args;
        let outcome = self.0.try_build(BuildArguments {
            cached_reads,
            execution_cache,
            state_root_handle,
            config: execution_config(config),
            cancel,
            best_payload: best_payload.map(|p| p.inner),
        })?;
        match outcome {
            BuildOutcome::Better {
                payload,
                cached_reads,
            } => Ok(BuildOutcome::Better {
                payload: IndexedBuiltPayload::from_ethereum(payload, &self.1)
                    .map_err(|e| PayloadBuilderError::other(std::io::Error::other(e)))?,
                cached_reads,
            }),
            BuildOutcome::Freeze(payload) => Ok(BuildOutcome::Freeze(
                IndexedBuiltPayload::from_ethereum(payload, &self.1)
                    .map_err(|e| PayloadBuilderError::other(std::io::Error::other(e)))?,
            )),
            BuildOutcome::Aborted { fees, cached_reads } => {
                Ok(BuildOutcome::Aborted { fees, cached_reads })
            }
            BuildOutcome::Cancelled => Ok(BuildOutcome::Cancelled),
        }
    }
    /// 空块仍包含规范的空索引及其认证承诺。
    fn build_empty_payload(
        &self,
        config: PayloadConfig<PayloadAttributes, IndexedHeader>,
    ) -> Result<IndexedBuiltPayload, PayloadBuilderError> {
        IndexedBuiltPayload::from_ethereum(
            self.0.build_empty_payload(execution_config(config))?,
            &self.1,
        )
        .map_err(|e| PayloadBuilderError::other(std::io::Error::other(e)))
    }
}

/// 将 config 投影给标准交易执行器；父哈希仍是完整原生头的哈希。
fn execution_config(
    config: PayloadConfig<PayloadAttributes, IndexedHeader>,
) -> PayloadConfig<PayloadAttributes> {
    PayloadConfig {
        parent_header: Arc::new(execution_parent(&config.parent_header)),
        parent_block_info: config.parent_block_info,
        attributes: config.attributes,
        payload_id: config.payload_id,
    }
}
