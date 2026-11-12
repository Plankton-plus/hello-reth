//! 原生实验区块头；方案字段参与 RLP、区块哈希和数据库编码。

use crate::scheme::{ActiveScheme, Scheme};
use alloy_consensus::{BlockHeader as AlloyHeader, Header};
use alloy_primitives::{Address, B64, B256, Bloom, Bytes, Sealable, U256, keccak256};
use alloy_rlp::{BufMut, Decodable, Encodable, RlpDecodable, RlpEncodable};
use reth_ethereum::{
    codec::Compact,
    primitives::{BlockHeader, InMemorySize, header::HeaderMut},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct IndexedHeader {
    #[serde(flatten)]
    pub ethereum: Header,
    pub scheme: u16,
    pub version: u16,
    pub fields: <ActiveScheme as Scheme>::HeaderFields,
}

impl From<Header> for IndexedHeader {
    /// 用 ethereum 执行字段创建当前方案的头；创世扩展字段为空。
    fn from(ethereum: Header) -> Self {
        Self {
            ethereum,
            scheme: ActiveScheme::ID,
            version: ActiveScheme::VERSION,
            fields: Default::default(),
        }
    }
}

impl Default for IndexedHeader {
    /// 创建与当前实验协议一致的默认头。
    fn default() -> Self {
        Header::default().into()
    }
}

impl IndexedHeader {
    /// 拒绝不属于当前方案及版本的头，避免混用不同原生编码。
    pub fn validate_protocol(&self) -> Result<(), String> {
        if self.scheme != ActiveScheme::ID || self.version != ActiveScheme::VERSION {
            return Err("区块方案或版本不匹配".into());
        }
        Ok(())
    }
}

impl AsRef<Self> for IndexedHeader {
    /// 返回完整自定义头，不丢弃方案字段。
    fn as_ref(&self) -> &Self {
        self
    }
}

impl std::ops::Deref for IndexedHeader {
    type Target = Header;
    /// 借用 Ethereum 执行字段；哈希始终由完整 IndexedHeader 计算。
    fn deref(&self) -> &Header {
        &self.ethereum
    }
}

impl Sealable for IndexedHeader {
    /// 将全部原生头字段的规范 RLP 编码纳入哈希。
    fn hash_slow(&self) -> B256 {
        keccak256(alloy_rlp::encode(self))
    }
}

impl InMemorySize for IndexedHeader {
    /// 统计公共头和方案字段的编码大小。
    fn size(&self) -> usize {
        self.ethereum.size() + self.fields.length() + 4
    }
}

impl Compact for IndexedHeader {
    /// 将完整头写入 buf，返回写入字节数。
    fn to_compact<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) -> usize {
        self.encode(buf);
        self.length()
    }
    /// 从 buf 的前 len 字节读取完整头；数据库损坏必须明确失败。
    fn from_compact(buf: &[u8], len: usize) -> (Self, &[u8]) {
        let mut encoded = &buf[..len];
        let header = Self::decode(&mut encoded).expect("自定义头数据库编码损坏");
        assert!(encoded.is_empty(), "自定义头数据库编码含多余字节");
        (header, &buf[len..])
    }
}

impl BlockHeader for IndexedHeader {}

impl reth_ethereum::codec::Compress for IndexedHeader {
    type Compressed = Vec<u8>;
    /// 将完整自定义头写入 buf，持久化格式与规范 RLP 一致。
    fn compress_to_buf<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        self.encode(buf);
    }
}

impl reth_ethereum::codec::Decompress for IndexedHeader {
    /// 从数据库 value 解码完整头，拒绝损坏或多余字节。
    fn decompress(mut value: &[u8]) -> Result<Self, reth_ethereum::codec::DecompressError> {
        let header =
            Self::decode(&mut value).map_err(reth_ethereum::codec::DecompressError::new)?;
        if !value.is_empty() {
            return Err(reth_ethereum::codec::DecompressError::new(
                std::io::Error::other("自定义头有多余字节"),
            ));
        }
        Ok(header)
    }
}

/// 为标准执行器投影 parent 的公共字段，同时保留完整自定义父块哈希。
pub fn execution_parent(
    parent: &reth_ethereum::primitives::SealedHeader<IndexedHeader>,
) -> reth_ethereum::primitives::SealedHeader {
    reth_ethereum::primitives::SealedHeader::new(parent.ethereum.clone(), parent.hash())
}

impl HeaderMut for IndexedHeader {
    /// 将公共执行字段 parent_hash 更新为 hash。
    fn set_parent_hash(&mut self, hash: B256) {
        self.ethereum.parent_hash = hash;
    }

    /// 将公共执行字段 number 更新为 number。
    fn set_block_number(&mut self, number: u64) {
        self.ethereum.number = number;
    }

    /// 将公共执行字段 timestamp 更新为 timestamp。
    fn set_timestamp(&mut self, timestamp: u64) {
        self.ethereum.timestamp = timestamp;
    }

    /// 将公共执行字段 state_root 更新为 root。
    fn set_state_root(&mut self, root: B256) {
        self.ethereum.state_root = root;
    }

    /// 将公共执行字段 difficulty 更新为 difficulty。
    fn set_difficulty(&mut self, difficulty: U256) {
        self.ethereum.difficulty = difficulty;
    }

    /// 将公共执行字段 mix_hash 更新为 hash。
    fn set_mix_hash(&mut self, hash: B256) {
        self.ethereum.mix_hash = hash;
    }

    /// 将公共执行字段 extra_data 更新为 data。
    fn set_extra_data(&mut self, data: Bytes) {
        self.ethereum.extra_data = data;
    }

    /// 将公共执行字段 parent_beacon_block_root 更新为 root。
    fn set_parent_beacon_block_root(&mut self, root: Option<B256>) {
        self.ethereum.parent_beacon_block_root = root;
    }
}

impl AlloyHeader for IndexedHeader {
    /// 读取公共执行字段 parent_hash。
    fn parent_hash(&self) -> B256 {
        self.ethereum.parent_hash()
    }

    /// 读取公共执行字段 ommers_hash。
    fn ommers_hash(&self) -> B256 {
        self.ethereum.ommers_hash()
    }

    /// 读取公共执行字段 beneficiary。
    fn beneficiary(&self) -> Address {
        self.ethereum.beneficiary()
    }

    /// 读取公共执行字段 state_root。
    fn state_root(&self) -> B256 {
        self.ethereum.state_root()
    }

    /// 读取公共执行字段 transactions_root。
    fn transactions_root(&self) -> B256 {
        self.ethereum.transactions_root()
    }

    /// 读取公共执行字段 receipts_root。
    fn receipts_root(&self) -> B256 {
        self.ethereum.receipts_root()
    }

    /// 读取公共执行字段 withdrawals_root。
    fn withdrawals_root(&self) -> Option<B256> {
        self.ethereum.withdrawals_root()
    }

    /// 读取公共执行字段 logs_bloom。
    fn logs_bloom(&self) -> Bloom {
        self.ethereum.logs_bloom()
    }
    /// 读取公共执行字段 difficulty。
    fn difficulty(&self) -> U256 {
        self.ethereum.difficulty()
    }

    /// 读取公共执行字段 number。
    fn number(&self) -> u64 {
        self.ethereum.number()
    }

    /// 读取公共执行字段 gas_limit。
    fn gas_limit(&self) -> u64 {
        self.ethereum.gas_limit()
    }

    /// 读取公共执行字段 gas_used。
    fn gas_used(&self) -> u64 {
        self.ethereum.gas_used()
    }

    /// 读取公共执行字段 timestamp。
    fn timestamp(&self) -> u64 {
        self.ethereum.timestamp()
    }

    /// 读取公共执行字段 mix_hash。
    fn mix_hash(&self) -> Option<B256> {
        self.ethereum.mix_hash()
    }

    /// 读取公共执行字段 nonce。
    fn nonce(&self) -> Option<B64> {
        self.ethereum.nonce()
    }

    /// 读取公共执行字段 base_fee_per_gas。
    fn base_fee_per_gas(&self) -> Option<u64> {
        self.ethereum.base_fee_per_gas()
    }
    /// 读取公共执行字段 blob_gas_used。
    fn blob_gas_used(&self) -> Option<u64> {
        self.ethereum.blob_gas_used()
    }

    /// 读取公共执行字段 excess_blob_gas。
    fn excess_blob_gas(&self) -> Option<u64> {
        self.ethereum.excess_blob_gas()
    }

    /// 读取公共执行字段 parent_beacon_block_root。
    fn parent_beacon_block_root(&self) -> Option<B256> {
        self.ethereum.parent_beacon_block_root()
    }

    /// 读取公共执行字段 requests_hash。
    fn requests_hash(&self) -> Option<B256> {
        self.ethereum.requests_hash()
    }

    /// 读取公共执行字段 block_access_list_hash。
    fn block_access_list_hash(&self) -> Option<B256> {
        self.ethereum.block_access_list_hash()
    }

    /// 读取公共执行字段 slot_number。
    fn slot_number(&self) -> Option<u64> {
        self.ethereum.slot_number()
    }

    /// 读取公共执行字段 extra_data。
    fn extra_data(&self) -> &Bytes {
        self.ethereum.extra_data()
    }
}
