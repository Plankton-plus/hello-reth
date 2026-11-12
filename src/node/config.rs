//! 组合扩展区块、同事务存储、接收校验和本地开发出块。固定套路无需动

use crate::{
    node::block::{IndexedBlock, IndexedPrimitives},
    node::builder::IndexedPayloadBuilderBuilder,
    node::chainspec::IndexedChainSpec,
    node::evm::IndexedExecutorBuilder,
    node::payload::IndexedEngineTypes,
    node::rpc::IndexedEngineApiBuilder,
    node::storage::IndexedStorage,
    node::validation::{IndexedConsensusBuilder, IndexedValidatorBuilder},
    node::{header::IndexedHeader, rpc_types::IndexedRpcTypes},
};
use reth_ethereum::{
    engine::local::LocalPayloadAttributesBuilder,
    node::{
        EthereumEthApiBuilder, EthereumNetworkBuilder, EthereumPoolBuilder,
        api::{FullNodeComponents, FullNodeTypes, NodeTypes, PayloadAttributesBuilder},
        builder::{
            DebugNode, Node, NodeAdapter,
            components::{BasicPayloadServiceBuilder, ComponentsBuilder},
            rpc::{BasicEngineValidatorBuilder, Identity, RpcAddOns},
        },
    },
    rpc::types::engine::PayloadAttributes,
};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct IndexedNode; //Reth节点是纯类型驱动，通过trait关联类型决定运行行为
impl NodeTypes for IndexedNode {
    //NodeTypes关联类型，定义链是什么
    type Primitives = IndexedPrimitives; //区块/区块体类型，将Block和BlockBody换成index字段的扩展版本
    type ChainSpec = IndexedChainSpec; // 执行规则复用 Ethereum，创世头为自定义类型。
    type Storage = IndexedStorage; //自定义存储，需要保存index
    type Payload = IndexedEngineTypes; //共识引擎指的不是以太坊共识规则Pow等，而是把区块出块节点搬到接收节点的通道，即Engine API通道，靠payload传递。
}

pub type IndexedAddOns<N> = RpcAddOns<
    N,
    EthereumEthApiBuilder<IndexedRpcTypes>,
    IndexedValidatorBuilder,
    IndexedEngineApiBuilder,
>;

impl<N> Node<N> for IndexedNode
//Node trait：组件装配
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<IndexedPayloadBuilderBuilder>, //出块时产出带索引的IndexedBuiltPayload
        EthereumNetworkBuilder,
        IndexedExecutorBuilder,  //执行状态转换，把标准区块升级成带索引区块
        IndexedConsensusBuilder, //再所有接收/导入路径上先做索引承诺校验
    >;
    type AddOns = IndexedAddOns<NodeAdapter<N>>;
    /// 复用标准交易池与网络，替换与完整区块体相关的组件。
    fn components_builder(&self) -> Self::ComponentsBuilder {
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(EthereumPoolBuilder::default())
            .executor(IndexedExecutorBuilder)
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .consensus(IndexedConsensusBuilder)
    }
    /// RPC 和共识引擎共同使用同一接收校验器。
    fn add_ons(&self) -> Self::AddOns {
        RpcAddOns::new(
            EthereumEthApiBuilder::<IndexedRpcTypes>::default(),
            IndexedValidatorBuilder,
            IndexedEngineApiBuilder, //自定义EngineAPI,标准newPayload不认识index字段
            BasicEngineValidatorBuilder::new(IndexedValidatorBuilder), //RPC入口做格式和索引承诺校验，共识引擎内部也校验索引承诺
            Default::default(),
            Identity::new(),
        )
    }
}

impl<N: FullNodeComponents<Types = Self>> DebugNode<N> for IndexedNode {
    //调试路径也不破坏索引语义。
    type RpcBlock = IndexedBlock;
    /// 调试导入也必须提供完整扩展区块，不从标准 RPC 猜测索引。
    fn rpc_to_primitive_block(block: IndexedBlock) -> IndexedBlock {
        block
    }
    /// 使用 chain_spec 生成本地开发区块属性。
    fn local_payload_attributes_builder(
        chain_spec: &IndexedChainSpec,
    ) -> impl PayloadAttributesBuilder<PayloadAttributes, IndexedHeader> {
        LocalPayloadAttributesBuilder::new(Arc::new(chain_spec.clone()))
    }
}
