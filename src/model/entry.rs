//! BLAS 原始日志条目；序号为文件内位置，时间使用 u64。
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub sn: u32,
    pub timestamp: u64,
    pub keywords: Vec<String>,
    pub log_value: String,
}
