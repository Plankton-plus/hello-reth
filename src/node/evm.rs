//! 扩展区块的执行配置：Ethereum 状态执行不变，主体类型包含完整索引。

use crate::{
    node::block::{IndexedBlock, IndexedPrimitives},
    node::payload::IndexedExecutionData,
    node::{
        chainspec::IndexedChainSpec,
        header::{IndexedHeader, execution_parent},
    },
    query::block::{IndexReader, reader},
    query::precompile::QueryEvmFactory,
};
use reth_ethereum::{
    chainspec::ChainSpec,
    evm::{
        EthBlockAssembler, EthBlockExecutionCtx, EthEvmConfig,
        primitives::{
            BlockAssembler, BlockAssemblerInput, BlockExecutionError, ConfigureEngineEvm,
            ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor, NextBlockEnvAttributes,
        },
    },
    node::{
        api::{FullNodeTypes, NodeTypes},
        builder::{BuilderContext, components::ExecutorBuilder},
    },
    primitives::{SealedBlock, SealedHeader},
};
use std::borrow::Cow;
use std::sync::Arc;

pub type QueryEthEvmConfig = EthEvmConfig<ChainSpec, QueryEvmFactory>;
type Factory = <QueryEthEvmConfig as ConfigureEvm>::BlockExecutorFactory;

#[derive(Clone, Debug)]
pub struct IndexedAssembler(EthBlockAssembler, IndexReader);
impl BlockAssembler<Factory> for IndexedAssembler {
    type Block = IndexedBlock;
    /// 组装 input 的最终交易并将完整索引写入原生区块体。
    fn assemble_block(
        &self,
        input: BlockAssemblerInput<'_, '_, Factory, IndexedHeader>,
    ) -> Result<IndexedBlock, BlockExecutionError> {
        let parent = execution_parent(input.parent);
        let input: BlockAssemblerInput<'_, '_, Factory> = BlockAssemblerInput::new(
            input.evm_env,
            input.execution_ctx,
            &parent,
            input.transactions,
            input.output,
            input.execution_state,
            input.state_provider,
            input.state_root,
            input.block_access_list_hash,
        );
        let ethereum = self.0.assemble_block(input, None, None, None)?;
        IndexedBlock::from_ethereum(ethereum, &self.1).map_err(BlockExecutionError::msg)
    }
}

#[derive(Clone, Debug)]
pub struct IndexedEvmConfig {
    pub(crate) inner: QueryEthEvmConfig,
    assembler: IndexedAssembler,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedExecutorBuilder;

impl ConfigureEvm for IndexedEvmConfig {
    type Primitives = IndexedPrimitives;
    type Error = <QueryEthEvmConfig as ConfigureEvm>::Error;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = Factory;
    type BlockAssembler = IndexedAssembler;
    /// 返回共用查询预编译的执行工厂。
    fn block_executor_factory(&self) -> &Factory {
        self.inner.block_executor_factory()
    }
    /// 返回生成扩展区块体的组装器。
    fn block_assembler(&self) -> &IndexedAssembler {
        &self.assembler
    }
    /// 根据 header 生成标准 EVM 环境。
    fn evm_env(&self, header: &IndexedHeader) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(&header.ethereum)
    }
    /// 根据 parent 和 attributes 生成下一个执行环境。
    fn next_evm_env(
        &self,
        parent: &IndexedHeader,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.next_evm_env(&parent.ethereum, attributes)
    }
    /// 从完整 block 取标准执行字段，索引由共识校验器单独验证。
    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<IndexedBlock>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error>
    where
        Self: 'a,
    {
        let body = &block.body().ethereum;
        let header = block.header();
        Ok(EthBlockExecutionCtx {
            tx_count_hint: Some(body.transactions.len()),
            parent_hash: header.parent_hash,
            parent_beacon_block_root: header.parent_beacon_block_root,
            ommers: &[], // 实验协议明确拒绝叔块。
            withdrawals: body
                .withdrawals
                .as_ref()
                .map(|w| Cow::Borrowed(w.as_slice())),
            extra_data: header.extra_data.clone(),
            slot_number: header.slot_number,
        })
    }
    /// 从 parent 和 attributes 取默认构建上下文。
    fn context_for_next_block(
        &self,
        parent: &SealedHeader<IndexedHeader>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        Ok(EthBlockExecutionCtx {
            tx_count_hint: None,
            parent_hash: parent.hash(),
            parent_beacon_block_root: attributes.parent_beacon_block_root,
            ommers: &[],
            withdrawals: attributes.withdrawals.map(|w| Cow::Owned(w.into_inner())),
            extra_data: attributes.extra_data,
            slot_number: attributes.slot_number,
        })
    }
}

impl ConfigureEngineEvm<IndexedExecutionData> for IndexedEvmConfig {
    /// payload 的标准执行环境保持不变。
    fn evm_env_for_payload(
        &self,
        payload: &IndexedExecutionData,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env_for_payload(&payload.ethereum)
    }
    /// 从 payload 借用标准执行上下文。
    fn context_for_payload<'a>(
        &self,
        payload: &'a IndexedExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(&payload.ethereum)
    }
    /// 复用默认交易解码与签名恢复，索引不是 EVM 交易。
    fn tx_iterator_for_payload(
        &self,
        payload: &IndexedExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        self.inner.tx_iterator_for_payload(&payload.ethereum)
    }
}

impl<N> ExecutorBuilder<N> for IndexedExecutorBuilder
where
    N: FullNodeTypes<
        Types: NodeTypes<ChainSpec = IndexedChainSpec, Primitives = IndexedPrimitives>,
    >,
{
    type EVM = IndexedEvmConfig;
    /// 根据 ctx 的链配置和真实 provider 安装查询工厂与扩展组装器。
    async fn build_evm(self, ctx: &BuilderContext<N>) -> eyre::Result<Self::EVM> {
        let reader = reader(ctx.provider().clone());
        let spec = Arc::new(
            ctx.chain_spec()
                .as_ref()
                .clone()
                .map_header(|header| header.ethereum),
        );
        let inner = EthEvmConfig::new_with_evm_factory(spec, QueryEvmFactory(reader.clone()));
        let assembler = IndexedAssembler(inner.block_assembler.clone(), reader);
        Ok(IndexedEvmConfig { inner, assembler })
    }
}
