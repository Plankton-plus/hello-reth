//! BLAS 原始日志条目；序号为文件内位置，时间使用 u64。
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub sn: u32, //文件内序号
    pub timestamp: u64,//时间戳
    pub keywords: Vec<String>, //条目关键词
    pub log_value: String, //日志内容
}
