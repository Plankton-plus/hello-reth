//! 原生自定义区块：公共执行字段与方案定义的头体字段共同编码。

use crate::{
    chain::BuildContext,
    model::record::records_from_transactions,
    node::header::IndexedHeader,
    scheme::{ActiveScheme, BodyFields, Scheme, SchemeReader},
};
use alloy_consensus::BlockBody as EthBody;
use alloy_eips::eip4895::Withdrawals;
use alloy_rlp::{Encodable, RlpDecodable, RlpEncodable};
use reth_ethereum::{
    Receipt, TransactionSigned,
    primitives::{Block, BlockBody, InMemorySize, NodePrimitives},
};
use serde::{Deserialize, Serialize};

/// 完整原生区块体；fields 的复合结构由当前方案决定。
#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct IndexedBody {
    pub ethereum: EthBody<TransactionSigned, IndexedHeader>,
    pub fields: BodyFields,
}

/// 实验链原生区块，网络编码、payload 和存储均使用本类型。
#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct IndexedBlock {
    pub header: IndexedHeader,
    pub body: IndexedBody,
}

/// 沿用 Ethereum 交易和回执，自定义区块头与区块体。告诉Reth我们的区块长什么样
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexedPrimitives;

impl NodePrimitives for IndexedPrimitives {
    type Block = IndexedBlock;
    type BlockHeader = IndexedHeader;
    type BlockBody = IndexedBody;
    type SignedTx = TransactionSigned;
    type Receipt = Receipt;
}

impl IndexedBlock {
    /// 从已执行的 ethereum 区块和父链 reader 构建当前方案的原生头体。 构建区块的唯一入口
    pub fn from_ethereum(
        ethereum: reth_ethereum::Block,
        reader: &SchemeReader,
    ) -> Result<Self, String> {
        let records = records_from_transactions(&ethereum.body.transactions)?;
        let context = BuildContext::new(ethereum.header.parent_hash, reader); //显示查询只能是祖先块，不是旁支
        let (fields, body_fields) = ActiveScheme::build(&context, &records)?; //从本块交易抽出的数据记录+父链只读试图算出头字段+体字段
        let mut header = IndexedHeader::from(ethereum.header);
        header.fields = fields; //标准头 -> IndexedHeader，再把头字段填入
        Ok(Self {
            header,
            body: IndexedBody {
                //体部照搬+体字段填进去
                ethereum: EthBody {
                    transactions: ethereum.body.transactions,
                    ommers: ethereum.body.ommers.into_iter().map(Into::into).collect(),
                    withdrawals: ethereum.body.withdrawals,
                },
                fields: body_fields,
            },
        })
    }

    /// 核对完整索引与头部承诺；只有空创世块不携带索引。
    pub fn validate_commitment(&self) -> Result<(), String> {
        self.header.validate_protocol()?;
        if !self.body.ethereum.ommers.is_empty() {
            return Err("本实验链不接受叔块".into());
        }
        if self.header.number == 0
            && self.body.ethereum.transactions.is_empty()
            && self.body.fields == Default::default()
            && self.header.fields == Default::default()
        {
            return Ok(());
        }
        ActiveScheme::verify_commitment(&self.header.fields, &self.body.fields)
    }

    /// 接收时使用 reader 的父链视图调用方案校验，不规定其具体算法。接收路径的完整索引校验，防止伪造承诺但索引内容被篡改
    pub fn validate_index(&self, reader: &SchemeReader) -> Result<(), String> {
        self.validate_commitment()?;
        if self.header.number == 0 {
            return Ok(());
        }
        let records = records_from_transactions(&self.body.ethereum.transactions)?;
        let context = BuildContext::new(self.header.parent_hash, reader);
        ActiveScheme::validate(&context, &records, &self.header.fields, &self.body.fields)
    }

    /// 转换为标准 Ethereum 视图，仅用于复用默认执行和 RPC 字段转换。
    pub fn ethereum(&self) -> reth_ethereum::Block {
        reth_ethereum::Block {
            header: self.header.ethereum.clone(),
            body: EthBody {
                transactions: self.body.ethereum.transactions.clone(),
                ommers: self
                    .body
                    .ethereum
                    .ommers
                    .iter()
                    .map(|h| h.ethereum.clone())
                    .collect(),
                withdrawals: self.body.ethereum.withdrawals.clone(),
            },
        }
    }
}

impl InMemorySize for IndexedBody {
    /// 包含索引实际字节，供缓存统计使用。
    fn size(&self) -> usize {
        self.ethereum.size() + self.fields.length()
    }
}

impl InMemorySize for IndexedBlock {
    /// 返回头部及完整区块体的内存大小。
    fn size(&self) -> usize {
        self.header.size() + self.body.size()
    }
}

impl BlockBody for IndexedBody {
    type Transaction = TransactionSigned;
    type OmmerHeader = IndexedHeader;
    /// 借用原交易列表。
    fn transactions(&self) -> &[TransactionSigned] {
        &self.ethereum.transactions
    }
    /// 输出标准 RPC 视图；原生网络和存储不调用此转换保存数据。
    fn into_ethereum_body(self) -> EthBody<TransactionSigned, IndexedHeader> {
        self.ethereum
    }
    /// 取出所有交易。
    fn into_transactions(self) -> Vec<TransactionSigned> {
        self.ethereum.transactions
    }
    /// 借用标准提款字段。
    fn withdrawals(&self) -> Option<&Withdrawals> {
        self.ethereum.withdrawals.as_ref()
    }
    /// 借用标准叔块字段。
    fn ommers(&self) -> Option<&[IndexedHeader]> {
        Some(&self.ethereum.ommers)
    }
}

impl Block for IndexedBlock {
    type Header = IndexedHeader;
    type Body = IndexedBody;
    /// 从 header 和包含索引的 body 创建区块。
    fn new(header: IndexedHeader, body: IndexedBody) -> Self {
        Self { header, body }
    }
    /// 借用区块头。
    fn header(&self) -> &IndexedHeader {
        &self.header
    }
    /// 借用完整区块体。
    fn body(&self) -> &IndexedBody {
        &self.body
    }
    /// 拆分为头部及完整区块体。
    fn split(self) -> (IndexedHeader, IndexedBody) {
        (self.header, self.body)
    }
    /// 计算原生扩展 RLP 长度，包含全部索引字节。
    fn rlp_length(header: &IndexedHeader, body: &IndexedBody) -> usize {
        Self::new(header.clone(), body.clone()).length()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use alloy_primitives::Sealable;
    use alloy_rlp::Decodable;

    /// 完整 RLP 往返保留索引；即使攻击者同步修改承诺，也不能伪造索引内容。
    #[test]
    fn roundtrip_and_reject_tampered_index() {
        let block = test_support::block();
        block.validate_index(&test_support::reader()).unwrap();
        let encoded = alloy_rlp::encode(&block);
        let mut bytes = encoded.as_slice();
        assert_eq!(IndexedBlock::decode(&mut bytes).unwrap(), block);
        assert!(bytes.is_empty());
        let mut bad = block.clone();
        bad.header.fields = Default::default();
        assert_ne!(bad.header.hash_slow(), block.header.hash_slow());
        assert!(bad.validate_index(&test_support::reader()).is_err());
        let json = serde_json::to_vec(&block).unwrap();
        assert_eq!(
            serde_json::from_slice::<IndexedBlock>(&json).unwrap(),
            block
        );
    }
}
