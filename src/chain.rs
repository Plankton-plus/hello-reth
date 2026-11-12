//! 方案可见的只读链视图；通过明确哈希读取，不提供 latest。

use alloy_primitives::B256;
use reth_ethereum::TransactionSigned;
use std::sync::Arc;

#[derive(Clone)]
pub struct HeaderView<H> {
    pub hash: B256,
    pub parent_hash: B256,
    pub number: u64,
    pub fields: H,
}

pub struct BlockView<H, B> {
    pub header: HeaderView<H>,
    pub fields: B,
    pub transactions: Vec<TransactionSigned>,
}

type HeaderRead<H> = Arc<dyn Fn(B256) -> Result<Option<HeaderView<H>>, String> + Send + Sync>;
type BlockRead<H, B> = Arc<dyn Fn(B256) -> Result<Option<BlockView<H, B>>, String> + Send + Sync>;

#[derive(Clone)]
pub struct ChainReader<H, B> {
    headers: HeaderRead<H>,
    blocks: BlockRead<H, B>,
}

impl<H, B> std::fmt::Debug for ChainReader<H, B> {
    /// 只显示类型，不输出 provider 内部状态。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChainReader")
    }
}

impl<H, B> ChainReader<H, B> {
    /// 使用按哈希读取的 headers、blocks 创建读取器；适配层负责认证内容哈希。
    pub fn new(headers: HeaderRead<H>, blocks: BlockRead<H, B>) -> Self {
        Self { headers, blocks }
    }

    /// 读取 hash 对应头部，缺失与损坏均明确返回错误。
    pub fn header(&self, hash: B256) -> Result<HeaderView<H>, String> {
        (self.headers)(hash)?.ok_or_else(|| "已认证历史区块头缺失".into())
    }

    /// 读取 hash 对应的完整区块，不重建索引。
    pub fn block(&self, hash: B256) -> Result<BlockView<H, B>, String> {
        (self.blocks)(hash)?.ok_or_else(|| "已认证历史区块体缺失".into())
    }
}

pub struct BuildContext<'a, H, B> {
    pub parent_hash: B256,
    reader: &'a ChainReader<H, B>,
}

impl<'a, H, B> BuildContext<'a, H, B> {
    /// 将 reader 固定在 parent_hash 的历史分支，用于构建和接收校验。
    pub fn new(parent_hash: B256, reader: &'a ChainReader<H, B>) -> Self {
        Self {
            parent_hash,
            reader,
        }
    }

    /// 验证 hash 属于父分支后读取头部，不能访问旁支或未来块。
    pub fn header(&self, hash: B256) -> Result<HeaderView<H>, String> {
        let mut cursor = self.parent_hash;
        while cursor != B256::ZERO {
            let header = self.reader.header(cursor)?;
            if cursor == hash {
                return Ok(header);
            }
            cursor = header.parent_hash;
        }
        Err("构建上下文只能读取父链祖先".into())
    }

    /// 在验证祖先关系后读取 hash 的完整索引和交易。
    pub fn block(&self, hash: B256) -> Result<BlockView<H, B>, String> {
        self.header(hash)?;
        self.reader.block(hash)
    }
}
