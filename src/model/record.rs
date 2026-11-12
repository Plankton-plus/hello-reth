//! 数据交易信封解析层，负责从交易里判断这是不是一条数据交易。

use super::metadata::{IndexedRecord, LogMetadata};
use alloy_consensus::Transaction;
use alloy_primitives::{Address, address};
use reth_ethereum::TransactionSigned;

pub const DATA_ADDRESS: Address = address!("000000000000000000000000000000000000a900");
pub const RECORD_TAG: &[u8] = b"ARGUS2\0";

pub struct RawRecord<'a> {
    pub transaction_index: u32,
    pub line: &'a str,
}

/// 从 input 提取文本；position 是真实块内位置，普通交易返回 None。
pub fn record_from_input(input: &[u8], position: usize) -> Result<Option<RawRecord<'_>>, String> {
    let Some(line) = input.strip_prefix(RECORD_TAG) else {
        return Ok(None);
    };
    if line.is_empty() || line.len() > 16384 {
        return Err("元数据载荷必须为 1～16384 字节".into());
    }
    Ok(Some(RawRecord {
        transaction_index: position.try_into().map_err(|_| "交易位置超过 u32")?,
        line: std::str::from_utf8(line).map_err(|_| "元数据载荷不是 UTF-8")?,
    }))
}

/// 从 transactions 按实际顺序提取统一元数据，各方案接收相同的解析结果。 区块级交易入口，对区块里的交易列表，逐个判断提取得到Vec<IndexedRecord>
pub fn records_from_transactions(
    transactions: &[TransactionSigned],
) -> Result<Vec<IndexedRecord>, String> {
    let mut records = Vec::new();
    for (position, transaction) in transactions.iter().enumerate() {
        if transaction.to() == Some(DATA_ADDRESS) {
            if let Some(raw) = record_from_input(transaction.input(), position)? {
                records.push(IndexedRecord {
                    transaction_index: raw.transaction_index,
                    metadata: LogMetadata::parse(raw.line)?,
                });
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 信封只提取原文及真实位置，不再消耗八字节时间前缀。
    #[test]
    fn extracts_raw_line_only() {
        let mut input = RECORD_TAG.to_vec();
        input.extend(crate::test_support::line().as_bytes());
        let raw = record_from_input(&input, 2).unwrap().unwrap();
        assert_eq!(raw.line, crate::test_support::line());
        assert_eq!(raw.transaction_index, 2);
        assert!(record_from_input(b"ordinary", 0).unwrap().is_none());
        assert!(record_from_input(RECORD_TAG, 0).is_err());
    }
}
