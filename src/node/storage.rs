//! 将完整索引与区块主体写入同一 MDBX 事务；读取时按保存内容组合主体。

use crate::node::block::{IndexedBlock, IndexedBody, IndexedPrimitives};
use crate::{node::header::IndexedHeader, scheme::BodyFields};
use alloy_primitives::Bytes;
use alloy_rlp::Decodable;
use reth_db_api::{
    DbTxUnwindExt,
    table::{Table, TableInfo},
    tables::TableSet,
    transaction::{DbTx, DbTxMut},
};
use reth_ethereum::TransactionSigned;
use reth_ethereum::chainspec::{ChainSpecProvider, EthereumHardforks};
use reth_ethereum::provider::{
    BlockBodyReader, BlockBodyWriter, ChainStorageReader, ChainStorageWriter, DBProvider,
    DatabaseProvider, EthStorage, ProviderError, ProviderResult, ReadBodyInput,
    providers::{ChainStorage, NodeTypesForProvider},
};

/// 一张表按区块高度保存所有方案体字段，每个值可以包含多个索引。
#[derive(Debug)]
pub struct BlockIndexes; //定义表结构
impl Table for BlockIndexes {
    const NAME: &'static str = "ArgusBlockIndexes"; //表名
    const DUPSORT: bool = false; // 每个高度一个复合值，不限制值内部的索引数量。
    type Key = u64; //区块高度
    type Value = Bytes; // 每块一条记录，保存全部复合体字段的规范编码。
}
impl TableInfo for BlockIndexes {
    /// 返回表名。
    fn name(&self) -> &'static str {
        Self::NAME
    }
    /// 每块对应一个复合值。
    fn is_dupsort(&self) -> bool {
        false
    }
}
pub struct IndexTables; //是一个表清单，告诉Reth除了标准以太坊表之外，这个项目还需要额外建一张名叫ArgusBlockIndexes的表。
impl TableSet for IndexTables {
    /// 启动时仅注册本项目的附加表。
    fn tables() -> Box<dyn Iterator<Item = Box<dyn TableInfo>>> {
        Box::new(std::iter::once(Box::new(BlockIndexes) as Box<dyn TableInfo>))
    }
}

/// 保留标准主体存储，再追加同一事务内的完整索引存储。
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedStorage; //接管IndexedPrimitives这种链的区块体存储，定义读和写

impl ChainStorage<IndexedPrimitives> for IndexedStorage {
    /// 为 Types 的只读 TX 提供完整区块体读取器。
    fn reader<TX, Types>(
        &self,
    ) -> impl ChainStorageReader<DatabaseProvider<TX, Types>, IndexedPrimitives>
    where
        TX: DbTx + 'static,
        Types: NodeTypesForProvider<Primitives = IndexedPrimitives>,
    {
        self
    }
    /// 为 Types 的可写 TX 提供同事务主体写入器。
    fn writer<TX, Types>(
        &self,
    ) -> impl ChainStorageWriter<DatabaseProvider<TX, Types>, IndexedPrimitives>
    where
        TX: DbTxMut + DbTx + 'static,
        Types: NodeTypesForProvider<Primitives = IndexedPrimitives>,
    {
        self
    }
}

impl<P: DBProvider<Tx: DbTxMut>> BlockBodyWriter<P, IndexedBody> for IndexedStorage {
    /// 将 bodies 的标准字段和索引字段写入 provider 当前事务。
    fn write_block_bodies(
        &self,
        provider: &P,
        bodies: Vec<(u64, Option<&IndexedBody>)>,
    ) -> ProviderResult<()> {
        EthStorage::<TransactionSigned, IndexedHeader>::default().write_block_bodies(
            provider,
            bodies
                .iter()
                .map(|(n, b)| (*n, b.map(|body| &body.ethereum)))
                .collect(),
        )?;
        for (number, body) in bodies {
            if number > 0 {
                //创世区块不写索引
                if let Some(body) = body {
                    provider
                        .tx_ref()
                        .put::<BlockIndexes>(number, alloy_rlp::encode(&body.fields).into())?;
                }
            }
        }
        Ok(())
    }
    /// 回滚至 block 时，同一事务删除后续区块的标准字段和完整索引。
    fn remove_block_bodies_above(&self, provider: &P, block: u64) -> ProviderResult<()> {
        <EthStorage<TransactionSigned, IndexedHeader> as BlockBodyWriter<
            P,
            alloy_consensus::BlockBody<TransactionSigned, IndexedHeader>,
        >>::remove_block_bodies_above(&EthStorage::default(), provider, block)?;
        provider
            .tx_ref()
            .unwind_table_by_num::<BlockIndexes>(block)?;
        Ok(())
    }
}

impl<P> BlockBodyReader<P> for IndexedStorage
where
    P: DBProvider + ChainSpecProvider<ChainSpec: EthereumHardforks>,
{
    type Block = IndexedBlock;
    /// 从 provider 的同一快照读标准主体与完整索引；非创世块缺失索引即报错。
    fn read_block_bodies(
        &self,
        provider: &P,
        inputs: Vec<ReadBodyInput<'_, IndexedBlock>>,
    ) -> ProviderResult<Vec<IndexedBody>> {
        let ethereum = EthStorage::<TransactionSigned, IndexedHeader>::default()
            .read_block_bodies(provider, inputs.clone())?;
        let mut bodies = Vec::new();
        for ((header, _), ethereum) in inputs.into_iter().zip(ethereum) {
            let fields = if header.number == 0 {
                BodyFields::default()
            } else {
                let encoded = provider
                    .tx_ref()
                    .get::<BlockIndexes>(header.number)?
                    .ok_or_else(|| {
                        ProviderError::other(std::io::Error::other("完整区块索引缺失"))
                    })?;
                let mut bytes = encoded.as_ref();
                let fields = BodyFields::decode(&mut bytes)
                    .map_err(|e| ProviderError::other(std::io::Error::other(e)))?;
                if !bytes.is_empty() {
                    return Err(ProviderError::other(std::io::Error::other(
                        "复合索引编码含多余字节",
                    )));
                }
                fields
            };
            let block = IndexedBlock {
                header: header.clone(),
                body: IndexedBody { ethereum, fields },
            };
            bodies.push(block.body);
        }
        Ok(bodies)
    }
}
