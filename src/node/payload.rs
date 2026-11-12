//! 让完整索引作为payload的独立字段随着区块一起传播，而不是硬塞进或其它标准字段。

use crate::node::block::{IndexedBlock, IndexedPrimitives};
use crate::scheme::{ActiveScheme, BodyFields, HeaderFields, Scheme, SchemeReader};
use alloy_eips::{eip4895::Withdrawal, eip7685::Requests};
use alloy_primitives::{B256, Bytes, U256};
use reth_ethereum::{
    node::{
        EthEngineTypes,
        api::{BuiltPayload, EngineTypes, PayloadTypes, payload::ExecutionPayload},
    },
    primitives::{Block, SealedBlock},
    rpc::types::engine::{ExecutionData, PayloadAttributes},
};
use reth_payload_builder::EthBuiltPayload;
use serde::{Deserialize, Serialize};

/// 标准执行字段与独立索引字段共同构成实验链的执行 payload。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexedExecutionData {
    pub ethereum: ExecutionData,
    pub scheme: u16,
    pub version: u16,
    pub block_hash: B256,
    pub header_fields: HeaderFields,
    pub body_fields: BodyFields,
}

/// 保留默认构建器的费用、请求和 sidecar，使用扩展区块作为最终区块。
#[derive(Clone, Debug)]
pub struct IndexedBuiltPayload {
    pub inner: EthBuiltPayload,
    pub block: SealedBlock<IndexedBlock>,
}

impl IndexedBuiltPayload {
    /// 将默认构建器的 payload 升级为索引随块携带的 payload。
    pub fn from_ethereum(payload: EthBuiltPayload, reader: &SchemeReader) -> Result<Self, String> {
        let block = IndexedBlock::from_ethereum(payload.block().clone().into_block(), reader)?;
        Ok(Self {
            inner: payload,
            block: block.seal_slow(),
        })
    }
}

impl BuiltPayload for IndexedBuiltPayload {
    type Primitives = IndexedPrimitives;
    /// 返回认证过完整索引的区块。
    fn block(&self) -> &SealedBlock<IndexedBlock> {
        &self.block
    }
    /// 返回默认执行计算出的交易费用。
    fn fees(&self) -> U256 {
        self.inner.fees()
    }
    /// 返回原执行请求。
    fn requests(&self) -> Option<Requests> {
        self.inner.requests()
    }
    /// 保留原区块访问列表。
    fn block_access_list(&self) -> Option<&Bytes> {
        self.inner.block_access_list()
    }
}

impl From<IndexedBuiltPayload> for IndexedExecutionData {
    /// 消耗 value，完整索引作为独立字段随执行数据一起传递。
    fn from(value: IndexedBuiltPayload) -> Self {
        Self {
            scheme: ActiveScheme::ID,
            version: ActiveScheme::VERSION,
            block_hash: value.block.hash(),
            header_fields: value.block.header().fields.clone(),
            body_fields: value.block.body().fields.clone(),
            ethereum: value.inner.into_execution_data(),
        }
    }
}

impl ExecutionPayload for IndexedExecutionData {
    /// 返回父块哈希。
    fn parent_hash(&self) -> B256 {
        self.ethereum.parent_hash()
    }
    /// 返回认证索引后的块哈希。
    fn block_hash(&self) -> B256 {
        self.block_hash
    }
    /// 返回区块高度。
    fn block_number(&self) -> u64 {
        self.ethereum.block_number()
    }
    /// 返回提款列表。
    fn withdrawals(&self) -> Option<&Vec<Withdrawal>> {
        self.ethereum.withdrawals()
    }
    /// 返回区块访问列表。
    fn block_access_list(&self) -> Option<&Bytes> {
        self.ethereum.block_access_list()
    }
    /// 返回父 beacon 根。
    fn parent_beacon_block_root(&self) -> Option<B256> {
        self.ethereum.parent_beacon_block_root()
    }
    /// 返回区块时间戳。
    fn timestamp(&self) -> u64 {
        self.ethereum.timestamp()
    }
    /// 返回实际耗气量。
    fn gas_used(&self) -> u64 {
        self.ethereum.gas_used()
    }
    /// 返回耗气量上限。
    fn gas_limit(&self) -> u64 {
        self.ethereum.gas_limit()
    }
    /// 返回交易数量。
    fn transaction_count(&self) -> usize {
        self.ethereum.transaction_count()
    }
    /// 返回协议 slot 编号。
    fn slot_number(&self) -> Option<u64> {
        self.ethereum.slot_number()
    }
}

/// 实验链使用明确的自定义 Engine 格式，认证 RPC 不冒充标准 newPayload 接口。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexedEngineTypes;

impl PayloadTypes for IndexedEngineTypes {
    type ExecutionData = IndexedExecutionData;
    type BuiltPayload = IndexedBuiltPayload;
    type PayloadAttributes = PayloadAttributes;
    /// 将完整 block 转为实验 payload，保留 index 和原标准字段。
    fn block_to_payload(
        block: SealedBlock<IndexedBlock>,
        bal: Option<Bytes>,
    ) -> IndexedExecutionData {
        let ethereum = <EthEngineTypes as PayloadTypes>::block_to_payload(
            block.clone().into_block().ethereum().seal_slow(),
            bal,
        );
        IndexedExecutionData {
            ethereum,
            scheme: block.header().scheme,
            version: block.header().version,
            block_hash: block.hash(),
            header_fields: block.header().fields.clone(),
            body_fields: block.body().fields.clone(),
        }
    }
}

impl EngineTypes for IndexedEngineTypes {
    type ExecutionPayloadEnvelopeV1 = IndexedExecutionData;
    type ExecutionPayloadEnvelopeV2 = IndexedExecutionData;
    type ExecutionPayloadEnvelopeV3 = IndexedExecutionData;
    type ExecutionPayloadEnvelopeV4 = IndexedExecutionData;
    type ExecutionPayloadEnvelopeV5 = IndexedExecutionData;
    type ExecutionPayloadEnvelopeV6 = IndexedExecutionData;
}
