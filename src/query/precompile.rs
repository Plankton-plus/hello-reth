//! 给每个 EVM 实例安装持有节点读取器的查询预编译，不使用全局节点状态。

use crate::{
    query::block::IndexReader,
    query::execute::{execute_query, query_error},
};
use alloy_primitives::{Address, address};
use evm2::{
    BaseEvmTypes, Evm, Precompiles, SpecId,
    evm::precompile::PrecompileProvider,
    interpreter::{GasTracker, Message},
    precompiles::{MovePrecompileError, PrecompileId, PrecompileResult},
};
use reth_ethereum::evm::EvmFactory;

pub const QUERY_ADDRESS: Address = address!("000000000000000000000000000000000000a901");

#[derive(Clone)]
pub struct QueryEvmFactory(pub IndexReader);

impl std::fmt::Debug for QueryEvmFactory {
    /// 工厂仅显示类型名，不输出数据库或读取器内部状态。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueryEvmFactory")
    }
}

impl EvmFactory for QueryEvmFactory {
    type Types = BaseEvmTypes;
    type SpecId = SpecId;
    /// 保留输入 spec 的 Ethereum 分叉规则。
    fn spec_id(&self, spec: SpecId) -> SpecId {
        spec
    }
    /// 保留 spec 的标准交易注册表。
    fn tx_registry(
        &self,
        spec: SpecId,
    ) -> evm2::registry::TxRegistry<BaseEvmTypes, evm2::TxResult<BaseEvmTypes>> {
        evm2::ethereum::ethereum_tx_registry(spec)
    }
    /// 为 evm 安装实例级读取器；默认禁用共享预编译缓存。
    fn configure_evm(&self, evm: &mut Evm<'_, BaseEvmTypes>) {
        evm.set_precompiles(QueryPrecompiles {
            base: Precompiles::base(evm.spec_id()),
            reader: self.0.clone(),
        });
    }
}

struct QueryPrecompiles {
    base: Precompiles<BaseEvmTypes>,
    reader: IndexReader,
}

impl PrecompileProvider<BaseEvmTypes> for QueryPrecompiles {
    /// 返回标准地址及查询地址，供 EVM 预热。
    fn addresses(&self) -> Vec<Address> {
        let mut addresses = self.base.addresses();
        addresses.push(QUERY_ADDRESS);
        addresses
    }
    /// 查询处理器使用独立 ID，与标准预编译区分。
    fn precompile_ids(&self) -> Vec<(Address, PrecompileId)> {
        let mut ids = self.base.precompile_ids();
        ids.push((QUERY_ADDRESS, PrecompileId::custom("argus-query-v1")));
        ids
    }
    /// 判断 address 是否由本实例处理。
    fn contains(&self, address: &Address) -> bool {
        *address == QUERY_ADDRESS || self.base.contains(address)
    }
    /// 沿用标准 moves；查询地址固定，禁止迁移或被覆盖。
    fn move_precompiles(
        &mut self,
        moves: &[(Address, Address)],
    ) -> Result<(), MovePrecompileError> {
        if moves
            .iter()
            .any(|(a, b)| a != b && (*a == QUERY_ADDRESS || *b == QUERY_ADDRESS))
        {
            return Err(MovePrecompileError::NotAPrecompile(QUERY_ADDRESS));
        }
        self.base.move_precompiles(moves)
    }
    /// message 命中查询地址时读取索引，其余调用交给标准预编译。
    fn execute(
        &mut self,
        evm: &mut Evm<'_, BaseEvmTypes>,
        message: &Message<BaseEvmTypes>,
        gas: &mut GasTracker,
    ) -> Option<PrecompileResult> {
        if message.call_target == QUERY_ADDRESS {
            return Some(if message.is_static() {
                execute_query(&self.reader, evm, &message.input, gas)
            } else {
                Err(query_error("查询预编译只接受 STATICCALL"))
            });
        }
        self.base.execute(evm, message, gas)
    }
}
